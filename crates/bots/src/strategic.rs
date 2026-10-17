//! Strategic bots, written to exploit the reference bots and each other.
//!
//! What they lean on:
//! - A share is worth its remaining dividends plus the terminal payments, nothing else,
//!   so prices far above that are a gift to whoever sells.
//! - The idle fee is offset by 1% of traded notional, so trading about 20% of equity per
//!   idle period makes it free, and crossing a spread under ~1.8% to get there still pays.
//! - Private news arrives before the price moves.
//! - Combo bonuses make a share worth more to the account that completes a set with it.

use crate::common::*;
use bot_api::*;

/// Dividend payments still to come, in-game and terminal.
fn payments_left(rules: &Rules, tick: Tick) -> i64 {
    payments_after(rules, tick, rules.ticks_total) + rules.terminal_payments
}

/// Marginal combo value of buying (`buy`) or giving up (`!buy`) one share of `stock`.
fn combo_adjust(rules: &Rules, tick: Tick, shares: &[Qty], stock: usize, buy: bool) -> f64 {
    let p = payments_left(rules, tick) as f64;
    let mut adj = 0.0;
    for c in rules.combos.iter().filter(|c| c.end_tick >= tick) {
        let (mine, my_units, other, other_units) = if c.a == stock {
            (c.a, c.a_units, c.b, c.b_units)
        } else if c.b == stock {
            (c.b, c.b_units, c.a, c.a_units)
        } else {
            continue;
        };
        let have = shares[mine] / my_units;
        let partner = shares[other] / other_units;
        let binding = if buy {
            have < partner
        } else {
            have <= partner && have > 0
        };
        if binding {
            adj += c.bonus as f64 * p / my_units as f64;
        }
    }
    adj
}

/// Desired resting orders and one optional immediate order for a stock.
#[derive(Clone, Copy, Default)]
struct Want {
    bid: Option<(Price, Qty)>,
    ask: Option<(Price, Qty)>,
    take: Option<(Side, Price, Qty)>,
}

/// Turns desired quotes into cancels and new orders. Keeps an existing order when its price is
/// within `tol_frac` of it and enough of it is left, so time priority is not thrown away every tick.
/// Respects the self-cross rule and the per-tick action limit.
fn requote(obs: &Observation, rules: &Rules, want: &[Want], tol_frac: f64) -> Vec<Action> {
    let n = want.len();
    let mut groups: Vec<Vec<Action>> = Vec::with_capacity(n);
    for (s, w) in want.iter().enumerate() {
        let mut g = Vec::new();
        let mut live_ask: Option<Price> = None;
        let mut live_bid: Option<Price> = None;
        for side in [Side::Sell, Side::Buy] {
            let target = if side == Side::Buy { w.bid } else { w.ask };
            let mine: Vec<&OpenOrder> = obs
                .open_orders
                .iter()
                .filter(|o| o.stock == s && o.side == side)
                .collect();
            let keep = match (target, mine.as_slice()) {
                (Some((p, q)), [o]) => {
                    (o.price - p).abs() as f64 <= p as f64 * tol_frac && o.remaining * 2 >= q
                }
                _ => false,
            };
            if keep {
                let p = mine[0].price;
                if side == Side::Buy {
                    live_bid = Some(p)
                } else {
                    live_ask = Some(p)
                }
                continue;
            }
            for o in &mine {
                g.push(Action::Cancel {
                    order_id: o.order_id,
                });
            }
            if let Some((p, q)) = target {
                if q <= 0 || p <= 0 {
                    continue;
                }
                let ok = match side {
                    Side::Buy => live_ask.is_none_or(|a| p < a),
                    Side::Sell => live_bid.is_none_or(|b| p > b),
                };
                if ok {
                    g.push(Action::limit(s, side, p, q));
                    if side == Side::Buy {
                        live_bid = Some(p)
                    } else {
                        live_ask = Some(p)
                    }
                }
            }
        }
        if let Some((side, p, q)) = w.take {
            let ok = match side {
                Side::Buy => live_ask.is_none_or(|a| p < a),
                Side::Sell => live_bid.is_none_or(|b| p > b),
            };
            if ok && q > 0 {
                g.push(Action::ioc(s, side, p, q));
            }
        }
        groups.push(g);
    }
    // rotate the starting stock so a full action budget does not starve the same stocks
    let start = obs.tick as usize % n.max(1);
    let mut out = Vec::new();
    for k in 0..n {
        let g = &groups[(start + k) % n];
        if out.len() + g.len() > rules.max_actions_per_tick {
            continue;
        }
        out.extend_from_slice(g);
    }
    out
}

