//! Reference strategies. They are controls, not good strategies.

use crate::common::*;
use bot_api::*;
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;

/// Random small limit orders near the current price.
pub struct Noise {
    rng: ChaCha8Rng,
    rules: Rules,
    prob: f64,
    max_qty: Qty,
    spread: f64,
    ttl: Tick,
}

impl Noise {
    pub fn new(s: &Start) -> Self {
        Self {
            rng: ChaCha8Rng::seed_from_u64(s.seed),
            rules: s.rules.clone(),
            prob: s.param("prob", 0.3),
            max_qty: s.param("max_qty", 20.0) as Qty,
            spread: s.param("spread", 0.01),
            ttl: s.param("ttl", 30.0) as Tick,
        }
    }
}

impl Bot for Noise {
    fn on_tick(&mut self, obs: &Observation) -> Vec<Action> {
        let mut acts: Vec<Action> = obs
            .open_orders
            .iter()
            .filter(|o| obs.tick - o.placed_tick > self.ttl)
            .map(|o| Action::Cancel {
                order_id: o.order_id,
            })
            .collect();
        for (s, b) in obs.books.iter().enumerate() {
            if self.rng.gen::<f64>() > self.prob {
                continue;
            }
            let reference = mid_or_last(b) as f64;
            let price = round_tick(
                &self.rules,
                reference * (1.0 + self.rng.gen_range(-1.0..1.0) * self.spread),
            );
            let qty = self.rng.gen_range(1..=self.max_qty);
            let own_buy = obs
                .open_orders
                .iter()
                .filter(|o| o.stock == s && o.side == Side::Buy)
                .map(|o| o.price)
                .max();
            let own_sell = obs
                .open_orders
                .iter()
                .filter(|o| o.stock == s && o.side == Side::Sell)
                .map(|o| o.price)
                .min();
            if self.rng.gen_bool(0.5) {
                let q = qty.min(buy_room(&obs.account, &self.rules, price, 1.0));
                if q > 0 && own_sell.is_none_or(|p| price < p) {
                    acts.push(Action::limit(s, Side::Buy, price, q));
                }
            } else {
                let q = qty.min(obs.account.sellable[s]);
                if q > 0 && own_buy.is_none_or(|p| price > p) {
                    acts.push(Action::limit(s, Side::Sell, price, q));
                }
            }
        }
        acts
    }
}

/// Two-sided quotes around a smoothed price, skewed against inventory.
pub struct Maker {
    rules: Rules,
    spread: f64,
    size: Qty,
    refresh: Tick,
    skew: f64,
    reference: Vec<f64>,
    base: Vec<Qty>,
}

impl Maker {
    pub fn new(s: &Start) -> Self {
        Self {
            rules: s.rules.clone(),
            spread: s.param("spread", 0.006),
            size: s.param("size", 15.0) as Qty,
            refresh: s.param("refresh", 5.0) as Tick,
            skew: s.param("skew", 0.015),
            reference: s
                .rules
                .stocks
                .iter()
                .map(|x| x.initial_price as f64)
                .collect(),
            base: s.shares.iter().map(|&q| q.max(1)).collect(),
        }
    }
}

impl Bot for Maker {
    fn on_tick(&mut self, obs: &Observation) -> Vec<Action> {
        for (s, tr) in obs.trades.iter().enumerate() {
            for t in tr {
                self.reference[s] = 0.9 * self.reference[s] + 0.1 * t.price as f64;
            }
        }
        if !obs.tick.is_multiple_of(self.refresh) {
            return vec![];
        }
        let mut acts = vec![Action::CancelAll { stock: None }];
        let n = obs.books.len() as f64;
        for (s, b) in obs.books.iter().enumerate() {
            let mut r = self.reference[s];
            if let Some(m) = b.mid() {
                r = 0.5 * r + 0.5 * m as f64;
            }
            let inv = ((obs.account.shares[s] - self.base[s]) as f64 / self.base[s] as f64)
                .clamp(-1.0, 1.0);
            let mid = r * (1.0 - inv * self.skew);
            let bid = round_tick(&self.rules, mid * (1.0 - self.spread / 2.0));
            let ask = round_tick(&self.rules, mid * (1.0 + self.spread / 2.0))
                .max(bid + self.rules.tick_size);
            let power_share = (obs.account.buying_power as f64 / n) as Money;
            let qb = self.size.min(affordable(
                &self.rules,
                power_share.min(obs.account.cash.max(0) / n as i64),
                bid,
            ));
            let qa = self.size.min(obs.account.shares[s]);
            if qb > 0 {
                acts.push(Action::limit(s, Side::Buy, bid, qb));
            }
            if qa > 0 {
                acts.push(Action::limit(s, Side::Sell, ask, qa));
            }
        }
        acts
    }
}

