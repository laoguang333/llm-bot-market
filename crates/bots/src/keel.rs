//! Keel: a rebalanced portfolio. It keeps a target share of equity in every stock and
//! trades back to those shares when prices drift apart, so it sells what has risen
//! relative to the rest and buys what has fallen. Three slow inputs set the targets:
//! - how much equity sits in stocks follows a fixed curve of the growth the average
//!   business still has ahead of it at this point of the game, never today's prices,
//!   so a crash does not change it;
//! - each stock's share leans toward stocks trading below their own long-run average
//!   price (long-horizon reversal);
//! - after mid-game, a logistic score of the public dividend path tilts toward the stock
//!   most likely to be the designated grower.
//!
//! In the last stretch it keeps only shares worth holding to the end at today's price.
//! It never rests orders: every trade is an IOC against the book, so its quotes cannot
//! be picked off. Before each idle-fee settlement it trades a same-tick round trip on
//! the tightest books when its turnover has not earned the credit yet.

use crate::keel_growth::Growth;
use bot_api::{Action, Bot, Observation, Rules, Side, Start, Tick};
use std::collections::HashSet;

/// Price memory is sampled every this many ticks.
const SAMPLE: Tick = 50;

/// Expected log ratio of a stock's remaining cash flows to "today's projected dividend
/// forever", by fraction of the game played. Measured on 3000 generated worlds (seeds
/// 100000+): every business starts at zero extra dividend and grows into its first
/// phase, so the early market is worth more than its current dividends.
const GROWTH: &[(f64, f64)] = &[
    (0.0, 0.36),
    (0.025, 0.337),
    (0.075, 0.235),
    (0.125, 0.176),
    (0.175, 0.151),
    (0.225, 0.128),
    (0.275, 0.111),
    (0.325, 0.091),
    (0.375, 0.07),
    (0.425, 0.064),
    (0.6, 0.055),
    (0.8, 0.045),
    (0.875, 0.025),
    (0.925, 0.015),
    (1.0, 0.0),
];

fn growth(x: f64) -> f64 {
    let x = x.clamp(0.0, 1.0);
    for w in GROWTH.windows(2) {
        let ((x0, y0), (x1, y1)) = (w[0], w[1]);
        if x <= x1 {
            return y0 + (y1 - y0) * (x - x0) / (x1 - x0);
        }
    }
    0.0
}

pub struct Keel {
    rules: Rules,
    n: usize,
    /// Public, not yet effective dividend changes: (effective tick, delta).
    pending: Vec<Vec<(Tick, i64)>>,
    seen: HashSet<u32>,
    hint: (i64, i64),
    // parameters
    base: f64,
    slope: f64,
    max_exposure: f64,
    min_exposure: f64,
    horizon: f64,
    late: Tick,
    late_margin: f64,
    growth: Growth,
    lean: f64,
    memory: f64,
    cap: f64,
    price_log: Vec<Vec<f64>>,
    next_price_sample: Tick,
    tilt: f64,
    tilt_floor: f64,
    tilt_start: Tick,
    band: f64,
    rebalance: Tick,
    slip: f64,
    max_step: f64,
    wash_window: Tick,
    wash_spread: f64,
    /// Fast average of each stock's log mid, to see a price still in a fast move.
    ema: Vec<f64>,
    ema_ticks: f64,
    guard: f64,
}

impl Keel {
    pub fn new(start: &Start) -> Self {
        let r = &start.rules;
        Keel {
            n: r.stocks.len(),
            pending: vec![Vec::new(); r.stocks.len()],
            seen: HashSet::new(),
            hint: (r.terminal_payments_min, r.terminal_payments_max),
            base: start.param("base", 1.5),
            slope: start.param("slope", 6.0),
            max_exposure: start.param("max_exposure", 2.5),
            // Floor on stock exposure while some stock is eligible.
            min_exposure: start.param("min_exposure", 0.0),
            horizon: start.param("horizon", 1000.0),
            late: start.param("late", 1000.0) as Tick,
            late_margin: start.param("late_margin", 0.0),
            growth: Growth::new(r),
            lean: start.param("lean", 1.5),
            memory: start.param("memory", 5000.0),
            cap: start.param("cap", 0.3),
            price_log: Vec::new(),
            next_price_sample: 0,
            tilt: start.param("tilt", 1.0),
            tilt_floor: start.param("tilt_floor", 0.4),
            tilt_start: start.param("tilt_start", 5000.0) as Tick,
            band: start.param("band", 0.12),
            rebalance: start.param("rebalance", 500.0) as Tick,
            slip: start.param("slip", 0.01),
            max_step: start.param("max_step", 0.1),
            wash_window: start.param("wash_window", 20.0) as Tick,
            wash_spread: start.param("wash_spread", 0.012),
            ema: Vec::new(),
            ema_ticks: start.param("ema_ticks", 50.0),
            guard: start.param("guard", 0.03),
            rules: r.clone(),
        }
    }

