//! Lattice learns a pooled short-horizon return model within each match.
//!
//! Its signal uses observed prices, carry and received changes in future cash
//! flow. It learns their coefficients instead of prescribing a news-response
//! curve. No order flow, hidden world data, or other bots is read. Labels mature
//! before entering the exponentially weighted ridge regression.

use bot_api::*;
use std::collections::{BTreeMap, VecDeque};

const D: usize = 14;
const CADENCE: Tick = 5;
const HORIZON: Tick = 60;

#[derive(Clone)]
struct Frame {
    tick: Tick,
    prices: Vec<f64>,
    rates: Vec<f64>,
    cashflows: Vec<f64>,
    features: Vec<[f64; D]>,
}

fn mature_labels(
    pending: &mut VecDeque<Frame>,
    tick: Tick,
    prices: &[f64],
    cashflows: &[f64],
) -> Vec<([f64; D], f64)> {
    let mut samples = Vec::new();
    while pending
        .front()
        .is_some_and(|f| tick.saturating_sub(f.tick) >= HORIZON)
    {
        let old = pending.pop_front().unwrap();
        for s in 0..prices.len() {
            let y = ((prices[s] + cashflows[s] - old.cashflows[s]) / old.prices[s])
                .max(0.01)
                .ln()
                .clamp(-0.20, 0.20);
            samples.push((old.features[s], y));
        }
    }
    samples
}

struct Learner {
    gram: [[f64; D]; D],
    rhs: [f64; D],
    weights: [f64; D],
    labels: usize,
}

impl Learner {
    fn new() -> Self {
        Self {
            gram: [[0.0; D]; D],
            rhs: [0.0; D],
            weights: [0.0; D],
            labels: 0,
        }
    }

    fn update(&mut self, samples: &[([f64; D], f64)], decay: f64) {
        for i in 0..D {
            self.rhs[i] *= decay;
            for j in 0..D {
                self.gram[i][j] *= decay;
            }
        }
        for (x, y) in samples {
            for i in 0..D {
                self.rhs[i] += x[i] * y;
                for j in 0..=i {
                    self.gram[i][j] += x[i] * x[j];
                    self.gram[j][i] = self.gram[i][j];
                }
            }
        }
        self.labels += samples.len();
        // Fixed ridge penalty remains present even after old observations fade.
        let mut lower = [[0.0; D]; D];
        for i in 0..D {
            for j in 0..=i {
                let mut a = self.gram[i][j] + if i == j { 20.0 } else { 0.0 };
                for k in 0..j {
                    a -= lower[i][k] * lower[j][k];
                }
                lower[i][j] = if i == j {
                    a.max(1e-9).sqrt()
                } else {
                    a / lower[j][j]
                };
            }
        }
        let mut z = [0.0; D];
        for i in 0..D {
            let mut v = self.rhs[i];
            for (k, zk) in z.iter().enumerate().take(i) {
                v -= lower[i][k] * zk;
            }
            z[i] = v / lower[i][i];
        }
        for i in (0..D).rev() {
            let mut v = z[i];
            for (k, row) in lower.iter().enumerate().skip(i + 1) {
                v -= row[i] * self.weights[k];
            }
            self.weights[i] = v / lower[i][i];
        }
    }

    fn predict(&self, x: &[f64; D]) -> f64 {
        x.iter()
            .zip(self.weights)
            .map(|(a, b)| a * b)
            .sum::<f64>()
            .clamp(-0.15, 0.15)
    }
}

pub struct Lattice {
    rules: Rules,
    history: VecDeque<Frame>,
    pending: VecDeque<Frame>,
    cashflows: Vec<f64>,
    previous_dividends: Vec<Money>,
    previous_tick: Tick,
    next_decision: Tick,
    learner: Learner,
    terminal_low: i64,
    terminal_high: i64,
    news: BTreeMap<u32, NewsItem>,
    gain: f64,
    leverage: f64,
    learn: bool,
    fee_management: bool,
    news_features: bool,
    price_features: bool,
    base: f64,
    rebalance: Tick,
    targets: Vec<f64>,
    refresh_at: Vec<Tick>,
    trace: bool,
    cost_buffer: f64,
    learn_until: Tick,
}

