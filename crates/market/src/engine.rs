//! The run loop: personal-tick scheduler, per-account settlement, order
//! handling with per-stock locks and mailbox delivery to makers.

use crate::account::{Account, Mail, OwnOrder};
use crate::book::{Resting, StockMarket};
use crate::config::Config;
use crate::world::World;
use anyhow::{bail, Result};
use bot_api::*;
use parking_lot::{Condvar, Mutex, RwLock};
use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicI64, AtomicU32, AtomicU64, Ordering};
use std::time::Instant;

pub type BotFactory<'f> = dyn Fn(&str, &Start) -> Option<Box<dyn Bot>> + Sync + 'f;

/// Optional read-only diagnostic hook. Called outside the bot decision timer.
/// Observers must not trade or modify simulation state. I/O can change concurrent
/// scheduling, so a traced match is a separate run, not a replay of another match.
pub trait MatchObserver: Sync {
    fn step(
        &self,
        account: &Account,
        tick: Tick,
        observation: Option<&Observation>,
        actions: &[Action],
        gross: Money,
    );
    fn finish(&self, account: &Account);
}

const MAX_CONSECUTIVE_BOT_ERRORS: u32 = 20;

/// Completed ticks only advance by one. The lower median therefore moves only
/// forward, at most `ticks` times across the entire run, rather than requiring
/// a tree search and two tree updates per account tick under the scheduler lock.
struct TickProgress {
    counts: Vec<usize>,
    rank: usize,
    reference: Tick,
    at_or_below: usize,
}

impl TickProgress {
    fn new(ticks: Tick, accounts: usize) -> Self {
        let mut counts = vec![0; ticks as usize + 1];
        counts[0] = accounts;
        Self {
            counts,
            rank: accounts.div_ceil(2),
            reference: 0,
            at_or_below: accounts,
        }
    }
    fn complete(&mut self, tick: Tick) {
        self.counts[tick as usize] -= 1;
        self.counts[tick as usize + 1] += 1;
        // Moving a value below the reference keeps it at or below; only a
        // value equal to the reference can cross this boundary in one step.
        if tick == self.reference {
            self.at_or_below -= 1;
        }
        while self.at_or_below < self.rank {
            self.reference += 1;
            self.at_or_below += self.counts[self.reference as usize];
        }
    }
}

struct Sched {
    idle: BinaryHeap<Reverse<(Tick, u32)>>,
    running: Vec<(u32, Tick)>,
    progress: TickProgress,
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
    pub markets: Vec<RwLock<StockMarket>>,
    marks: Vec<AtomicI64>,
    mark_ticks: Vec<AtomicU32>,
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
    pub book_samples: Mutex<Vec<BookSample>>,
    pub observer: Option<&'a dyn MatchObserver>,
    social_groups: Vec<Mutex<crate::social::Group>>,
}

/// Order book snapshot for market-quality statistics.
#[derive(Clone, Copy, Debug)]
pub struct BookSample {
    pub tick: Tick,
    pub stock: usize,
    pub bid: Option<Price>,
    pub ask: Option<Price>,
    /// Resting notional within 2% of the mid on each side.
    pub bid_depth: Money,
    pub ask_depth: Money,
}

pub struct RunOutput {
    pub wall_seconds: f64,
    pub threads: usize,
}

impl<'a> Shared<'a> {
    pub fn new(cfg: &'a Config, seed: u64, factory: &BotFactory<'_>) -> Result<Shared<'a>> {
        cfg.validate()?;
        let n_stocks = cfg.stocks.len();
        // with count_max, each world draws its own number of accounts per group
        let counts: Vec<usize> = cfg
            .bots
            .iter()
            .enumerate()
            .map(|(gi, g)| match g.count_max {
                Some(max) if max > g.count => {
                    let h = (seed ^ 0xc0_4e7).wrapping_mul(0x9e3779b97f4a7c15)
                        ^ (gi as u64 + 1).wrapping_mul(0xd1b54a32d192ed03);
                    let h = (h ^ (h >> 31)).wrapping_mul(0xbf58476d1ce4e5b9);
                    g.count + ((h >> 33) as usize) % (max - g.count + 1)
                }
                _ => g.count,
            })
            .collect();
        let mut accounts = Vec::new();
        let owner_offsets = cfg.auto_owner_offsets()?;
        for ((g, &count), base) in cfg.bots.iter().zip(&counts).zip(owner_offsets) {
            for i in 0..count {
                // one account per owner: without an explicit owner each account is its own
                let owner = g
                    .owner
                    .clone()
                    .unwrap_or_else(|| format!("{}#{}", g.bot, base + i));
                let mut shares = vec![g.shares_each; n_stocks];
                for (name, q) in &g.shares {
                    shares[cfg.stock_index(name)?] = *q;
                }
                let idx = accounts.len();
                accounts.push(Account::new(
                    idx,
                    owner.clone(),
                    owner.clone(),
                    g.bot.clone(),
                    g.cash,
                    shares,
                ));
            }
        }
        let n = accounts.len();
        if n == 0 {
            bail!("world {seed} has no accounts; give at least one group a positive minimum count");
        }
        crate::capital::redistribute(cfg, &counts, &mut accounts)?;
        let initial_shares: Vec<Qty> = (0..n_stocks)
            .map(|s| accounts.iter().map(|a| a.shares[s]).sum())
            .collect();
        let initial_cash = accounts.iter().map(|a| a.cash).sum();
        let news_multipliers: Vec<_> = cfg
            .bots
            .iter()
            .zip(&counts)
            .flat_map(|(g, &count)| std::iter::repeat_n(g.news_multiplier, count))
            .collect();
        let forecasts: Vec<_> = cfg
            .bots
            .iter()
            .zip(&counts)
            .flat_map(|(g, &count)| std::iter::repeat_n(g.forecast_access, count))
            .collect();
        let world = World::generate_with_information(cfg, seed, &news_multipliers, &forecasts);
        let mut rules = cfg.rules(&initial_shares);
        for (st, &p) in rules.stocks.iter_mut().zip(&world.opening_prices) {
            st.initial_price = p;
        }