    fn terminal_units(&self) -> f64 {
        (self.hint.0 + self.hint.1) as f64 / 2.0
    }

    fn projected(&self, obs: &Observation, s: usize) -> i64 {
        obs.books[s].dividend + self.pending[s].iter().map(|(_, d)| d).sum::<i64>()
    }

    fn read_news(&mut self, obs: &Observation) {
        for p in &mut self.pending {
            p.retain(|(t, _)| *t > obs.tick);
        }
        for item in &obs.news {
            if !item.public || item.forecast || !self.seen.insert(item.id) {
                continue;
            }
            for c in &item.changes {
                if c.stock < self.n {
                    let before = self.projected(obs, c.stock);
                    self.growth
                        .record(item.layer, item.public_tick, c.stock, before, c.low);
                    self.pending[c.stock].push((item.effective_tick, c.low));
                }
            }
        }
        let projected: Vec<i64> = (0..self.n).map(|s| self.projected(obs, s)).collect();
        self.growth.update(obs.tick, &projected);
        for &(lo, hi) in &obs.terminal_hints {
            self.hint = (
                self.hint.0.max(lo),
                self.hint.1.min(hi).max(self.hint.0.max(lo)),
            );
        }
    }

    /// Price used for valuation and sizing: mid when both sides exist.
    fn price(obs: &Observation, s: usize) -> f64 {
        let b = &obs.books[s];
        match (b.best_bid(), b.best_ask()) {
            (Some(x), Some(y)) => (x + y) as f64 / 2.0,
            (Some(x), None) => x as f64,
            (None, Some(y)) => y as f64,
            _ => b.mark.max(b.last).max(1) as f64,
        }
    }

    /// Value of holding one share to the end, in cents, before expected growth.
    fn flat_value(&self, obs: &Observation, s: usize) -> f64 {
        let rate = self.projected(obs, s);
        let remaining = self.rules.last_tick().saturating_sub(obs.tick) as f64
            / self.rules.dividend_period.max(1) as f64;
        let terminal = self.rules.terminal_dividend_payments(self.terminal_units());
        self.rules.dividend_cents(rate.max(0)) * (remaining + terminal)
    }

    /// Log of value over price per stock, with no growth: the loss or gain of holding the
    /// share to the end at today's price.
    fn hold_edge(&self, obs: &Observation, prices: &[f64]) -> Vec<f64> {
        (0..self.n)
            .map(|s| (self.flat_value(obs, s).max(1.0) / prices[s]).ln())
            .collect()
    }

    /// Stock exposure: a base level plus the expected growth the model says the market
    /// will still collect over the next `horizon` ticks. Prices are not an input, so a
    /// crash never raises the exposure.
    fn exposure(&self, obs: &Observation) -> f64 {
        let total = self.rules.ticks_total.max(1) as f64;
        let x = obs.tick as f64 / total;
        let ahead = growth(x) - growth(x + self.horizon / total);
        (self.base + self.slope * ahead).clamp(0.0, self.max_exposure)
    }

    /// Remember log prices every `SAMPLE` ticks, keeping `memory` ticks of them.
    fn sample_prices(&mut self, tick: Tick, prices: &[f64]) {
        if tick < self.next_price_sample {
            return;
        }
        self.next_price_sample = (tick / SAMPLE + 1) * SAMPLE;
        self.price_log
            .push(prices.iter().map(|p| p.max(1.0).ln()).collect());
        let keep = (self.memory / SAMPLE as f64).ceil().max(1.0) as usize;
        if self.price_log.len() > keep {
            self.price_log.remove(0);
        }
    }