fn value(rules: &Rules, obs: &Observation, s: usize, pending: &[(usize, Money, Tick)]) -> f64 {
    hold_value(rules, obs.tick, s, obs.books[s].dividend, pending)
}

// ---------------------------------------------------------------------------

/// Just before an idle charge that trading has not yet covered, a round-trip leg that earns
/// the missing credit: sells where it holds shares that trade at or above value, else buys.
/// Uses the tightest spread and only when crossing it costs less than the credit.
fn credit_topup(
    obs: &Observation,
    rules: &Rules,
    values: &[f64],
    max_cross: f64,
) -> Option<(usize, Side, Price, Qty)> {
    let idle = &obs.account.idle;
    let left = idle.period_end_tick.saturating_sub(obs.tick);
    if idle.due <= 0 || left > 3 || rules.credit_ratio <= 0.0 {
        return None;
    }
    let need = (idle.due as f64 / rules.credit_ratio) as Money;
    let (s, bb, ba) = obs
        .books
        .iter()
        .enumerate()
        .filter_map(|(s, b)| Some((s, b.best_bid()?, b.best_ask()?)))
        .min_by(|x, y| {
            ((x.2 - x.1) as f64 / x.2 as f64).total_cmp(&((y.2 - y.1) as f64 / y.2 as f64))
        })?;
    if (ba - bb) as f64 / ba as f64 > max_cross {
        return None;
    }
    let held = obs.account.sellable[s];
    if held > 0 && (bb + ba) as f64 / 2.0 >= values[s] {
        Some((s, Side::Sell, bb, (need / bb).clamp(1, held)))
    } else {
        let q = (need / ba).min(buy_room(&obs.account, rules, ba, 1.0));
        (q > 0).then_some((s, Side::Buy, ba, q))
    }
}

/// Fundamental trader with a valuation-scaled position: fully invested (equity / stocks in
/// each) at fair value, more with leverage when cheap, nothing at `exit` times value.
/// Sells the gap at the top of the book, so it sells a bubble gradually and at bubble
/// prices instead of all at once at value. Takes liquidity when the book is far from value,
/// and tops up the idle-fee credit before each charge.
/// With `combo = 1` values include the marginal combo bonus.
/// With a large `bid_margin` it is a vulture: deep bids that catch dumps and liquidations.
pub struct Fund {
    rules: Rules,
    know: Knowledge,
    bid_margin: f64,
    ask_margin: f64,
    take_margin: f64,
    leverage: f64,
    exit: f64,
    order_frac: f64,
    combo: bool,
    topup: f64,
}

impl Fund {
    pub fn new(s: &Start) -> Self {
        let m = s.param("margin", 0.01);
        Self {
            rules: s.rules.clone(),
            know: Knowledge::default(),
            bid_margin: s.param("bid_margin", m),
            ask_margin: s.param("ask_margin", m),
            take_margin: s.param("take_margin", 0.05),
            leverage: s.param("leverage", 1.5),
            exit: s.param("exit", 1.6),
            order_frac: s.param("order_frac", 0.02),
            combo: s.param("combo", 0.0) > 0.0,
            topup: s.param("topup", 0.012),
        }
    }
}

