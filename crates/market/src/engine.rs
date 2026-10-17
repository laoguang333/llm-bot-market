//! The run loop: personal-tick scheduler, per-account settlement, order
//! handling with per-stock locks and mailbox delivery to makers.

use crate::account::{Account, Mail, OwnOrder};
use crate::book::{Resting, StockMarket};
use crate::config::Config;
use crate::world::World;
use anyhow::{bail, Result};
use bot_api::*;
use parking_lot::{Condvar, Mutex};
use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::time::Instant;

pub type BotFactory<'f> = dyn Fn(&str, &Start) -> Option<Box<dyn Bot>> + Sync + 'f;

const MAX_CONSECUTIVE_BOT_ERRORS: u32 = 20;

/// Fenwick tree over completed-tick values, for the lower median.
struct Fenwick {
    tree: Vec<i64>,
}

impl Fenwick {
    fn new(n: usize) -> Self {
        Self {
            tree: vec![0; n + 1],
        }
    }
    fn add(&mut self, i: usize, v: i64) {
        let mut i = i + 1;
        while i < self.tree.len() {
            self.tree[i] += v;
            i += i & i.wrapping_neg();
        }
    }
    /// Smallest value with cumulative count >= k (k >= 1).
    fn kth(&self, mut k: i64) -> usize {
        let mut pos = 0;
        let mut step = self.tree.len().next_power_of_two();
        while step > 0 {
            let next = pos + step;
            if next < self.tree.len() && self.tree[next] < k {
                pos = next;
                k -= self.tree[next];
            }
            step >>= 1;
        }
        pos
    }
}

struct Sched {
    idle: BinaryHeap<Reverse<(Tick, u32)>>,
    running: Vec<(u32, Tick)>,
    fen: Fenwick,
    finished: usize,
    next_check: Tick,
    pausing: bool,
    error: Option<String>,
}

impl Sched {
    fn min_completed(&self, ticks: Tick) -> Tick {
        let a = self.idle.peek().map(|r| r.0 .0).unwrap_or(ticks);
        let b = self.running.iter().map(|r| r.1).min().unwrap_or(ticks);
        a.min(b)
    }
}

pub struct Shared<'a> {
    pub cfg: &'a Config,
    pub rules: Rules,
    pub world: World,
    pub markets: Vec<Mutex<StockMarket>>,
    marks: Vec<AtomicI64>,
    pub accounts: Vec<Mutex<Account>>,
    pub mailboxes: Vec<Mutex<Vec<Mail>>>,
    pub treasury: AtomicI64,
    pub burned: Vec<AtomicI64>,
    next_order_id: AtomicU64,
    sched: Mutex<Sched>,
    cv: Condvar,
    pub initial_cash: Money,
    pub initial_shares: Vec<Qty>,
    pub audits: AtomicU64,
}

pub struct RunOutput {
    pub wall_seconds: f64,
    pub threads: usize,
}

impl<'a> Shared<'a> {
    pub fn new(cfg: &'a Config, seed: u64, factory: &BotFactory<'_>) -> Result<Shared<'a>> {
        let n_stocks = cfg.stocks.len();
        let mut accounts = Vec::new();
        for g in &cfg.bots {
            let owner = g.owner.clone().unwrap_or_else(|| g.bot.clone());
            for i in 0..g.count {
                let mut shares = vec![g.shares_each; n_stocks];
                for (name, q) in &g.shares {
                    shares[cfg.stock_index(name)?] = *q;
                }
                let idx = accounts.len();
                accounts.push(Account::new(
                    idx,
                    format!("{owner}/{}#{i}", g.bot),
                    owner.clone(),
                    g.bot.clone(),
                    g.cash,
                    shares,
                ));
            }
        }
        let n = accounts.len();
        let initial_shares: Vec<Qty> = (0..n_stocks)
            .map(|s| accounts.iter().map(|a| a.shares[s]).sum())
            .collect();
        let initial_cash = accounts.iter().map(|a| a.cash).sum();
        let rules = cfg.rules(&initial_shares);
        let world = World::generate(cfg, seed, n);