    /// Equal weights, leaned toward stocks trading below their own long-run average
    /// price: weight ~ (price / average)^-lean, no stock above `cap` of the budget.
    fn reversal_weights(&self, prices: &[f64]) -> Vec<f64> {
        let k = self.price_log.len().max(1) as f64;
        let mut w: Vec<f64> = (0..self.n)
            .map(|s| {
                let avg = self.price_log.iter().map(|row| row[s]).sum::<f64>() / k;
                let gap = if self.price_log.is_empty() {
                    0.0
                } else {
                    prices[s].max(1.0).ln() - avg
                };
                (-self.lean * gap).exp()
            })
            .collect();
        for _ in 0..5 {
            let total: f64 = w.iter().sum();
            for x in w.iter_mut() {
                *x = (*x / total).min(self.cap);
            }
        }
        let total: f64 = w.iter().sum();
        w.iter().map(|x| x / total).collect()
    }

    /// Fraction of the stock budget for each stock: reversal weights, tilted toward the
    /// likely grower after `tilt_start`; in the last `late` ticks, equal among the shares
    /// worth holding to the end at today's price.
    fn weights(&self, obs: &Observation, edge: &[f64], prices: &[f64]) -> Vec<f64> {
        let left = self.rules.last_tick().saturating_sub(obs.tick);
        let mut w = if left <= self.late {
            let ok: Vec<bool> = edge.iter().map(|e| *e > self.late_margin).collect();
            let k = ok.iter().filter(|o| **o).count();
            ok.iter()
                .map(|o| if *o { 1.0 / k as f64 } else { 0.0 })
                .collect()
        } else {
            self.reversal_weights(prices)
        };
        if left > self.late && obs.tick >= self.tilt_start && self.tilt > 0.0 {
            let tilt: Vec<f64> = self
                .growth
                .prob()
                .iter()
                .map(|p| {
                    if *p > self.tilt_floor {
                        (self.tilt * p).min(0.6)
                    } else {
                        0.0
                    }
                })
                .collect();
            let total: f64 = tilt.iter().sum();
            let scale = if total > 0.9 { 0.9 / total } else { 1.0 };
            let keep = 1.0 - total * scale;
            for (wi, ti) in w.iter_mut().zip(&tilt) {
                *wi = *wi * keep + ti * scale;
            }
        }
        w
    }

    /// Shares available on the far side within `slip` of the best price.
    fn depth(obs: &Observation, s: usize, side: Side, limit: i64) -> i64 {
        let b = &obs.books[s];
        let levels = if side == Side::Buy { &b.asks } else { &b.bids };
        levels
            .iter()
            .take_while(|l| match side {
                Side::Buy => l.price <= limit,
                Side::Sell => l.price >= limit,
            })
            .map(|l| l.qty)
            .sum()
    }

    /// Near the end of an idle-fee period, if traded notional has not yet earned the
    /// credit, round-trip shares on the tightest books within the same tick.
    fn wash(&self, obs: &Observation, acts: &mut Vec<Action>, traded: &[bool], mut power: f64) {
        let idle = &obs.account.idle;
        if idle.period_end_tick.saturating_sub(obs.tick) > self.wash_window {
            return;
        }
        let need = idle.fee - idle.credit;
        if need <= 0 {
            return;
        }
        // Notional needed, both legs counted.
        let mut notional = (need as f64 / self.rules.credit_ratio.max(1e-9)) as i64;
        let mut order: Vec<usize> = (0..self.n).filter(|&s| !traded[s]).collect();
        let spread = |s: usize| -> f64 {
            let b = &obs.books[s];
            match (b.best_bid(), b.best_ask()) {
                (Some(x), Some(y)) => (y - x) as f64 / x.max(1) as f64,
                _ => f64::INFINITY,
            }
        };
        order.sort_by(|a, b| spread(*a).total_cmp(&spread(*b)));
        for s in order {
            if notional <= 0 || acts.len() + 2 > self.rules.max_actions_per_tick {
                break;
            }
            if spread(s) > self.wash_spread {
                break;
            }
            let b = &obs.books[s];
            let (bid, ask) = (b.best_bid().unwrap(), b.best_ask().unwrap());
            let lo = ((bid as f64) * (1.0 - self.slip)) as i64;
            let hi = ((ask as f64) * (1.0 + self.slip)).ceil() as i64;
            let tick = self.rules.tick_size.max(1);
            let (lo, hi) = (lo / tick * tick, (hi + tick - 1) / tick * tick);
            let held = obs.account.sellable[s];
            let reserve = self.rules.buy_reserve_per_share(hi) as f64;
            let qty = (notional / 2 / bid.max(1))
                .min(Self::depth(obs, s, Side::Sell, lo))
                .min(Self::depth(obs, s, Side::Buy, hi));
            // Sell first when the shares are there; otherwise buy first and sell them back.
            let qty = if held > 0 {
                qty.min(held)
            } else {
                qty.min((power / reserve) as i64)
            };
            if qty <= 0 {
                continue;
            }
            if held > 0 {
                acts.push(Action::ioc(s, Side::Sell, lo, qty));
                acts.push(Action::ioc(s, Side::Buy, hi, qty));
            } else {
                power -= qty as f64 * reserve;
                acts.push(Action::ioc(s, Side::Buy, hi, qty));
                acts.push(Action::ioc(s, Side::Sell, lo, qty));
            }
            notional -= qty * (bid + ask);
        }
    }
}