impl Lattice {
    pub fn new(start: &Start) -> Self {
        let rules = start.rules.clone();
        Self {
            targets: vec![0.08; rules.stocks.len()],
            refresh_at: (0..rules.stocks.len())
                .map(|s| s as Tick * CADENCE)
                .collect(),
            cashflows: vec![0.0; rules.stocks.len()],
            previous_dividends: rules.stocks.iter().map(|s| s.initial_dividend).collect(),
            previous_tick: 0,
            next_decision: 0,
            terminal_low: rules.terminal_payments_min,
            terminal_high: rules.terminal_payments_max,
            rules,
            history: VecDeque::new(),
            pending: VecDeque::new(),
            learner: Learner::new(),
            news: BTreeMap::new(),
            gain: start.param("gain", 12.0).clamp(0.0, 30.0),
            leverage: start.param("leverage", 2.5).clamp(0.0, 3.0),
            learn: start.param("learn", 1.0) > 0.0,
            fee_management: start.param("fee_management", 1.0) > 0.0,
            news_features: start.param("news_features", 1.0) > 0.0,
            price_features: start.param("price_features", 1.0) > 0.0,
            base: start.param("base", 0.03).clamp(0.0, 0.2),
            rebalance: start.param("rebalance", CADENCE as f64).clamp(5.0, 180.0) as Tick,
            trace: start.param("trace", 0.0) > 0.0,
            cost_buffer: start.param("cost_buffer", 1.5).clamp(0.0, 5.0),
            learn_until: start
                .param("learn_until", start.rules.ticks_total as f64)
                .max(0.0) as Tick,
        }
    }

    fn price(book: &BookView) -> f64 {
        match (book.best_bid(), book.best_ask()) {
            (Some(b), Some(a)) if b > 0 && a >= b && a < b * 11 / 10 => (a as f64 + b as f64) * 0.5,
            _ => book.mark.max(1) as f64,
        }
    }

    fn past(&self, tick: Tick, lag: Tick) -> Option<&Frame> {
        let t = tick.saturating_sub(lag);
        self.history.iter().rev().find(|f| f.tick <= t)
    }

    fn returns(&self, obs: &Observation, prices: &[f64], lag: Tick) -> Vec<f64> {
        if let Some(old) = self.past(obs.tick, lag) {
            prices
                .iter()
                .enumerate()
                .map(|(s, p)| {
                    ((p + self.cashflows[s] - old.cashflows[s]) / old.prices[s])
                        .max(0.01)
                        .ln()
                        .clamp(-0.5, 0.5)
                })
                .collect()
        } else {
            vec![0.0; prices.len()]
        }
    }

    fn features(&self, obs: &Observation, prices: &[f64]) -> Vec<[f64; D]> {
        let short = self.returns(obs, prices, 10);
        let medium = self.returns(obs, prices, 40);
        let long = self.returns(obs, prices, 160);
        let slow = self.returns(obs, prices, 640);
        let market = medium.iter().sum::<f64>() / prices.len().max(1) as f64;
        let duration = self.rules.last_tick().saturating_sub(obs.tick) as f64
            / self.rules.dividend_period as f64
            + self
                .rules
                .terminal_dividend_payments((self.terminal_low + self.terminal_high) as f64 * 0.5);
        let mut pending = vec![0.0; prices.len()];
        if self.news_features {
            for event in self.news.values() {
                if event.effective_tick <= self.rules.last_tick() {
                    for change in &event.changes {
                        let delta = if change.low > 0 {
                            change.low as f64
                        } else if change.high < 0 {
                            change.high as f64
                        } else {
                            0.0
                        };
                        pending[change.stock] += delta / self.rules.dividend_scale as f64;
                    }
                }
            }
        }
        (0..prices.len())
            .map(|s| {
                let sector = self.rules.stocks[s].sector;
                let peers: Vec<usize> = (0..prices.len())
                    .filter(|&j| j != s && self.rules.stocks[j].sector == sector)
                    .collect();
                let sector_return = if peers.is_empty() {
                    market
                } else {
                    peers.iter().map(|&j| medium[j]).sum::<f64>() / peers.len() as f64
                };
                let carry = self.rules.dividend_cents(obs.books[s].dividend) * HORIZON as f64
                    / self.rules.dividend_period as f64
                    / prices[s];
                let cashflow_change = if self.news_features {
                    self.past(obs.tick, HORIZON)
                        .map(|f| self.rules.dividend_cents(obs.books[s].dividend) - f.rates[s])
                        .unwrap_or(0.0)
                } else {
                    0.0
                };
                let spread = match (obs.books[s].best_bid(), obs.books[s].best_ask()) {
                    (Some(b), Some(a)) => (a - b).max(0) as f64 / prices[s],
                    _ => 0.05,
                };
                let mut x = [
                    1.0,
                    short[s] / 0.02,
                    medium[s] / 0.04,
                    long[s] / 0.08,
                    slow[s] / 0.16,
                    market / 0.04,
                    (sector_return - medium[s]) / 0.04,
                    carry / 0.01,
                    short[s].abs() / 0.02,
                    (prices[s] / obs.books[s].mark.max(1) as f64).ln() / 0.02,
                    pending[s] * duration / prices[s] / 0.10,
                    cashflow_change * duration / prices[s] / 0.10,
                    (pending[s] * duration / prices[s] / 0.10).abs(),
                    spread / 0.01,
                ];
                for v in &mut x {
                    *v = v.clamp(-4.0, 4.0);
                }
                if !self.price_features {
                    x[1..7].fill(0.0);
                    x[8..10].fill(0.0);
                }
                x
            })
            .collect()
    }