impl Bot for Fund {
    fn on_tick(&mut self, obs: &Observation) -> Vec<Action> {
        self.know.update(obs);
        let pending: Vec<_> = self.know.pending().collect();
        let r = &self.rules;
        let n = obs.books.len();
        let t = r.tick_size;
        let eq = obs.account.equity.max(0) as f64;
        let mut want = vec![Want::default(); n];
        let mut values = vec![0.0; n];
        for (s, b) in obs.books.iter().enumerate() {
            let v = value(r, obs, s, &pending);
            values[s] = v;
            let (vb, va) = if self.combo {
                let sh = &obs.account.shares;
                (
                    v + combo_adjust(r, obs.tick, sh, s, true),
                    v + combo_adjust(r, obs.tick, sh, s, false),
                )
            } else {
                (v, v)
            };
            let price = b.mid().unwrap_or(b.mark) as f64;
            // 1 at value, 0 at `exit` x value, up to `leverage` when cheap
            let rel = price / v.max(1.0);
            let weight = ((self.exit - rel) / (self.exit - 1.0)).clamp(0.0, self.leverage);
            let target = (eq / n as f64 * weight / price.max(1.0)) as Qty;
            let have = obs.account.shares[s];
            let lot = ((eq * self.order_frac / price.max(1.0)) as Qty).max(1);

            let mut bid = round_tick(r, vb * (1.0 - self.bid_margin));
            if let Some(bb) = b.best_bid() {
                bid = bid.min(bb + t); // never pay more than one tick over the market
            }
            let mut ask = round_tick(r, va * (1.0 + self.ask_margin));
            if let Some(ba) = b.best_ask() {
                ask = ask.max(ba - t); // the market pays more than value: sell there
            }
            if bid >= ask {
                bid = ask - t;
            }
            let qb = lot
                .min(target - have)
                .min(buy_room(&obs.account, r, bid, self.leverage));
            let qa = lot.min(have - target).min(have);
            want[s].bid = (qb > 0).then_some((bid, qb));
            want[s].ask = (qa > 0).then_some((ask, qa));

            let take_sell = round_tick(r, va * (1.0 + self.take_margin));
            let take_buy = round_tick(r, vb * (1.0 - self.take_margin));
            if b.best_bid().is_some_and(|p| p >= take_sell) && have > target {
                let q = (lot * 2).min(have - target).min(obs.account.sellable[s]);
                want[s].take = Some((Side::Sell, take_sell, q));
            } else if b.best_ask().is_some_and(|p| p <= take_buy) {
                let q = (lot * 2).min(buy_room(&obs.account, r, take_buy, self.leverage));
                want[s].bid = None;
                want[s].take = Some((Side::Buy, take_buy, q));
            }
        }
        if let Some((s, side, p, q)) = credit_topup(obs, r, &values, self.topup) {
            match side {
                Side::Buy => want[s].ask = None,
                Side::Sell => want[s].bid = None,
            }
            want[s].take = Some((side, p, q));
        }
        requote(obs, r, &want, 0.002)
    }
}

// ---------------------------------------------------------------------------

/// Pure liquidity taker on mispricing, sized by the edge, with leverage. Mostly acts on
/// private news (which moves value before the book does). Repays its loan when the edge is gone.
pub struct Sniper {
    rules: Rules,
    know: Knowledge,
    threshold: f64,
    leverage: f64,
    size_frac: f64,
}

impl Sniper {
    pub fn new(s: &Start) -> Self {
        Self {
            rules: s.rules.clone(),
            know: Knowledge::default(),
            threshold: s.param("threshold", 0.03),
            leverage: s.param("leverage", 3.0),
            size_frac: s.param("size_frac", 0.1),
        }
    }
}