/// Values each share as dividends to the end plus the terminal value (with news it has heard),
/// buys below value and sells above it.
pub struct Value {
    rules: Rules,
    know: Knowledge,
    margin: f64,
    max_leverage: f64,
    lot: Qty,
}

impl Value {
    pub fn new(s: &Start) -> Self {
        Self {
            rules: s.rules.clone(),
            know: Knowledge::default(),
            margin: s.param("margin", 0.03),
            max_leverage: s.param("max_leverage", 1.0),
            lot: s.param("lot", 20.0) as Qty,
        }
    }
}

impl Bot for Value {
    fn on_tick(&mut self, obs: &Observation) -> Vec<Action> {
        self.know.update(obs);
        let pending: Vec<_> = self.know.pending().collect();
        let mut acts = vec![Action::CancelAll { stock: None }];
        for (s, b) in obs.books.iter().enumerate() {
            let v = hold_value(&self.rules, obs.tick, s, b.dividend, &pending);
            let bid = round_tick(&self.rules, v * (1.0 - self.margin));
            let ask = round_tick(&self.rules, v * (1.0 + self.margin));
            let qb = self.lot.min(
                buy_room(&obs.account, &self.rules, bid, self.max_leverage)
                    / obs.books.len() as Qty,
            );
            if qb > 0 {
                acts.push(Action::limit(s, Side::Buy, bid, qb));
            }
            let qa = self.lot.min(obs.account.shares[s]);
            if qa > 0 && ask > bid {
                acts.push(Action::limit(s, Side::Sell, ask, qa));
            }
        }
        acts
    }
}

/// Fast/slow moving-average trend follower.
pub struct Momentum {
    rules: Rules,
    fast: Vec<f64>,
    slow: Vec<f64>,
    fa: f64,
    sa: f64,
    threshold: f64,
    lot: Qty,
    every: Tick,
    max_leverage: f64,
}

impl Momentum {
    pub fn new(s: &Start) -> Self {
        let p: Vec<f64> = s
            .rules
            .stocks
            .iter()
            .map(|x| x.initial_price as f64)
            .collect();
        Self {
            rules: s.rules.clone(),
            fast: p.clone(),
            slow: p,
            fa: 2.0 / (s.param("fast", 20.0) + 1.0),
            sa: 2.0 / (s.param("slow", 120.0) + 1.0),
            threshold: s.param("threshold", 0.004),
            lot: s.param("lot", 15.0) as Qty,
            every: s.param("every", 5.0) as Tick,
            max_leverage: s.param("max_leverage", 1.5),
        }
    }
}