        // create bots
        let mut idx = 0;
        for g in &cfg.bots {
            for _ in 0..g.count {
                let a = &mut accounts[idx];
                let start = Start {
                    account: a.name.clone(),
                    owner: a.owner.clone(),
                    account_index: idx,
                    n_accounts: n,
                    seed: seed.wrapping_mul(0x9e3779b97f4a7c15)
                        ^ (idx as u64).wrapping_mul(0xd1b54a32d192ed03),
                    params: g.params.clone(),
                    cash: a.cash,
                    shares: a.shares.clone(),
                    rules: rules.clone(),
                };
                match catch_unwind(AssertUnwindSafe(|| factory(&g.bot, &start))) {
                    Ok(Some(b)) => a.bot = Some(b),
                    Ok(None) => bail!("unknown bot {}", g.bot),
                    Err(_) => a.disabled = Some("panicked during start".into()),
                }
                idx += 1;
            }
        }

        let mut fen = Fenwick::new(cfg.ticks as usize + 1);
        fen.add(0, n as i64);
        let idle = (0..n as u32).map(|i| Reverse((0, i))).collect();
        let check = cfg.schedule.check_interval;
        Ok(Shared {
            markets: cfg
                .stocks
                .iter()
                .map(|s| Mutex::new(StockMarket::new(s.initial_price, cfg.margin.mark_window)))
                .collect(),
            marks: cfg
                .stocks
                .iter()
                .map(|s| AtomicI64::new(s.initial_price))
                .collect(),
            accounts: accounts.into_iter().map(Mutex::new).collect(),
            mailboxes: (0..n).map(|_| Mutex::new(Vec::new())).collect(),
            treasury: AtomicI64::new(0),
            burned: (0..n_stocks).map(|_| AtomicI64::new(0)).collect(),
            next_order_id: AtomicU64::new(1),
            sched: Mutex::new(Sched {
                idle,
                running: Vec::new(),
                fen,
                finished: 0,
                next_check: if check == 0 { Tick::MAX } else { check },
                pausing: false,
                error: None,
            }),
            cv: Condvar::new(),
            cfg,
            rules,
            world,
            initial_cash,
            initial_shares,
            audits: AtomicU64::new(0),
        })
    }

    pub fn n_accounts(&self) -> usize {
        self.accounts.len()
    }

    pub fn marks(&self) -> Vec<Price> {
        self.marks
            .iter()
            .map(|m| m.load(Ordering::Relaxed))
            .collect()
    }

    // ---- scheduler -----------------------------------------------------

    pub fn run(&self) -> Result<RunOutput> {
        let threads = match self.cfg.schedule.threads {
            0 => std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(4),
            t => t,
        };
        let t0 = Instant::now();
        std::thread::scope(|s| {
            for _ in 0..threads {
                s.spawn(|| self.worker());
            }
        });
        if let Some(e) = self.sched.lock().error.take() {
            bail!(e);
        }
        // deliver every remaining fill before scoring
        for i in 0..self.n_accounts() {
            let mut a = self.accounts[i].lock();
            self.drain_mailbox(&mut a);
        }
        self.audit()?;
        Ok(RunOutput {
            wall_seconds: t0.elapsed().as_secs_f64(),
            threads,
        })
    }

    fn worker(&self) {
        let ticks = self.cfg.ticks;
        let n = self.n_accounts();
        let skew = self.cfg.schedule.max_tick_skew.max(1);
        loop {
            let (idx, tick, reference) = {
                let mut s = self.sched.lock();
                loop {
                    if s.finished == n || s.error.is_some() {
                        self.cv.notify_all();
                        return;
                    }
                    let min = s.min_completed(ticks);
                    if !s.pausing && min >= s.next_check {
                        s.pausing = true;
                    }
                    if s.pausing {
                        if s.running.is_empty() {
                            if let Err(e) = self.audit() {
                                s.error = Some(e.to_string());
                                continue;
                            }
                            s.next_check = s
                                .next_check
                                .saturating_add(self.cfg.schedule.check_interval.max(1));
                            s.pausing = false;
                            self.cv.notify_all();
                            continue;
                        }
                    } else if let Some(&Reverse((t, i))) = s.idle.peek() {
                        if t - min < skew {
                            s.idle.pop();
                            s.running.push((i, t));
                            let k = (n as i64 - 1) / 2 + 1;
                            let reference = s.fen.kth(k) as Tick;
                            break (i as usize, t, reference);
                        }
                    }
                    self.cv.wait(&mut s);
                }
            };
            let result = catch_unwind(AssertUnwindSafe(|| self.step(idx, tick, reference)));
            let mut s = self.sched.lock();
            if result.is_err() {
                s.error = Some(format!(
                    "engine panic while stepping account {idx} at tick {tick}"
                ));
            }
            s.running.retain(|r| r.0 as usize != idx);
            s.fen.add(tick as usize, -1);
            s.fen.add(tick as usize + 1, 1);
            if tick + 1 < ticks {
                s.idle.push(Reverse((tick + 1, idx as u32)));
            } else {
                s.finished += 1;
            }
            self.cv.notify_all();
        }
    }

    // ---- one personal tick ------------------------------------------------

    fn step(&self, idx: usize, tick: Tick, reference: Tick) {
        let mut guard = self.accounts[idx].lock();
        let a = &mut *guard;
        self.drain_mailbox(a);
        self.settle_world(a, tick);
        let marks = self.marks();
        self.periodic_charges(a, tick, &marks);
        self.check_liquidation(a, tick);

        let lag = reference.saturating_sub(tick);
        let skip =
            self.cfg.schedule.skip_decision_lag > 0 && lag >= self.cfg.schedule.skip_decision_lag;
        if a.disabled.is_some() || a.bot.is_none() {
            // passive holder
        } else if skip {
            a.stats.skipped += 1;
            a.skipped_since += 1;
        } else {
            let obs = self.observation(a, tick, reference);
            let mut bot = a.bot.take().unwrap();
            let t0 = Instant::now();
            let res = catch_unwind(AssertUnwindSafe(|| bot.on_tick(&obs)));
            let ns = t0.elapsed().as_nanos() as u64;
            a.stats.decisions += 1;
            a.stats.decision_nanos_total += ns;
            a.stats.decision_nanos_max = a.stats.decision_nanos_max.max(ns);
            match res {
                Ok(actions) => {
                    a.bot = Some(bot);
                    a.consecutive_errors = 0;
                    self.apply_actions(a, tick, actions);
                }
                Err(_) => {
                    a.stats.bot_errors += 1;
                    a.consecutive_errors += 1;
                    if a.consecutive_errors >= MAX_CONSECUTIVE_BOT_ERRORS {
                        a.disabled =
                            Some(format!("{MAX_CONSECUTIVE_BOT_ERRORS} consecutive panics"));
                        self.cancel_all(a, None, AckStatus::Cancelled);
                    } else {
                        a.bot = Some(bot);
                    }
                }
            }
        }
        // end-of-tick samples for the idle fee and interest
        let marks = self.marks();
        let gross = a.gross(&marks);
        let equity = a.cash + gross;
        a.equity_sum += equity.max(0) as i128;
        a.loan_sum += (-a.cash).max(0) as i128;
        a.samples += 1;
        a.loan_samples += 1;
        if equity > 0 {
            let lev = gross as f64 / equity as f64;
            if lev > a.stats.max_leverage {
                a.stats.max_leverage = lev;
            }
        }
    }

    pub fn drain_mailbox(&self, a: &mut Account) {
        let mail = std::mem::take(&mut *self.mailboxes[a.idx].lock());
        let credit_maker = self.rules.credit_side != "taker";
        for m in mail {
            let notional = m.price * m.qty;
            match m.side {
                Side::Buy => {
                    a.cash -= notional + m.fee;
                    a.shares[m.stock] += m.qty;
                    a.reserved_buy -= m.qty * m.reserve_per_share;
                }
                Side::Sell => {
                    a.cash += notional - m.fee;
                    a.shares[m.stock] -= m.qty;
                    a.reserved_sell[m.stock] -= m.qty;
                }
            }
            if let Some(o) = a.orders.get_mut(&m.order_id) {
                o.remaining -= m.qty;
                if o.remaining <= 0 {
                    a.orders.remove(&m.order_id);
                }
            }
            a.stats.trade_fees += m.fee;
            a.stats.volume_notional += notional;
            a.stats.trades += 1;
            if credit_maker {
                a.credit_notional += notional;
                a.stats.credited_notional += notional;
            }
            a.inbox.fills.push(Fill {
                order_id: m.order_id,
                stock: m.stock,
                side: m.side,
                price: m.price,
                qty: m.qty,
                fee: m.fee,
                maker: true,
                tick: m.tick,
            });
        }
    }

    fn settle_world(&self, a: &mut Account, tick: Tick) {
        let w = &self.world;
        let deliveries = &w.deliveries[a.idx];
        while a.delivery_cursor < deliveries.len() && deliveries[a.delivery_cursor].tick <= tick {
            let d = deliveries[a.delivery_cursor];
            a.inbox.news.push(w.events[d.event as usize].view(d.public));
            a.delivery_cursor += 1;
        }
        while a.season_cursor < w.season_changes.len()
            && w.season_changes[a.season_cursor].0 <= tick
        {
            a.inbox
                .season_changes
                .push(w.season_changes[a.season_cursor]);
            a.season_cursor += 1;
        }
        let period = self.rules.dividend_period;
        if tick > 0 && tick.is_multiple_of(period) {
            let mut total = 0;
            for s in 0..a.shares.len() {
                let amt = a.shares[s] * w.dividend(s, tick);
                if amt != 0 {
                    total += amt;
                    a.stats.dividends += amt;
                    a.inbox.income.push(Income::Dividend {
                        stock: s,
                        amount: amt,
                    });
                }
            }
            for (r, c) in self.rules.combos.iter().enumerate() {
                if c.active(tick) {
                    let amt = c.sets(&a.shares) * c.bonus;
                    if amt != 0 {
                        total += amt;
                        a.stats.combo_dividends += amt;
                        a.inbox.income.push(Income::Combo {
                            rule: r,
                            amount: amt,
                        });
                    }
                }
            }
            a.cash += total;
            self.treasury.fetch_sub(total, Ordering::Relaxed);
        }
    }

    fn periodic_charges(&self, a: &mut Account, tick: Tick, marks: &[Price]) {
        if tick == 0 {
            return;
        }
        if tick.is_multiple_of(self.rules.interest_period) && a.loan_samples > 0 {
            let avg_loan = (a.loan_sum / a.loan_samples as i128) as Money;
            let interest = (avg_loan as f64 * self.rules.loan_rate) as Money;
            if interest > 0 {
                a.cash -= interest;
                a.stats.interest += interest;
                self.treasury.fetch_add(interest, Ordering::Relaxed);
                a.inbox.income.push(Income::Interest { amount: interest });
            }
            a.loan_sum = 0;
            a.loan_samples = 0;
        }
        if tick.is_multiple_of(self.rules.idle_period) && a.samples > 0 {
            let (gross_fee, credit, due) = self.idle_estimate(a);
            if due > 0 {
                self.charge_idle(a, due, marks);
            }
            if gross_fee > 0 {
                a.inbox.income.push(Income::IdleFee {
                    paid: due,
                    gross: gross_fee,
                    credit: credit.min(gross_fee),
                });
            }
            a.equity_sum = 0;
            a.samples = 0;
            a.credit_notional = 0;
        }
    }

    pub fn idle_estimate(&self, a: &Account) -> (Money, Money, Money) {
        let avg = if a.samples > 0 {
            (a.equity_sum / a.samples as i128) as Money
        } else {
            0
        };
        let fee = (avg as f64 * self.rules.idle_rate) as Money;
        let credit = (a.credit_notional as f64 * self.rules.credit_ratio) as Money;
        (fee, credit, (fee - credit).max(0))
    }

    /// Takes `due` from positive cash and every stock in the same proportion. Cash goes to
    /// the treasury, shares are burned.
    fn charge_idle(&self, a: &mut Account, due: Money, marks: &[Price]) {
        let pos_cash = a.cash.max(0);
        let base = pos_cash + a.gross(marks);
        if base <= 0 {
            return;
        }
        let cash_part = ((pos_cash as i128 * due as i128) / base as i128) as Money;
        a.cash -= cash_part;
        self.treasury.fetch_add(cash_part, Ordering::Relaxed);
        let mut paid = cash_part;
        for s in 0..a.shares.len() {
            if a.shares[s] <= 0 {
                continue;
            }
            a.share_frac[s] +=
                ((a.shares[s] as i128 * due as i128 * 1_000_000) / base as i128) as i64;
            let whole = (a.share_frac[s] / 1_000_000).min(a.shares[s]);
            if whole <= 0 {
                continue;
            }
            a.share_frac[s] -= whole * 1_000_000;
            if a.sellable(s) < whole {
                self.free_shares(a, s, whole);
            }
            a.shares[s] -= whole;
            a.stats.burned_shares += whole;
            self.burned[s].fetch_add(whole, Ordering::Relaxed);
            paid += whole * marks[s];
        }
        a.stats.idle_fees += paid;
    }

    /// Cancels this account's sell orders on `stock`, newest first, until `need` shares are free.
    fn free_shares(&self, a: &mut Account, stock: usize, need: Qty) {
        let mut sells: Vec<(OrderId, Tick)> = a
            .orders
            .iter()
            .filter(|(_, o)| o.live && o.stock == stock && o.side == Side::Sell)
            .map(|(id, o)| (*id, o.placed_tick))
            .collect();
        sells.sort_by_key(|(id, t)| Reverse((*t, *id)));
        for (id, _) in sells {
            if a.sellable(stock) >= need {
                break;
            }
            self.cancel_order(a, id, AckStatus::Cancelled);
        }
    }

    fn check_liquidation(&self, a: &mut Account, tick: Tick) {
        let marks = self.marks();
        let gross = a.gross(&marks);
        let equity = a.cash + gross;
        let breach = if equity <= 0 {
            gross > 0 || a.cash < 0
        } else {
            gross as f64 > self.rules.maintenance_leverage * equity as f64
        };
        if !breach {
            return;
        }
        a.stats.liquidations += 1;
        a.inbox.income.push(Income::Liquidation);
        self.cancel_all(a, None, AckStatus::CancelledByLiquidation);
        let frac = if equity <= 0 {
            1.0
        } else {
            let target = self.rules.max_leverage * 0.9 * equity as f64;
            ((gross as f64 - target) / gross as f64).clamp(0.0, 1.0)
        };
        for s in 0..a.shares.len() {
            let q = ((a.shares[s] as f64) * frac).ceil() as Qty;
            let q = q.min(a.sellable(s));
            if q > 0 {
                self.place(
                    a,
                    tick,
                    Action::Market {
                        stock: s,
                        side: Side::Sell,
                        qty: q,
                        cid: 0,
                    },
                    true,
                );
            }
        }
        if a.shares.iter().all(|&s| s == 0) && a.cash < 0 && a.orders.is_empty() {
            let debt = -a.cash;
            a.cash = 0;
            a.stats.bad_debt += debt;
            self.treasury.fetch_sub(debt, Ordering::Relaxed);
            a.inbox
                .income
                .push(Income::BadDebtWrittenOff { amount: debt });
        }
    }

    // ---- observation -------------------------------------------------------

    fn observation(&self, a: &mut Account, tick: Tick, reference: Tick) -> Observation {
        let depth = self.rules.book_depth;
        let limit = self.cfg.market.trade_tape_limit;
        let mut books = Vec::with_capacity(self.markets.len());
        let mut trades = Vec::with_capacity(self.markets.len());
        for (s, m) in self.markets.iter().enumerate() {
            let m = m.lock();
            books.push(BookView {
                bids: m.depth(Side::Buy, depth),
                asks: m.depth(Side::Sell, depth),
                last: m.last,
                mark: m.mark,
                dividend: self.world.dividend(s, tick),
            });
            let start = a.tape_seen[s].max(m.tape.len().saturating_sub(limit));
            trades.push(
                m.tape[start..]
                    .iter()
                    .map(|e| Trade {
                        tick: e.tick,
                        price: e.price,
                        qty: e.qty,
                        aggressor: e.aggressor,
                    })
                    .collect(),
            );
            a.tape_seen[s] = m.tape.len();
        }
        let marks: Vec<Price> = books.iter().map(|b| b.mark).collect();
        let gross = a.gross(&marks);
        let equity = a.cash + gross;
        let (fee, credit, due) = self.idle_estimate(a);
        let period_end = (tick / self.rules.idle_period + 1) * self.rules.idle_period;
        let avg_loan = if a.loan_samples > 0 {
            (a.loan_sum / a.loan_samples as i128) as Money
        } else {
            (-a.cash).max(0)
        };
        let account = AccountView {
            cash: a.cash,
            shares: a.shares.clone(),
            sellable: (0..a.shares.len()).map(|s| a.sellable(s)).collect(),
            equity,
            gross,
            leverage: if equity > 0 {
                gross as f64 / equity as f64
            } else {
                f64::INFINITY
            },
            buying_power: self.buying_power(a, equity, gross),
            reserved_buy: a.reserved_buy,
            idle: IdleEstimate {
                period_end_tick: period_end,
                avg_equity: if a.samples > 0 {
                    (a.equity_sum / a.samples as i128) as Money
                } else {
                    equity
                },
                fee,
                credit,
                due,
            },
            interest_estimate: (avg_loan as f64 * self.rules.loan_rate) as Money,
        };
        let mut open_orders: Vec<OpenOrder> = a
            .orders
            .iter()
            .filter(|(_, o)| o.live)
            .map(|(id, o)| OpenOrder {
                order_id: *id,
                stock: o.stock,
                side: o.side,
                price: o.price,
                remaining: o.remaining,
                placed_tick: o.placed_tick,
            })
            .collect();
        open_orders.sort_by_key(|o| o.order_id);
        let inbox = std::mem::take(&mut a.inbox);
        let skipped = std::mem::take(&mut a.skipped_since);
        Observation {
            tick,
            ticks_total: self.rules.ticks_total,
            reference_tick: reference,
            season: self.world.season(tick),
            account,
            open_orders,
            books,
            trades,
            skipped,
            fills: inbox.fills,
            acks: inbox.acks,
            rejects: inbox.rejects,
            news: inbox.news,
            season_changes: inbox.season_changes,
            income: inbox.income,
        }
    }

    fn buying_power(&self, a: &Account, equity: Money, gross: Money) -> Money {
        ((self.rules.max_leverage * equity as f64) as Money - gross - a.reserved_buy).max(0)
    }

    // ---- actions -----------------------------------------------------------

    fn apply_actions(&self, a: &mut Account, tick: Tick, actions: Vec<Action>) {
        let max = self.rules.max_actions_per_tick;
        for (i, act) in actions.into_iter().enumerate() {
            if i >= max {
                a.stats.rejects += 1;
                a.inbox.rejects.push(Reject {
                    action: act,
                    reason: RejectReason::TooManyActions,
                });
                continue;
            }
            match act {
                Action::Cancel { order_id } => match a.orders.get(&order_id) {
                    Some(o) if o.live => self.cancel_order(a, order_id, AckStatus::Cancelled),
                    _ => {
                        a.stats.rejects += 1;
                        a.inbox.rejects.push(Reject {
                            action: act,
                            reason: RejectReason::UnknownOrder,
                        });
                    }
                },
                Action::CancelAll { stock } => self.cancel_all(a, stock, AckStatus::Cancelled),
                Action::Limit { .. } | Action::Market { .. } => self.place(a, tick, act, false),
            }
        }
    }

    fn cancel_all(&self, a: &mut Account, stock: Option<usize>, status: AckStatus) {
        let ids: Vec<OrderId> = a
            .orders
            .iter()
            .filter(|(_, o)| o.live && stock.is_none_or(|s| s == o.stock))
            .map(|(id, _)| *id)
            .collect();
        for id in ids {
            self.cancel_order(a, id, status);
        }
    }

    fn cancel_order(&self, a: &mut Account, id: OrderId, status: AckStatus) {
        let (stock, side, rps) = {
            let o = &a.orders[&id];
            (o.stock, o.side, o.reserve_per_share)
        };
        let on_book = self.markets[stock].lock().cancel(id);
        match side {
            Side::Buy => a.reserved_buy -= on_book * rps,
            Side::Sell => a.reserved_sell[stock] -= on_book,
        }
        let o = a.orders.get_mut(&id).unwrap();
        o.remaining -= on_book;
        o.live = false;
        let remaining = o.remaining;
        if remaining <= 0 {
            a.orders.remove(&id);
        }
        a.inbox.acks.push(Ack {
            cid: 0,
            order_id: id,
            status,
            remaining: 0,
        });
    }

    fn reject(&self, a: &mut Account, action: Action, reason: RejectReason) {
        a.stats.rejects += 1;
        a.inbox.rejects.push(Reject { action, reason });
    }

    fn place(&self, a: &mut Account, tick: Tick, act: Action, liquidation: bool) {
        let (stock, side, limit, qty, tif, cid) = match act {
            Action::Limit {
                stock,
                side,
                price,
                qty,
                tif,
                cid,
            } => (stock, side, Some(price), qty, tif, cid),
            Action::Market {
                stock,
                side,
                qty,
                cid,
            } => (stock, side, None, qty, Tif::Ioc, cid),
            _ => unreachable!(),
        };
        if stock >= self.markets.len() {
            return self.reject(a, act, RejectReason::UnknownStock);
        }
        if qty <= 0 || qty > crate::config::MAX_SHARES {
            return self.reject(a, act, RejectReason::BadQty);
        }
        if let Some(p) = limit {
            if p <= 0 || p > crate::config::MAX_PRICE || p % self.rules.tick_size != 0 {
                return self.reject(a, act, RejectReason::BadPrice);
            }
        }
        if tif == Tif::Gtc && a.live_orders() >= self.rules.max_open_orders {
            return self.reject(a, act, RejectReason::TooManyOpenOrders);
        }
        // self-cross: never able to reach our own resting orders
        let (own_min_sell, own_max_buy) = a.own_touch(stock);
        let crosses = match (side, limit) {
            (Side::Buy, Some(p)) => own_min_sell.is_some_and(|s| p >= s),
            (Side::Sell, Some(p)) => own_max_buy.is_some_and(|b| p <= b),
            (Side::Buy, None) => own_min_sell.is_some(),
            (Side::Sell, None) => own_max_buy.is_some(),
        };
        if crosses {
            return self.reject(a, act, RejectReason::SelfCross);
        }

        let marks = self.marks();
        let rps = match (side, limit) {
            (Side::Buy, Some(p)) => self.rules.buy_reserve_per_share(p),
            _ => 0,
        };
        let mut power = 0;
        match side {
            Side::Sell => {
                if a.sellable(stock) < qty {
                    return self.reject(a, act, RejectReason::InsufficientShares);
                }
                a.reserved_sell[stock] += qty;
            }
            Side::Buy => {
                let gross = a.gross(&marks);
                power = self.buying_power(a, a.cash + gross, gross);
                if limit.is_some() {
                    let need = qty * rps;
                    if need > power {
                        return self.reject(a, act, RejectReason::InsufficientBuyingPower);
                    }
                    a.reserved_buy += need;
                } else if power <= 0 {
                    return self.reject(a, act, RejectReason::InsufficientBuyingPower);
                }
            }
        }
        a.stats.orders += 1;
        let id = self.next_order_id.fetch_add(1, Ordering::Relaxed);
        let fee_bps = self.rules.trade_fee_bps;
        let market_buy = side == Side::Buy && limit.is_none();
        let mut cap = |price: Price, n: Qty| -> Qty {
            if !market_buy {
                return n;
            }
            let per = price + (price * fee_bps + 9_999) / 10_000;
            let afford = power / per;
            let take = n.min(afford);
            power -= take * per;
            take
        };
        let idx = a.idx as u32;
        let (fills, rested, new_mark) = {
            let mut m = self.markets[stock].lock();
            let fills = m.take(side, limit, qty, idx, tick, &mut cap);
            let filled: Qty = fills.iter().map(|f| f.qty).sum();
            let remaining = qty - filled;
            let rested = remaining > 0 && tif == Tif::Gtc;
            if rested {
                m.add(Resting {
                    id,
                    account: idx,
                    side,
                    price: limit.unwrap(),
                    qty: remaining,
                    reserve_per_share: rps,
                });
            }
            (fills, rested, m.mark)
        };
        self.marks[stock].store(new_mark, Ordering::Relaxed);

        let credit_taker = self.rules.credit_side != "maker";
        let mut filled = 0;
        for f in &fills {
            let notional = f.price * f.qty;
            let fee = self.rules.trade_fee(notional);
            filled += f.qty;
            match side {
                Side::Buy => {
                    a.cash -= notional + fee;
                    a.shares[stock] += f.qty;
                    if rps > 0 {
                        a.reserved_buy -= f.qty * rps;
                    }
                }
                Side::Sell => {
                    a.cash += notional - fee;
                    a.shares[stock] -= f.qty;
                    a.reserved_sell[stock] -= f.qty;
                }
            }
            a.stats.trade_fees += fee;
            a.stats.volume_notional += notional;
            a.stats.trades += 1;
            if credit_taker {
                a.credit_notional += notional;
                a.stats.credited_notional += notional;
            }
            a.inbox.fills.push(Fill {
                order_id: id,
                stock,
                side,
                price: f.price,
                qty: f.qty,
                fee,
                maker: false,
                tick,
            });
            // maker pays its fee now (treasury) and settles the rest through its mailbox
            self.treasury.fetch_add(2 * fee, Ordering::Relaxed);
            self.mailboxes[f.account as usize].lock().push(Mail {
                order_id: f.id,
                stock,
                side: f.side,
                price: f.price,
                qty: f.qty,
                fee,
                tick,
                reserve_per_share: f.reserve_per_share,
            });
        }
        let remaining = qty - filled;
        if rested {
            a.orders.insert(
                id,
                OwnOrder {
                    stock,
                    side,
                    price: limit.unwrap(),
                    remaining,
                    reserve_per_share: rps,
                    placed_tick: tick,
                    live: true,
                },
            );
        } else if remaining > 0 {
            match side {
                Side::Buy => a.reserved_buy -= remaining * rps,
                Side::Sell => a.reserved_sell[stock] -= remaining,
            }
        }
        if !liquidation {
            let status = if rested {
                if filled == 0 {
                    AckStatus::Resting
                } else {
                    AckStatus::PartialResting
                }
            } else if remaining == 0 {
                AckStatus::Filled
            } else if filled > 0 {
                AckStatus::PartialCancelled
            } else {
                AckStatus::NoFill
            };
            a.inbox.acks.push(Ack {
                cid,
                order_id: id,
                status,
                remaining: if rested { remaining } else { 0 },
            });
        }
    }

    // ---- audit -------------------------------------------------------------

    /// Conservation check. Call only when no account is being stepped.
    pub fn audit(&self) -> Result<()> {
        self.audits.fetch_add(1, Ordering::Relaxed);
        let n_stocks = self.markets.len();
        let mut cash: i128 = self.treasury.load(Ordering::Relaxed) as i128;
        let mut shares: Vec<i128> = (0..n_stocks)
            .map(|s| self.burned[s].load(Ordering::Relaxed) as i128)
            .collect();
        let mut book_buy_reserve: Vec<i128> = vec![0; self.n_accounts()];
        let mut book_sell: Vec<Vec<i128>> = vec![vec![0; n_stocks]; self.n_accounts()];
        for (s, m) in self.markets.iter().enumerate() {
            for o in m.lock().resting_orders() {
                match o.side {
                    Side::Buy => {
                        book_buy_reserve[o.account as usize] +=
                            (o.qty * o.reserve_per_share) as i128
                    }
                    Side::Sell => book_sell[o.account as usize][s] += o.qty as i128,
                }
            }
        }
        for (i, acc) in self.accounts.iter().enumerate() {
            let a = acc.lock();
            let mail = self.mailboxes[i].lock();
            cash += a.cash as i128;
            let mut pending_buy_release: i128 = 0;
            let mut pending_sell: Vec<i128> = vec![0; n_stocks];
            for m in mail.iter() {
                cash += m.cash_effect() as i128;
                shares[m.stock] += m.share_effect() as i128;
                match m.side {
                    Side::Buy => pending_buy_release += (m.qty * m.reserve_per_share) as i128,
                    Side::Sell => pending_sell[m.stock] += m.qty as i128,
                }
            }
            for s in 0..n_stocks {
                shares[s] += a.shares[s] as i128;
                let held_after_mail = a.shares[s] as i128 - pending_sell[s];
                if held_after_mail < 0 || a.reserved_sell[s] < 0 {
                    bail!("{}: negative shares", a.name);
                }
                if a.reserved_sell[s] as i128 - pending_sell[s] != book_sell[i][s] {
                    bail!("{}: sell reservation mismatch on stock {s}", a.name);
                }
            }
            if a.reserved_buy as i128 - pending_buy_release != book_buy_reserve[i] {
                bail!("{}: buy reservation mismatch", a.name);
            }
        }
        if cash != self.initial_cash as i128 {
            bail!("cash not conserved: {} vs {}", cash, self.initial_cash);
        }
        for s in 0..n_stocks {
            if shares[s] != self.initial_shares[s] as i128 {
                bail!("stock {s} shares not conserved");
            }
        }
        Ok(())
    }
}