        let mut social_names = std::collections::HashMap::new();
        let mut social_groups: Vec<Mutex<crate::social::Group>> = Vec::new();
        // create bots
        let mut idx = 0;
        for (g, &count) in cfg.bots.iter().zip(&counts) {
            for _ in 0..count {
                let a = &mut accounts[idx];
                if let Some(name) = &g.social_group {
                    let group = *social_names.entry(name.clone()).or_insert_with(|| {
                        social_groups.push(Mutex::new(crate::social::Group::new(n_stocks)));
                        social_groups.len() - 1
                    });
                    social_groups[group].get_mut().members += 1;
                    a.social = Some(crate::social::Tracker::new(
                        group,
                        a.cash,
                        &a.shares,
                        &world.opening_prices,
                    ));
                }
                let start = Start {
                    account: a.name.clone(),
                    owner: a.owner.clone(),
                    account_index: idx,
                    n_accounts: n,
                    news_multiplier: g.news_multiplier,
                    forecast_access: g.forecast_access,
                    // Intentionally reversible: every bot is reviewed before it runs.
                    // Review forbids reconstructing hidden world information. Seed
                    // derivability itself is accepted, not an information-isolation bug.
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

        let idle = (0..n as u32).map(|i| Reverse((0, i))).collect();
        let check = cfg.schedule.check_interval;
        Ok(Shared {
            markets: world
                .opening_prices
                .iter()
                .map(|&p| {
                    RwLock::new(StockMarket::with_depth_cache(
                        p,
                        cfg.margin.mark_window,
                        cfg.market.book_depth,
                    ))
                })
                .collect(),
            marks: world
                .opening_prices
                .iter()
                .map(|&p| AtomicI64::new(p))
                .collect(),
            mark_ticks: (0..n_stocks).map(|_| AtomicU32::new(0)).collect(),
            accounts: accounts.into_iter().map(Mutex::new).collect(),
            mailboxes: (0..n).map(|_| Mutex::new(Vec::new())).collect(),
            treasury: AtomicI64::new(0),
            burned: (0..n_stocks).map(|_| AtomicI64::new(0)).collect(),
            next_order_id: AtomicU64::new(1),
            sched: Mutex::new(Sched {
                idle,
                running: Vec::new(),
                progress: TickProgress::new(cfg.ticks, n),
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
            book_samples: Mutex::new(Vec::new()),
            observer: None,
            social_groups,
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

    fn gross_at_marks(&self, a: &Account) -> Money {
        a.shares
            .iter()
            .zip(&self.marks)
            .map(|(shares, mark)| shares * mark.load(Ordering::Relaxed))
            .sum()
    }

    /// The mark clock is the highest personal tick reached for each book, as
    /// it already was for trades. Refresh before settlement/risk checks; other
    /// accounts at the same or an older tick only read the published counters.
    fn advance_marks(&self, tick: Tick) {
        for (s, market) in self.markets.iter().enumerate() {
            if self.mark_ticks[s].load(Ordering::Acquire) >= tick {
                continue;
            }
            let mut m = market.write();
            let old_mark = m.mark;
            m.advance_to(tick);
            if m.mark != old_mark {
                self.marks[s].store(m.mark, Ordering::Relaxed);
            }
            self.mark_ticks[s].store(m.mark_tick(), Ordering::Release);
        }
    }

    // ---- scheduler -----------------------------------------------------

    pub fn run(&self) -> Result<RunOutput> {
        let threads = self.cfg.schedule.worker_threads(self.n_accounts());
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
            if let Some(observer) = self.observer {
                observer.finish(&a);
            }
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
        let mut obs = self.empty_observation();
        // Complete one job and claim the next under the same scheduler guard.
        // Dropping/reacquiring it between those operations doubles lock traffic.
        let mut s = self.sched.lock();
        loop {
            let (idx, tick, reference) = loop {
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
                        let reference = s.progress.reference;
                        break (i as usize, t, reference);
                    }
                }
                self.cv.wait(&mut s);
            };
            drop(s);
            let result = catch_unwind(AssertUnwindSafe(|| {
                self.step_with_observation(idx, tick, reference, &mut obs)
            }));
            s = self.sched.lock();
            if result.is_err() {
                s.error = Some(format!(
                    "engine panic while stepping account {idx} at tick {tick}"
                ));
            }
            s.running.retain(|r| r.0 as usize != idx);
            s.progress.complete(tick);
            if tick + 1 < ticks {
                s.idle.push(Reverse((tick + 1, idx as u32)));
            } else {
                s.finished += 1;
            }
            self.cv.notify_all();
        }
    }

    // ---- one personal tick ------------------------------------------------

    fn sample_books(&self, tick: Tick) {
        let mut out = Vec::with_capacity(self.markets.len());
        for (stock, m) in self.markets.iter().enumerate() {
            let m = m.read();
            let (bid, ask) = (m.best(Side::Buy), m.best(Side::Sell));
            let (mut bid_depth, mut ask_depth) = (0, 0);
            if let (Some(b), Some(a)) = (bid, ask) {
                let mid = (a + b) as f64 / 2.0;
                for l in m.depth(Side::Buy, 100) {
                    if (l.price as f64) >= mid * 0.98 {
                        bid_depth += l.price * l.qty;
                    }
                }
                for l in m.depth(Side::Sell, 100) {
                    if (l.price as f64) <= mid * 1.02 {
                        ask_depth += l.price * l.qty;
                    }
                }
            }
            out.push(BookSample {
                tick,
                stock,
                bid,
                ask,
                bid_depth,
                ask_depth,
            });
        }
        self.book_samples.lock().extend(out);
    }

    #[cfg(test)]
    fn step(&self, idx: usize, tick: Tick, reference: Tick) {
        self.step_with_observation(idx, tick, reference, &mut self.empty_observation());
    }

    fn step_with_observation(
        &self,
        idx: usize,
        tick: Tick,
        reference: Tick,
        obs: &mut Observation,
    ) {
        self.advance_marks(tick);
        if idx == 0 && tick.is_multiple_of(10) {
            self.sample_books(tick);
        }
        let mut guard = self.accounts[idx].lock();
        let a = &mut *guard;
        let mut observed = false;
        let mut traced_actions = Vec::new();
        self.drain_mailbox(a);
        self.settle_world(a, tick);
        self.periodic_charges(a, tick);
        self.check_liquidation(a, tick);

        let lag = reference.saturating_sub(tick);
        let budget_us = self.cfg.schedule.decision_budget_us;
        let skip = self.cfg.schedule.skip_decision_lag > 0
            && lag >= self.cfg.schedule.skip_decision_lag
            && (budget_us == 0 || a.decision_pace.is_slow());
        if a.disabled.is_some() || a.bot.is_none() {
            // passive holder
        } else if skip {
            a.stats.skipped += 1;
            a.skipped_since += 1;
        } else {
            self.observation(a, tick, reference, obs);
            observed = true;
            let mut bot = a.bot.take().unwrap();
            let t0 = Instant::now();
            let res = catch_unwind(AssertUnwindSafe(|| bot.on_tick(obs)));
            let ns = t0.elapsed().as_nanos() as u64;
            a.stats.decisions += 1;
            a.stats.decision_nanos_total += ns;
            a.stats.decision_nanos_max = a.stats.decision_nanos_max.max(ns);
            a.decision_pace.record(ns, budget_us);
            if budget_us > 0 && ns > budget_us.saturating_mul(1000) {
                a.stats.over_budget_decisions += 1;
            }
            match res {
                Ok(actions) => {
                    if self.observer.is_some() {
                        traced_actions.clone_from(&actions);
                    }
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
        let gross = self.gross_at_marks(a);
        let equity = a.cash + gross;
        if let Some(observer) = self.observer {
            observer.step(a, tick, observed.then_some(obs), &traced_actions, gross);
        }
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
            a.inbox.news.push(w.delivery_view(d, a.idx));
            a.delivery_cursor += 1;
        }
        while a.hint_cursor < w.hints_due(tick) {
            a.inbox
                .terminal_hints
                .push(w.terminal_hint(a.idx, a.hint_cursor));
            a.hint_cursor += 1;
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
        let mut total = 0;
        if tick > 0 && tick.is_multiple_of(period) {
            for s in 0..a.shares.len() {
                let amt = self
                    .rules
                    .dividend_payment(a.shares[s], w.dividend(s, tick));
                if amt != 0 {
                    total += amt;
                    a.stats.dividends += amt;
                    a.inbox.income.push(Income::Dividend {
                        stock: s,
                        amount: amt,
                    });
                }
            }
        }
        if tick > 0 && tick.is_multiple_of(self.rules.combo_period) {
            for (r, c) in self.rules.combos.iter().enumerate() {
                if c.active(tick) {
                    let amt = self.rules.combo_payment(c.sets(&a.shares), c.bonus);
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
        }
        if total != 0 {
            a.cash += total;
            self.treasury.fetch_sub(total, Ordering::Relaxed);
        }
    }

    fn periodic_charges(&self, a: &mut Account, tick: Tick) {
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
            let credit_applied = credit.min(gross_fee - a.idle_fee_carry);
            let paid = if due > 0 {
                self.charge_idle(a, due, &self.marks())
            } else {
                0
            };
            if gross_fee > 0 {
                a.inbox.income.push(Income::IdleFee {
                    paid,
                    gross: gross_fee,
                    credit: credit_applied,
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
        (
            fee + a.idle_fee_carry,
            credit,
            (fee - credit).max(0) + a.idle_fee_carry,
        )
    }

    /// Takes `due` from positive cash and every stock in the same proportion. Cash goes to
    /// the treasury, shares are burned.
    fn charge_idle(&self, a: &mut Account, due: Money, marks: &[Price]) -> Money {
        let pos_cash = a.cash.max(0);
        let base = pos_cash + a.gross(marks);
        if base <= 0 {
            a.idle_fee_carry = due;
            return 0;
        }
        let cash_part = ((pos_cash as i128 * due as i128) / base as i128) as Money;
        let mut paid = 0;
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
            if a.sellable(s) < whole {
                self.free_shares(a, s, whole);
            }
            // A concurrent taker may already have bought these shares. Cancel
            // only frees the resting remainder; pending fills stay reserved.
            // Defer the uncollectible fraction until shares are available again.
            let whole = whole.min(a.sellable(s).max(0));
            a.share_frac[s] -= whole * 1_000_000;
            a.shares[s] -= whole;
            a.stats.burned_shares += whole;
            self.burned[s].fetch_add(whole, Ordering::Relaxed);
            paid += whole * marks[s];
        }
        // Stock burns also reduce equity and therefore the capacity backing
        // resting buys. Cancel newest buys against the post-fee balance sheet.
        let gross = a.gross(marks);
        let capacity = ((self.rules.max_leverage * (a.cash - cash_part + gross) as f64) as Money
            - gross)
            .max(0);
        if a.reserved_buy > capacity {
            let mut buys: Vec<_> = a
                .orders
                .iter()
                .filter(|(_, o)| o.live && o.side == Side::Buy)
                .map(|(id, o)| (*id, o.placed_tick))
                .collect();
            buys.sort_by_key(|(id, t)| Reverse((*t, *id)));
            for (id, _) in buys {
                if a.reserved_buy <= capacity {
                    break;
                }
                self.cancel_order(a, id, AckStatus::Cancelled);
            }
        }
        // Cancellation cannot release buys already matched by another worker.
        // Keep their reservation intact, just as idle burns preserve pending
        // sells. Carry the uncollectible fee into the next period, without
        // advancing mailbox delivery or discarding the amount owed.
        let headroom = self.buying_power(a, a.cash + gross, gross);
        let affordable = (headroom as f64 / self.rules.max_leverage).floor() as Money;
        let cash_paid = cash_part.min(pos_cash).min(affordable);
        a.idle_fee_carry = cash_part - cash_paid;
        a.cash -= cash_paid;
        self.treasury.fetch_add(cash_paid, Ordering::Relaxed);
        paid += cash_paid;
        a.stats.idle_fees += paid;
        a.stats.idle_cash_fees += cash_paid;
        a.stats.idle_stock_fees += paid - cash_paid;
        paid
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
        let gross = self.gross_at_marks(a);
        let equity = a.cash + gross;
        let breach = if equity <= 0 {
            gross > 0 || a.cash < 0
        } else {
            gross as f64 > self.rules.maintenance_leverage * equity as f64
        };
        if !breach {
            return;
        }
        let mut event = crate::account::LiquidationEvent {
            tick,
            cash_before: a.cash,
            gross_before: gross,
            shares_before: a.shares.clone(),
            marks_before: self
                .marks
                .iter()
                .map(|m| m.load(Ordering::Relaxed))
                .collect(),
            cash_after: 0,
            gross_after: 0,
            shares_after: Vec::new(),
            sale_notional: 0,
            fills: Vec::new(),
            trade_fees: 0,
            bad_debt: 0,
            pending_orders: 0,
        };
        let fees_before = a.stats.trade_fees;
        let fill_start = a.inbox.fills.len();
        let debt_before = a.stats.bad_debt;
        a.stats.liquidations += 1;
        a.inbox.income.push(Income::Liquidation);
        self.cancel_all(a, None, AckStatus::CancelledByLiquidation);
        loop {
            let gross = self.gross_at_marks(a);
            let equity = a.cash + gross;
            if gross == 0 || (equity > 0 && gross as f64 <= self.rules.max_leverage * equity as f64)
            {
                break;
            }
            let frac = if equity <= 0 {
                1.0
            } else {
                let target = self.rules.max_leverage * 0.9 * equity as f64;
                ((gross as f64 - target) / gross as f64).clamp(0.0, 1.0)
            };
            let mut sold = false;
            for s in 0..a.shares.len() {
                let q = ((a.shares[s] as f64) * frac).ceil() as Qty;
                let q = q.min(a.sellable(s)).min(crate::config::MAX_SHARES);
                if q > 0 {
                    let before = a.shares[s];
                    self.place(a, tick, Action::market(s, Side::Sell, q), true);
                    sold |= a.shares[s] < before;
                }
            }
            // Reassess actual prices/fees before the next proportional pass.
            // Every successful pass reduces inventory; an empty book or only
            // pending sells stops immediately rather than spinning forever.
            if !sold {
                break;
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
        event.cash_after = a.cash;
        event.gross_after = self.gross_at_marks(a);
        event.shares_after = a.shares.clone();
        event.trade_fees = a.stats.trade_fees - fees_before;
        event.bad_debt = a.stats.bad_debt - debt_before;
        event.fills = a.inbox.fills[fill_start..].to_vec();
        event.sale_notional = event.fills.iter().map(|f| f.price * f.qty).sum();
        event.pending_orders = a.orders.len();
        a.liquidation_events.push(event);
    }

    // ---- observation -------------------------------------------------------

    fn empty_observation(&self) -> Observation {
        Observation {
            tick: 0,
            ticks_total: self.rules.ticks_total,
            reference_tick: 0,
            season: self.world.season(0),
            account: AccountView::default(),
            open_orders: Vec::new(),
            books: vec![BookView::default(); self.markets.len()],
            trades: vec![Vec::new(); self.markets.len()],
            skipped: 0,
            fills: Vec::new(),
            acks: Vec::new(),
            rejects: Vec::new(),
            news: Vec::new(),
            season_changes: Vec::new(),
            income: Vec::new(),
            terminal_hints: Vec::new(),
            social: None,
        }
    }

    fn observation(&self, a: &mut Account, tick: Tick, reference: Tick, obs: &mut Observation) {
        let depth = self.rules.book_depth;
        let limit = self.cfg.market.trade_tape_limit;
        for (s, m) in self.markets.iter().enumerate() {
            let m = m.read();
            let book = &mut obs.books[s];
            m.depth_into(Side::Buy, depth, &mut book.bids);
            m.depth_into(Side::Sell, depth, &mut book.asks);
            book.last = m.last;
            book.mark = m.mark;
            book.dividend = self.world.dividend(s, tick);
            let start = a.tape_seen[s].max(m.tape.len().saturating_sub(limit));
            obs.trades[s].clear();
            obs.trades[s].extend(m.tape[start..].iter().map(|e| Trade {
                tick: e.tick,
                price: e.price,
                qty: e.qty,
                aggressor: e.aggressor,
            }));
            a.tape_seen[s] = m.tape.len();
        }
        let gross: Money = a
            .shares
            .iter()
            .zip(&obs.books)
            .map(|(q, b)| q * b.mark)
            .sum();
        let equity = a.cash + gross;
        let (fee, credit, due) = self.idle_estimate(a);
        let period_end = (tick / self.rules.idle_period + 1) * self.rules.idle_period;
        let avg_loan = if a.loan_samples > 0 {
            (a.loan_sum / a.loan_samples as i128) as Money
        } else {
            (-a.cash).max(0)
        };
        let shares = &mut obs.account.shares;
        shares.clone_from(&a.shares);
        let sellable = &mut obs.account.sellable;
        sellable.clear();
        sellable.extend((0..a.shares.len()).map(|s| a.sellable(s)));
        obs.account = AccountView {
            cash: a.cash,
            shares: std::mem::take(&mut obs.account.shares),
            sellable: std::mem::take(&mut obs.account.sellable),
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
        obs.open_orders.clear();
        obs.open_orders.extend(
            a.orders
                .iter()
                .filter(|(_, o)| o.live)
                .map(|(id, o)| OpenOrder {
                    order_id: *id,
                    stock: o.stock,
                    side: o.side,
                    price: o.price,
                    remaining: o.remaining,
                    placed_tick: o.placed_tick,
                }),
        );
        // OrderStore already yields increasing IDs, including after removals.
        obs.tick = tick;
        obs.reference_tick = reference;
        obs.season = self.world.season(tick);
        obs.skipped = std::mem::take(&mut a.skipped_since);
        // Give the account empty buffers for subsequent events. The worker may
        // serve another account next; never retain any previous event contents.
        fn receive<T>(out: &mut Vec<T>, incoming: &mut Vec<T>) {
            out.clear();
            std::mem::swap(out, incoming);
        }
        receive(&mut obs.fills, &mut a.inbox.fills);
        receive(&mut obs.acks, &mut a.inbox.acks);
        receive(&mut obs.rejects, &mut a.inbox.rejects);
        receive(&mut obs.news, &mut a.inbox.news);
        receive(&mut obs.season_changes, &mut a.inbox.season_changes);
        receive(&mut obs.income, &mut a.inbox.income);
        receive(&mut obs.terminal_hints, &mut a.inbox.terminal_hints);
        // No other account lock, no world barrier. Only opted-in members pay
        // this O(stocks + fills) cost. Reset when a worker switches accounts.
        obs.social = if let Some(member) = &mut a.social {
            member.update(obs);
            Some(self.social_groups[member.group].lock().publish(member))
        } else {
            None
        };
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
        let on_book = self.markets[stock].write().cancel(id);
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
                let gross = self.gross_at_marks(a);
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
        let (fills, rested) = {
            let mut m = self.markets[stock].write();
            let old_mark = m.mark;
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
            if m.mark != old_mark {
                // Avoid invalidating readers' cache lines on unchanged prices.
                // Publish under the book lock so an older writer cannot replace
                // a newer mark after releasing the lock.
                self.marks[stock].store(m.mark, Ordering::Relaxed);
            }
            if m.mark_tick() > self.mark_ticks[stock].load(Ordering::Relaxed) {
                self.mark_ticks[stock].store(m.mark_tick(), Ordering::Release);
            }
            (fills, rested)
        };

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
            for o in m.read().resting_orders() {
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

#[cfg(test)]
mod scheduling_tests {
    use super::*;

    #[test]
    fn published_marks_follow_books_after_concurrent_orders() {
        struct Trader(usize);
        impl Bot for Trader {
            fn on_tick(&mut self, obs: &Observation) -> Vec<Action> {
                let stock = self.0 % obs.books.len();
                let side = if self.0 < obs.books.len() {
                    Side::Sell
                } else {
                    Side::Buy
                };
                let price = if side == Side::Sell {
                    100 + i64::from(obs.tick % 17)
                } else {
                    200
                };
                vec![
                    Action::CancelAll { stock: Some(stock) },
                    Action::limit(stock, side, price, 1),
                ]
            }
        }
        let mut cfg = config();
        cfg.bots[0].count = cfg.stocks.len() * 2;
        cfg.bots[0].cash = 1_000_000;
        cfg.bots[0].shares_each = 1000;
        cfg.schedule.threads = 4;
        cfg.schedule.check_interval = 13;
        let factory = |_: &str, start: &Start| -> Option<Box<dyn Bot>> {
            Some(Box::new(Trader(start.account_index)))
        };
        let shared = Shared::new(&cfg, 7, &factory).unwrap();
        shared.run().unwrap();
        assert!(shared.markets.iter().any(|m| !m.read().tape.is_empty()));
        for (stock, market) in shared.markets.iter().enumerate() {
            assert_eq!(
                shared.marks[stock].load(Ordering::Relaxed),
                market.read().mark
            );
        }
    }

    #[test]
    fn incremental_reference_matches_sorted_accounts_with_skew_and_early_finishes() {
        use rand::{Rng, SeedableRng};
        let mut rng = rand_chacha::ChaCha8Rng::seed_from_u64(37);
        for accounts in [1, 2, 3, 4, 52, 53] {
            let ticks = 91;
            let mut progress = TickProgress::new(ticks, accounts);
            let mut completed = vec![0; accounts];
            let mut active: Vec<_> = (0..accounts).collect();
            while !active.is_empty() {
                let pos = if rng.gen_bool(0.5) {
                    0
                } else {
                    rng.gen_range(0..active.len())
                };
                let account = active[pos];
                progress.complete(completed[account]);
                completed[account] += 1;
                if completed[account] == ticks {
                    active.swap_remove(pos);
                }
                let mut sorted = completed.clone();
                sorted.sort_unstable();
                assert_eq!(progress.reference, sorted[(accounts - 1) / 2]);
                assert_eq!(
                    progress.at_or_below,
                    completed
                        .iter()
                        .filter(|&&t| t <= progress.reference)
                        .count()
                );
            }
            assert_eq!(progress.reference, ticks);
            assert_eq!(progress.counts[ticks as usize], accounts);
        }
    }

    struct Idle;
    impl Bot for Idle {
        fn on_tick(&mut self, _: &Observation) -> Vec<Action> {
            vec![]
        }
    }

    fn config() -> Config {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../scenarios/baseline-v6.toml"
        );
        let mut c = Config::load(std::path::Path::new(path)).unwrap();
        c.ticks = 100;
        c.bots.truncate(1);
        c.bots[0].count = 1;
        c.bots[0].count_max = None;
        c.fees.idle_rate = 0.0;
        c.world.news.market.events_per_1000_ticks = 0.0;
        c.world.news.sector.events_per_1000_ticks = 0.0;
        c.world.news.stock.events_per_1000_ticks = 0.0;
        c
    }

    #[test]
    fn scheduler_handles_skew_waits_audits_and_finishing_workers() {
        use std::sync::{atomic::AtomicU32, Arc};
        use std::time::Duration;

        struct Delayed {
            idx: usize,
            calls: Arc<Vec<AtomicU32>>,
        }
        impl Bot for Delayed {
            fn on_tick(&mut self, obs: &Observation) -> Vec<Action> {
                assert_eq!(
                    self.calls[self.idx].fetch_add(1, Ordering::Relaxed),
                    obs.tick
                );
                if self.idx == 0 && obs.tick.is_multiple_of(7) {
                    std::thread::sleep(Duration::from_millis(1));
                }
                vec![]
            }
        }

        for accounts in [1, 4, 7] {
            let mut cfg = config();
            cfg.ticks = 19;
            cfg.bots[0].count = accounts;
            cfg.schedule.threads = 4;
            cfg.schedule.max_tick_skew = 1;
            cfg.schedule.skip_decision_lag = 0;
            cfg.schedule.check_interval = 3;
            let calls = Arc::new((0..accounts).map(|_| AtomicU32::new(0)).collect::<Vec<_>>());
            let factory = |_: &str, start: &Start| -> Option<Box<dyn Bot>> {
                Some(Box::new(Delayed {
                    idx: start.account_index,
                    calls: Arc::clone(&calls),
                }))
            };
            let shared = Shared::new(&cfg, 1, &factory).unwrap();
            let run = shared.run().unwrap();
            assert_eq!(run.threads, cfg.schedule.worker_threads(accounts));
            assert!(shared.audits.load(Ordering::Relaxed) >= 2);
            for (idx, account) in shared.accounts.iter().enumerate() {
                assert_eq!(calls[idx].load(Ordering::Relaxed), cfg.ticks);
                assert_eq!(account.lock().stats.bot_errors, 0);
                assert_eq!(account.lock().stats.skipped, 0);
            }
        }
    }

    #[test]
    fn reused_observation_keeps_account_cursors_and_delivers_events_once() {
        let mut cfg = config();
        cfg.bots[0].count = 2;
        cfg.market.trade_tape_limit = 1;
        let factory = |_: &str, _: &Start| -> Option<Box<dyn Bot>> { Some(Box::new(Idle)) };
        let shared = Shared::new(&cfg, 1, &factory).unwrap();
        let mut obs = shared.empty_observation();
        {
            let mut seller = shared.accounts[0].lock();
            shared.place(&mut seller, 0, Action::limit(0, Side::Sell, 100, 10), false);
            seller.inbox.terminal_hints.push((10, 20));
        }
        {
            let mut buyer = shared.accounts[1].lock();
            for tick in [1, 2] {
                shared.place(&mut buyer, tick, Action::market(0, Side::Buy, 1), false);
            }
        }
        {
            let mut seller = shared.accounts[0].lock();
            shared.drain_mailbox(&mut seller);
            shared.observation(&mut seller, 3, 3, &mut obs);
            assert_eq!(obs.fills.len(), 2);
            assert_eq!(obs.acks.len(), 1);
            assert_eq!(obs.open_orders[0].remaining, 8);
            assert_eq!(obs.terminal_hints, [(10, 20)]);
            assert_eq!(obs.trades[0].len(), 1);
            assert_eq!(obs.trades[0][0].tick, 2);
        }
        {
            let mut buyer = shared.accounts[1].lock();
            shared.observation(&mut buyer, 3, 3, &mut obs);
            assert_eq!(obs.account.shares, buyer.shares);
            assert_eq!(obs.fills.len(), 2);
            assert!(obs.fills.iter().all(|fill| !fill.maker));
            assert_eq!(obs.acks.len(), 2);
            assert!(obs.open_orders.is_empty());
            assert!(obs.terminal_hints.is_empty());
            assert_eq!(obs.trades[0].len(), 1);
        }
        {
            let mut seller = shared.accounts[0].lock();
            shared.observation(&mut seller, 4, 4, &mut obs);
            assert!(obs.fills.is_empty());
            assert!(obs.acks.is_empty());
            assert!(obs.terminal_hints.is_empty());
            assert!(obs.trades.iter().all(Vec::is_empty));
            assert_eq!(obs.books[0].asks[0].qty, 8);
            shared.cancel_all(&mut seller, None, AckStatus::Cancelled);
            shared.observation(&mut seller, 5, 5, &mut obs);
            assert!(obs.books[0].asks.is_empty());
            assert!(obs.open_orders.is_empty());
            assert_eq!(obs.acks.len(), 1);
        }
        shared.audit().unwrap();
    }

    #[test]
    fn ordinary_lagging_bot_keeps_every_decision() {
        let cfg = config();
        let factory = |_: &str, _: &Start| -> Option<Box<dyn Bot>> { Some(Box::new(Idle)) };
        let shared = Shared::new(&cfg, 1, &factory).unwrap();
        for tick in 0..100 {
            shared.step(0, tick, (tick + 10).min(100));
        }
        let a = shared.accounts[0].lock();
        assert_eq!(a.stats.decisions, 100);
        assert_eq!(a.stats.skipped, 0);
        drop(a);
        shared.audit().unwrap();
    }

    #[test]
    fn slow_lagging_bot_skips_decision_but_still_settles_and_recovers() {
        let cfg = config();
        let factory = |_: &str, _: &Start| -> Option<Box<dyn Bot>> { Some(Box::new(Idle)) };
        let shared = Shared::new(&cfg, 1, &factory).unwrap();
        for tick in 0..50 {
            shared.step(0, tick, tick);
        }
        {
            let mut a = shared.accounts[0].lock();
            for _ in 0..8 {
                a.decision_pace
                    .record(10_000_000, cfg.schedule.decision_budget_us);
            }
        }
        shared.step(0, 50, 60);
        {
            let a = shared.accounts[0].lock();
            assert_eq!(a.stats.skipped, 1);
            assert_eq!(a.stats.decisions, 50);
            let expected: Money = cfg
                .stocks
                .iter()
                .map(|s| s.dividend * cfg.bots[0].shares_each)
                .sum();
            assert_eq!(
                a.stats.dividends, expected,
                "a skipped decision must not skip the dividend"
            );
        }
        // Once caught up, a slow bot can decide again and demonstrate recovery.
        for tick in 51..54 {
            shared.step(0, tick, tick);
        }
        shared.step(0, 54, 64);
        let a = shared.accounts[0].lock();
        assert_eq!(a.stats.skipped, 1);
        assert_eq!(a.stats.decisions, 54);
        drop(a);
        shared.audit().unwrap();
    }

    #[test]
    fn legacy_policy_and_explicit_skip_disable_remain_available() {
        let mut cfg = config();
        cfg.schedule.decision_budget_us = 0;
        let factory = |_: &str, _: &Start| -> Option<Box<dyn Bot>> { Some(Box::new(Idle)) };
        let shared = Shared::new(&cfg, 1, &factory).unwrap();
        shared.step(0, 0, 10);
        assert_eq!(shared.accounts[0].lock().stats.skipped, 1);
        cfg.schedule.skip_decision_lag = 0;
        let shared = Shared::new(&cfg, 1, &factory).unwrap();
        shared.step(0, 0, 10);
        assert_eq!(shared.accounts[0].lock().stats.decisions, 1);
    }

    #[test]
    fn idle_burn_preserves_shares_already_sold_with_pending_settlement() {
        // Reproduce fills arriving between a tick's mailbox drain and idle fee.
        // Cancel releases only the still-resting quantity, never pending fills.
        for (sold, owed, expected_burn) in [(20, 1, 0), (13, 10, 7)] {
            let mut cfg = config();
            cfg.bots[0].count = 2;
            cfg.bots[0].shares_each = 20;
            let factory = |_: &str, _: &Start| -> Option<Box<dyn Bot>> { Some(Box::new(Idle)) };
            let shared = Shared::new(&cfg, 1, &factory).unwrap();
            let marks = shared.marks();
            {
                let mut seller = shared.accounts[0].lock();
                shared.place(
                    &mut seller,
                    1,
                    Action::Limit {
                        stock: 0,
                        side: Side::Sell,
                        price: marks[0],
                        qty: 20,
                        tif: Tif::Gtc,
                        cid: 0,
                    },
                    false,
                );
            }
            {
                let mut buyer = shared.accounts[1].lock();
                shared.place(
                    &mut buyer,
                    1,
                    Action::Limit {
                        stock: 0,
                        side: Side::Buy,
                        price: marks[0],
                        qty: sold,
                        tif: Tif::Ioc,
                        cid: 0,
                    },
                    false,
                );
            }
            {
                let mut seller = shared.accounts[0].lock();
                assert_eq!(seller.shares[0], 20);
                seller.share_frac[0] = owed * 1_000_000;
                shared.charge_idle(&mut seller, 1, &marks);
                assert_eq!(seller.stats.burned_shares, expected_burn);
                assert!(seller.share_frac[0] >= (owed - expected_burn) * 1_000_000);
            }
            shared.audit().unwrap();
            {
                let mut seller = shared.accounts[0].lock();
                shared.drain_mailbox(&mut seller);
                assert_eq!(seller.shares[0], 20 - sold - expected_burn);
                assert_eq!(seller.reserved_sell[0], 0);
            }
            shared.audit().unwrap();
        }
    }

    #[test]
    fn idle_fee_preserves_pending_buy_reservations_and_collects_carry_once() {
        for (bought, due, cash_paid, carry) in
            [(10, 300, 0, 100), (7, 300, 100, 0), (9, 600, 100, 100)]
        {
            let mut cfg = config();
            cfg.stocks.truncate(1);
            cfg.combos.clear();
            cfg.stocks[0].initial_price = 100;
            cfg.market.opening_spread = None;
            cfg.bots[0].count = 2;
            cfg.bots[0].cash = 1000;
            cfg.bots[0].shares_each = 20;
            cfg.fees.trade_fee_bps = 0;
            cfg.fees.credit_ratio = 0.0;
            cfg.margin.max_leverage = 1.0;
            let shared = Shared::new(&cfg, 1, &|_, _| Some(Box::new(Idle))).unwrap();
            {
                let mut buyer = shared.accounts[0].lock();
                shared.place(&mut buyer, 0, Action::limit(0, Side::Buy, 100, 10), false);
            }
            {
                let mut seller = shared.accounts[1].lock();
                shared.place(&mut seller, 1, Action::market(0, Side::Sell, bought), false);
            }
            {
                let mut buyer = shared.accounts[0].lock();
                let paid = shared.charge_idle(&mut buyer, due, &[100]);
                assert_eq!(buyer.cash, 1000 - cash_paid);
                assert_eq!(buyer.idle_fee_carry, carry);
                assert_eq!(buyer.reserved_buy, bought * 100);
                assert_eq!(paid, due - carry);
                assert!(
                    buyer.inbox.fills.is_empty(),
                    "fee collection must not consume maker mail early"
                );
            }
            shared.audit().unwrap();
            {
                let mut buyer = shared.accounts[0].lock();
                shared.drain_mailbox(&mut buyer);
                assert_eq!(buyer.cash, 1000 - cash_paid - bought * 100);
                assert!(buyer.cash >= 0);
                assert_eq!(buyer.reserved_buy, 0);
                assert_eq!(buyer.inbox.fills.len(), 1);
                let (_, _, owed) = shared.idle_estimate(&buyer);
                assert_eq!(owed, carry);
                if owed > 0 {
                    shared.charge_idle(&mut buyer, owed, &[100]);
                }
                assert_eq!(buyer.idle_fee_carry, 0);
                assert_eq!(buyer.stats.idle_fees, due);
                shared.drain_mailbox(&mut buyer);
                assert_eq!(
                    buyer.inbox.fills.len(),
                    1,
                    "maker fill must not be replayed"
                );
            }
            shared.audit().unwrap();
        }
    }

    #[test]
    fn stock_idle_burn_trims_buys_backed_by_margin_equity() {
        let mut cfg = config();
        cfg.stocks.truncate(1);
        cfg.combos.clear();
        cfg.stocks[0].initial_price = 100;
        cfg.market.opening_spread = None;
        cfg.bots[0].cash = 1000;
        cfg.bots[0].shares_each = 20;
        cfg.fees.trade_fee_bps = 0;
        let shared = Shared::new(&cfg, 1, &|_, _| Some(Box::new(Idle))).unwrap();
        {
            let mut a = shared.accounts[0].lock();
            // G=2000, E=3000, L=4: original buying power=10000.
            shared.place(&mut a, 0, Action::limit(0, Side::Buy, 100, 50), false);
            shared.place(&mut a, 1, Action::limit(0, Side::Buy, 100, 50), false);
            assert_eq!(a.reserved_buy, 10000);
            shared.charge_idle(&mut a, 300, &[100]);
            assert_eq!(a.cash, 900);
            assert_eq!(a.shares, [18]);
            assert_eq!(a.reserved_buy, 5000);
            assert_eq!(a.live_orders(), 1);
            assert_eq!(a.orders.values().next().unwrap().placed_tick, 0);
            assert_eq!(a.idle_fee_carry, 0);
        }
        shared.audit().unwrap();
    }
}