    fn ingest(&mut self, obs: &Observation) {
        let dt = obs.tick.saturating_sub(self.previous_tick);
        for s in 0..self.cashflows.len() {
            // Observations normally arrive each tick. For a skipped interval use
            // only the previous observed rate; never backfill unseen news.
            let payments = obs.tick / self.rules.dividend_period
                - self.previous_tick / self.rules.dividend_period;
            self.cashflows[s] +=
                self.rules.dividend_cents(self.previous_dividends[s]) * payments as f64;
            if dt > 0 && obs.tick.is_multiple_of(self.rules.dividend_period) {
                self.cashflows[s] += self
                    .rules
                    .dividend_cents(obs.books[s].dividend - self.previous_dividends[s]);
            }
            self.previous_dividends[s] = obs.books[s].dividend;
        }
        self.previous_tick = obs.tick;
        for &(lo, hi) in &obs.terminal_hints {
            self.terminal_low = self.terminal_low.max(lo);
            self.terminal_high = self.terminal_high.min(hi);
        }
        for item in &obs.news {
            if item.effective_tick > obs.tick {
                self.news.insert(item.id, item.clone());
            }
        }
        self.news.retain(|_, n| n.effective_tick > obs.tick);
    }

    fn settlement_targets(&self, obs: &Observation, prices: &[f64], weights: &mut [f64]) {
        let left = self.rules.last_tick().saturating_sub(obs.tick);
        let window = (self.rules.ticks_total / 12).max(HORIZON);
        if left > window {
            return;
        }
        let n = (self.terminal_low + self.terminal_high) as f64 / 2.0;
        let terminal = self.rules.terminal_dividend_payments(n);
        for s in 0..weights.len() {
            let mut rate = obs.books[s].dividend as f64;
            let mut cash = rate * (left / self.rules.dividend_period) as f64;
            for event in self.news.values() {
                if event.effective_tick > self.rules.last_tick() {
                    continue;
                }
                for c in &event.changes {
                    if c.stock == s {
                        rate += c.mid();
                        cash += c.mid()
                            * ((self.rules.last_tick() - event.effective_tick + 1)
                                / self.rules.dividend_period) as f64;
                    }
                }
            }
            let value = (cash + rate.max(0.0) * terminal) / self.rules.dividend_scale as f64;
            let edge = value / prices[s] - 1.0;
            let settlement_weight = (edge * 2.0).clamp(0.0, 0.4);
            let fraction = 1.0 - left as f64 / window as f64;
            weights[s] = weights[s] * (1.0 - fraction) + settlement_weight * fraction;
            if left < HORIZON && edge < -0.02 {
                weights[s] = 0.0;
            }
        }
    }