impl Bot for Keel {
    fn on_tick(&mut self, obs: &Observation) -> Vec<Action> {
        self.read_news(obs);
        let mut acts = Vec::new();
        if obs.tick == 0 {
            return acts;
        }
        let prices: Vec<f64> = (0..self.n).map(|s| Self::price(obs, s)).collect();
        let logs: Vec<f64> = prices.iter().map(|p| p.max(1.0).ln()).collect();
        if self.ema.is_empty() {
            self.ema = logs.clone();
        }
        let alpha = 1.0 / self.ema_ticks.max(1.0);
        for (e, l) in self.ema.iter_mut().zip(&logs) {
            *e += (l - *e) * alpha;
        }
        let equity = obs.account.equity.max(1) as f64;
        let edge = self.hold_edge(obs, &prices);
        self.sample_prices(obs.tick, &prices);
        let weights = self.weights(obs, &edge, &prices);
        let eligible = weights.iter().any(|w| *w > 0.0);
        let mut exposure = self.exposure(obs);
        if eligible {
            exposure = exposure.max(self.min_exposure);
        }
        let periodic = self.rebalance > 0 && obs.tick.is_multiple_of(self.rebalance);
        let tick = self.rules.tick_size.max(1);
        let mut power = obs.account.buying_power as f64;
        let mut traded = vec![false; self.n];
        for s in 0..self.n {
            let target = exposure * equity * weights[s];
            let current = obs.account.shares[s] as f64 * prices[s];
            let gap = target - current;
            let trigger = if periodic { 0.01 } else { self.band };
            // A price far from its fast average is still moving: wait for it to settle.
            let moving = self.guard > 0.0 && (logs[s] - self.ema[s]).abs() > self.guard;
            if gap.abs() < trigger * equity || moving {
                continue;
            }
            let value = gap.abs().min(self.max_step * equity);
            let b = &obs.books[s];
            if gap > 0.0 {
                let Some(ask) = b.best_ask() else { continue };
                let limit =
                    (((ask as f64) * (1.0 + self.slip)).ceil() as i64 + tick - 1) / tick * tick;
                let reserve = self.rules.buy_reserve_per_share(limit) as f64;
                let qty = ((value.min(power) / reserve) as i64).min(Self::depth(
                    obs,
                    s,
                    Side::Buy,
                    limit,
                ));
                if qty > 0 {
                    power -= qty as f64 * reserve;
                    acts.push(Action::ioc(s, Side::Buy, limit, qty));
                    traded[s] = true;
                }
            } else {
                let Some(bid) = b.best_bid() else { continue };
                let limit = ((bid as f64) * (1.0 - self.slip)) as i64 / tick * tick;
                let qty = ((value / bid as f64) as i64)
                    .min(obs.account.sellable[s])
                    .min(Self::depth(obs, s, Side::Sell, limit.max(tick)));
                if qty > 0 {
                    acts.push(Action::ioc(s, Side::Sell, limit.max(tick), qty));
                    traded[s] = true;
                }
            }
        }
        self.wash(obs, &mut acts, &traded, power);
        acts.truncate(self.rules.max_actions_per_tick);
        acts
    }
}