impl Bot for Sniper {
    fn on_tick(&mut self, obs: &Observation) -> Vec<Action> {
        self.know.update(obs);
        let pending: Vec<_> = self.know.pending().collect();
        let r = &self.rules;
        let eq = obs.account.equity.max(0) as f64;
        let mut acts = Vec::new();
        let mut repay = obs.account.cash < 0;
        for (s, b) in obs.books.iter().enumerate() {
            let v = value(r, obs, s, &pending);
            if let Some(ask) = b.best_ask() {
                let edge = 1.0 - ask as f64 / v;
                if edge > self.threshold {
                    let px = round_tick(r, v * (1.0 - self.threshold));
                    let want =
                        (eq * self.size_frac * (edge / self.threshold).min(3.0) / px as f64) as Qty;
                    let q = want.min(buy_room(&obs.account, r, px, self.leverage));
                    if q > 0 {
                        acts.push(Action::ioc(s, Side::Buy, px, q));
                        repay = false;
                    }
                    continue;
                }
            }
            if let Some(bid) = b.best_bid() {
                let edge = bid as f64 / v - 1.0;
                let held = obs.account.sellable[s];
                if edge > self.threshold && held > 0 {
                    let px = round_tick(r, v * (1.0 + self.threshold));
                    let want =
                        (eq * self.size_frac * (edge / self.threshold).min(3.0) / px as f64) as Qty;
                    acts.push(Action::ioc(s, Side::Sell, px, want.clamp(1, held)));
                } else if repay && held > 0 && edge > -self.threshold / 3.0 {
                    // fair enough: sell a little to pay down the loan
                    let need = ((-obs.account.cash) as f64 / bid as f64).ceil() as Qty;
                    acts.push(Action::ioc(s, Side::Sell, bid, need.clamp(1, held)));
                    repay = false;
                }
            }
        }
        if acts.is_empty() {
            let values: Vec<f64> = (0..obs.books.len())
                .map(|s| value(r, obs, s, &pending))
                .collect();
            if let Some((s, side, p, q)) = credit_topup(obs, r, &values, 0.012) {
                acts.push(Action::ioc(s, side, p, q));
            }
        }
        acts.truncate(r.max_actions_per_tick);
        acts
    }
}

// ---------------------------------------------------------------------------

/// Market maker that farms the idle-fee credit. Quotes one tick inside the spread, leans its
/// inventory toward cash when the price is above value, and before each idle charge crosses
/// the spread to make up any notional still needed when that is cheaper than the fee.
pub struct Harvester {
    rules: Rules,
    know: Knowledge,
    initial: Vec<Qty>,
    size_frac: f64,
    rich: f64,
    max_cross: f64,
}

impl Harvester {
    pub fn new(s: &Start) -> Self {
        Self {
            rules: s.rules.clone(),
            know: Knowledge::default(),
            initial: s.shares.clone(),
            size_frac: s.param("size_frac", 0.02),
            rich: s.param("rich", 1.05),
            max_cross: s.param("max_cross", 0.012),
        }
    }
}

impl Bot for Harvester {
    fn on_tick(&mut self, obs: &Observation) -> Vec<Action> {
        self.know.update(obs);
        let pending: Vec<_> = self.know.pending().collect();
        let r = &self.rules;
        let t = r.tick_size;
        let n = obs.books.len();
        let eq = obs.account.equity.max(0) as f64;
        let mut want = vec![Want::default(); n];
        let mut values = vec![0.0; n];
        for (s, b) in obs.books.iter().enumerate() {
            let v = value(r, obs, s, &pending);
            values[s] = v;
            let (Some(bb), Some(ba)) = (b.best_bid(), b.best_ask()) else {
                // empty side: quote around value
                let p = round_tick(r, v);
                let q = ((eq * self.size_frac / v) as Qty).max(1);
                if b.best_bid().is_none() && obs.account.shares[s] > 0 {
                    want[s].ask = Some((
                        b.best_ask().map_or(p + t, |a| a.max(p + t)),
                        q.min(obs.account.shares[s]),
                    ));
                }
                if b.best_ask().is_none() {
                    let bid = b.best_bid().map_or(p - t, |x| x.min(p - t));
                    want[s].bid = Some((bid, q.min(buy_room(&obs.account, r, bid, 1.0))));
                }
                continue;
            };
            let mid = (bb + ba) as f64 / 2.0;
            let rich = mid > v * self.rich;
            let target = if rich { 0 } else { self.initial[s] };
            let inv = obs.account.shares[s];
            let q = ((eq * self.size_frac / mid) as Qty).max(1);
            let (mut bid, mut ask) = if ba - bb > 2 * t {
                (bb + t, ba - t)
            } else {
                (bb, ba)
            };
            if inv > target {
                bid = bid.min(round_tick(r, v)) - t; // lean out of the long side
            } else if inv < target {
                ask += t;
            }
            if rich {
                bid = bid.min(round_tick(r, v * self.rich));
            }
            want[s].bid = Some((bid, q.min(buy_room(&obs.account, r, bid, 1.0))));
            want[s].ask = (inv > 0).then_some((ask.max(bid + t), q.min(inv)));
        }

        if let Some((s, side, p, q)) = credit_topup(obs, r, &values, self.max_cross) {
            match side {
                Side::Buy => want[s].ask = None,
                Side::Sell => want[s].bid = None,
            }
            want[s].take = Some((side, p, q));
        }
        requote(obs, r, &want, 0.0)
    }
}