    fn turnover(
        &self,
        obs: &Observation,
        prices: &[f64],
        weights: &[f64],
        actions: &mut Vec<Action>,
        busy: &[bool],
        buy_budget: Money,
    ) {
        if !self.fee_management
            || self.rules.credit_ratio <= 0.0
            || !matches!(self.rules.credit_side.as_str(), "both" | "taker")
            || obs.account.idle.due <= 0
            || obs.account.idle.period_end_tick.saturating_sub(obs.tick) > 8
        {
            return;
        }
        let needed = obs.account.idle.due as f64 / self.rules.credit_ratio;
        let mut candidates: Vec<(usize, f64)> = obs
            .books
            .iter()
            .enumerate()
            .filter_map(|(s, b)| {
                let bid = b.bids.first()?;
                let ask = b.asks.first()?;
                let cost = (ask.price - bid.price) as f64 / prices[s]
                    + 2.0 * self.rules.trade_fee_bps as f64 / 10_000.0;
                // Two legs earn two credits. Require a material saving.
                (!busy[s] && weights[s] > 0.02 && cost < self.rules.credit_ratio)
                    .then_some((s, cost))
            })
            .collect();
        candidates.sort_by(|a, b| a.1.total_cmp(&b.1));
        if let Some(&(s, _)) = candidates.first() {
            let bid = obs.books[s].bids[0];
            let ask = obs.books[s].asks[0];
            let q = (needed / (bid.price + ask.price) as f64).ceil() as Qty;
            // Buyback is affordable even if the sell fails. Both legs are
            // independent IOC orders: never assume atomic round trips.
            let q = q
                .min(obs.account.sellable[s])
                .min(bid.qty)
                .min(ask.qty)
                .min(buy_budget / self.rules.buy_reserve_per_share(ask.price));
            if q > 0 && actions.len() + 2 <= self.rules.max_actions_per_tick {
                actions.push(Action::ioc(s, Side::Sell, bid.price, q));
                actions.push(Action::ioc(s, Side::Buy, ask.price, q));
            }
        }
    }
}