impl Bot for Momentum {
    fn on_tick(&mut self, obs: &Observation) -> Vec<Action> {
        for (s, tr) in obs.trades.iter().enumerate() {
            for t in tr {
                let p = t.price as f64;
                self.fast[s] += self.fa * (p - self.fast[s]);
                self.slow[s] += self.sa * (p - self.slow[s]);
            }
        }
        if !obs.tick.is_multiple_of(self.every) || obs.tick < 100 {
            return vec![];
        }
        let mut acts = Vec::new();
        for (s, b) in obs.books.iter().enumerate() {
            let sig = self.fast[s] / self.slow[s] - 1.0;
            if sig > self.threshold {
                if let Some(ask) = b.best_ask() {
                    let px = round_tick(&self.rules, ask as f64 * 1.002);
                    let q =
                        self.lot
                            .min(buy_room(&obs.account, &self.rules, px, self.max_leverage));
                    if q > 0 {
                        acts.push(Action::ioc(s, Side::Buy, px, q));
                    }
                }
            } else if sig < -self.threshold {
                if let Some(bid) = b.best_bid() {
                    let q = self.lot.min(obs.account.sellable[s]);
                    if q > 0 {
                        acts.push(Action::ioc(
                            s,
                            Side::Sell,
                            round_tick(&self.rules, bid as f64 * 0.998),
                            q,
                        ));
                    }
                }
            }
        }
        acts
    }
}

/// Trades in the direction of news it hears privately, takes profit once it is public.
pub struct News {
    rules: Rules,
    lot: Qty,
    min_change: f64,
    plans: Vec<(u32, usize, i32)>, // news id, stock, remaining lots (+buy / -sell)
    seen: std::collections::HashSet<u32>,
}

impl News {
    pub fn new(s: &Start) -> Self {
        Self {
            rules: s.rules.clone(),
            lot: s.param("lot", 30.0) as Qty,
            min_change: s.param("min_change", 0.03),
            plans: Vec::new(),
            seen: Default::default(),
        }
    }
}

impl Bot for News {
    fn on_tick(&mut self, obs: &Observation) -> Vec<Action> {
        for n in &obs.news {
            for &(s, d) in &n.changes {
                let rel = d as f64 / obs.books[s].dividend.max(1) as f64;
                if !n.public && rel.abs() >= self.min_change && self.seen.insert(n.id) {
                    self.plans.push((n.id, s, if rel > 0.0 { 3 } else { -3 }));
                } else if n.public {
                    for p in self.plans.iter_mut().filter(|p| p.0 == n.id && p.1 == s) {
                        p.2 = if p.2 >= 0 { -1 } else { 1 };
                    }
                }
            }
        }
        let mut acts = Vec::new();
        for p in self.plans.iter_mut() {
            let b = &obs.books[p.1];
            if p.2 > 0 {
                if let Some(ask) = b.best_ask() {
                    let px = round_tick(&self.rules, ask as f64 * 1.003);
                    let q = self.lot.min(buy_room(&obs.account, &self.rules, px, 2.0));
                    if q > 0 {
                        acts.push(Action::ioc(p.1, Side::Buy, px, q));
                    }
                }
                p.2 -= 1;
            } else if p.2 < 0 {
                if let Some(bid) = b.best_bid() {
                    let q = self.lot.min(obs.account.sellable[p.1]);
                    if q > 0 {
                        acts.push(Action::ioc(
                            p.1,
                            Side::Sell,
                            round_tick(&self.rules, bid as f64 * 0.997),
                            q,
                        ));
                    }
                }
                p.2 += 1;
            }
        }
        self.plans.retain(|p| p.2 != 0);
        acts
    }
}

/// Buys once and holds.
pub struct Hold {
    rules: Rules,
    every: Tick,
}

impl Hold {
    pub fn new(s: &Start) -> Self {
        Self {
            rules: s.rules.clone(),
            every: s.param("every", 50.0) as Tick,
        }
    }
}

impl Bot for Hold {
    fn on_tick(&mut self, obs: &Observation) -> Vec<Action> {
        if obs.tick < 10 || !obs.tick.is_multiple_of(self.every) {
            return vec![];
        }
        let n = obs.books.len() as i64;
        let mut acts = vec![Action::CancelAll { stock: None }];
        let budget = obs.account.cash.max(0) / n;
        for (s, b) in obs.books.iter().enumerate() {
            if let Some(ask) = b.best_ask() {
                let px = round_tick(&self.rules, ask as f64 * 1.01);
                let q = affordable(&self.rules, budget, px);
                if q > 0 {
                    acts.push(Action::limit(s, Side::Buy, px, q));
                }
            }
        }
        acts
    }
}