// ---------------------------------------------------------------------------

/// Bubble rider: follows the trend with leverage while the price is below `cap` times value,
/// sells when the trend turns or the price passes the cap, and is out before the end.
pub struct Rider {
    rules: Rules,
    know: Knowledge,
    fast: Vec<f64>,
    slow: Vec<f64>,
    fa: f64,
    sa: f64,
    threshold: f64,
    cap: f64,
    leverage: f64,
    size_frac: f64,
    exit_before: Tick,
}

impl Rider {
    pub fn new(s: &Start) -> Self {
        let p: Vec<f64> = s
            .rules
            .stocks
            .iter()
            .map(|x| x.initial_price as f64)
            .collect();
        Self {
            rules: s.rules.clone(),
            know: Knowledge::default(),
            fast: p.clone(),
            slow: p,
            fa: 2.0 / (jittered(s, "fast", 10.0, 0.3) + 1.0),
            sa: 2.0 / (jittered(s, "slow", 60.0, 0.3) + 1.0),
            threshold: jittered(s, "threshold", 0.003, 0.3),
            cap: jittered(s, "cap", 2.0, 0.3),
            leverage: s.param("leverage", 2.0),
            size_frac: s.param("size_frac", 0.05),
            exit_before: jittered(s, "exit_before", 300.0, 0.3) as Tick,
        }
    }
}

impl Bot for Rider {
    fn on_tick(&mut self, obs: &Observation) -> Vec<Action> {
        self.know.update(obs);
        let pending: Vec<_> = self.know.pending().collect();
        let r = &self.rules;
        for (s, tr) in obs.trades.iter().enumerate() {
            for t in tr {
                let p = t.price as f64;
                self.fast[s] += self.fa * (p - self.fast[s]);
                self.slow[s] += self.sa * (p - self.slow[s]);
            }
        }
        let late = obs.tick + self.exit_before >= obs.ticks_total;
        let eq = obs.account.equity.max(0) as f64;
        let mut acts = Vec::new();
        for (s, b) in obs.books.iter().enumerate() {
            let v = value(r, obs, s, &pending);
            let sig = self.fast[s] / self.slow[s] - 1.0;
            let held = obs.account.sellable[s];
            if !late && sig > self.threshold && (self.fast[s] < v * self.cap) {
                if let Some(ask) = b.best_ask() {
                    let px = round_tick(r, ask as f64 * 1.002);
                    let q = ((eq * self.size_frac / px as f64) as Qty).min(buy_room(
                        &obs.account,
                        r,
                        px,
                        self.leverage,
                    ));
                    if q > 0 {
                        acts.push(Action::ioc(s, Side::Buy, px, q));
                    }
                }
            } else if held > 0 && (late || sig < -self.threshold || self.fast[s] > v * self.cap) {
                if let Some(bid) = b.best_bid() {
                    let px = round_tick(r, bid as f64 * 0.998);
                    let q = ((eq * self.size_frac / px as f64) as Qty).clamp(1, held);
                    acts.push(Action::ioc(s, Side::Sell, px, q));
                }
            }
        }
        acts.truncate(r.max_actions_per_tick);
        acts
    }
}