impl Bot for Lattice {
    fn on_tick(&mut self, obs: &Observation) -> Vec<Action> {
        self.ingest(obs);
        if obs.tick < self.next_decision || obs.books.is_empty() {
            return Vec::new();
        }
        self.next_decision = obs.tick.saturating_add(CADENCE);
        let prices: Vec<f64> = obs.books.iter().map(Self::price).collect();
        let samples = mature_labels(&mut self.pending, obs.tick, &prices, &self.cashflows);
        if self.learn && obs.tick <= self.learn_until && !samples.is_empty() {
            self.learner
                .update(&samples, (-(CADENCE as f64) / 1600.0).exp());
        }
        let features = self.features(obs, &prices);
        let frame = Frame {
            tick: obs.tick,
            prices: prices.clone(),
            rates: obs
                .books
                .iter()
                .map(|b| self.rules.dividend_cents(b.dividend))
                .collect(),
            cashflows: self.cashflows.clone(),
            features: features.clone(),
        };
        self.history.push_back(frame.clone());
        self.pending.push_back(frame);
        while self
            .history
            .front()
            .is_some_and(|f| obs.tick.saturating_sub(f.tick) > 650)
        {
            self.history.pop_front();
        }
        let equity = obs.account.equity.max(0) as f64;
        if equity < 1.0 {
            return Vec::new();
        }
        let financing = self.rules.loan_rate * HORIZON as f64 / self.rules.interest_period as f64;
        for (s, x) in features.iter().enumerate() {
            if obs.tick >= self.refresh_at[s] {
                self.refresh_at[s] = obs.tick + self.rebalance;
                let forecast = self.learner.predict(x);
                self.targets[s] = (self.base + self.gain * (forecast - financing)).clamp(0.0, 0.5);
            }
        }
        let mut weights = self.targets.clone();
        if self.trace && obs.tick.is_multiple_of(1000) {
            eprintln!(
                "lattice tick={} equity={} beta={:?} targets={:?}",
                obs.tick, equity, self.learner.weights, weights
            );
        }
        self.settlement_targets(obs, &prices, &mut weights);
        let sum: f64 = weights.iter().sum();
        let limit = self.leverage.min(self.rules.max_leverage * 0.75);
        if sum > limit {
            for w in &mut weights {
                *w *= limit / sum;
            }
        }
        let mut actions = Vec::new();
        let mut busy = vec![false; prices.len()];
        let mut buy_budget = obs.account.buying_power;
        // Sells first; buy budget nevertheless does not depend on their success.
        for side in [Side::Sell, Side::Buy] {
            for s in 0..prices.len() {
                if actions.len() >= self.rules.max_actions_per_tick.saturating_sub(2) {
                    break;
                }
                let mark = obs.books[s].mark.max(1) as f64;
                let spread = match (obs.books[s].best_bid(), obs.books[s].best_ask()) {
                    (Some(b), Some(a)) => (a - b).max(0) as f64 / prices[s],
                    _ => 0.05,
                };
                let crossing_cost = spread * 0.5 + self.rules.trade_fee_bps as f64 / 10_000.0;
                let deadband = if self.rules.last_tick().saturating_sub(obs.tick) < HORIZON
                    && weights[s] == 0.0
                {
                    0.0
                } else {
                    self.gain * self.cost_buffer * crossing_cost
                };
                let adjusted = match side {
                    Side::Buy => (weights[s] - deadband).max(0.0),
                    Side::Sell => weights[s] + deadband,
                };
                let target = (equity * adjusted / mark) as Qty;
                let held = obs.account.shares[s];
                let difference = target - held;
                if (side == Side::Buy && difference <= 0)
                    || (side == Side::Sell && difference >= 0)
                    || difference.abs() as f64 * prices[s] < equity * 0.012
                {
                    continue;
                }
                let level = match side {
                    Side::Buy => obs.books[s].asks.first(),
                    Side::Sell => obs.books[s].bids.first(),
                };
                let Some(level) = level else { continue };
                // A fixed limit bounds slippage; no market orders or stale quotes.
                let price = level.price;
                let mut qty = difference
                    .abs()
                    .min((equity * 0.08 / price.max(1) as f64).ceil() as Qty)
                    .min(level.qty);
                if side == Side::Buy {
                    let cost = self.rules.buy_reserve_per_share(price);
                    qty = qty.min(buy_budget / cost);
                    buy_budget -= qty * cost;
                } else {
                    qty = qty.min(obs.account.sellable[s]);
                }
                if qty > 0 {
                    actions.push(Action::ioc(s, side, price, qty));
                    busy[s] = true;
                }
            }
        }
        // A fee-management buyback must share the same remaining buy budget.
        self.turnover(obs, &prices, &weights, &mut actions, &busy, buy_budget);
        actions
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn training_labels_are_unavailable_until_the_full_horizon() {
        let frame = Frame {
            tick: 20,
            prices: vec![100.0],
            rates: vec![1.0],
            cashflows: vec![7.0],
            features: vec![[1.0; D]],
        };
        let mut pending = VecDeque::from([frame]);
        assert!(mature_labels(&mut pending, 79, &[110.0], &[9.0]).is_empty());
        assert_eq!(pending.len(), 1);
        let labels = mature_labels(&mut pending, 80, &[110.0], &[9.0]);
        assert_eq!(labels.len(), 1);
        assert!((labels[0].1 - 1.12_f64.ln()).abs() < 1e-12);
        assert!(pending.is_empty());
    }

    #[test]
    fn learns_sign_reversal_without_changing_features() {
        let mut model = Learner::new();
        let mut positive = [0.0; D];
        positive[1] = 1.0;
        let mut negative = positive;
        negative[1] = -1.0;
        for _ in 0..100 {
            model.update(&[(positive, 0.03), (negative, -0.03)], 0.97);
        }
        assert!(model.predict(&positive) > 0.02);
        for _ in 0..160 {
            model.update(&[(positive, -0.03), (negative, 0.03)], 0.97);
        }
        assert!(model.predict(&positive) < -0.02);
    }

    #[test]
    fn collinear_history_stays_finite() {
        let mut model = Learner::new();
        for _ in 0..1000 {
            model.update(&[([1.0; D], 0.02)], 0.995);
        }
        assert!(model.weights.iter().all(|w| w.is_finite()));
        assert!((model.predict(&[1.0; D]) - 0.02).abs() < 0.001);
    }
}
