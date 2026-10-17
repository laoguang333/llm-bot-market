//! Reference strategies. They are controls, not good strategies.

use crate::common::*;
use bot_api::*;
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;

/// Random small limit orders near the current price, which each account blends with its own
/// value estimate by a different amount.
pub struct Noise {
    rng: ChaCha8Rng,
    rules: Rules,
    know: Knowledge,
    value_weight: f64,
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
            know: Knowledge::default(),
            value_weight: jittered(s, "value_weight", 0.5, 0.8),
            prob: jittered(s, "prob", 0.3, 0.6),
            max_qty: jittered(s, "max_qty", 20.0, 0.6) as Qty,
            spread: jittered(s, "spread", 0.01, 0.7),
            ttl: jittered(s, "ttl", 30.0, 0.6) as Tick,
        }
    }
}

impl Bot for Noise {
    fn on_tick(&mut self, obs: &Observation) -> Vec<Action> {
        self.know.update(obs);
        let pending: Vec<_> = self.know.pending().collect();
        let terminal = self.know.terminal(&self.rules, Estimate::Mid);
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
            let v = hold_value(&self.rules, obs.tick, s, b.dividend, &pending, terminal);
            let reference =
                (1.0 - self.value_weight) * mid_or_last(b) as f64 + self.value_weight * v;
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

/// Grid trader: a ladder of price levels a fixed percentage apart around an anchor. It keeps a
/// buy on the nearest levels below the last price and a sell on the nearest levels above, so it
/// buys on the way down and sells one level up. It does not follow the price; only when the price
/// has stayed outside the ladder for a while does it lay a new ladder around the current price.
pub struct Grid {
    rng: ChaCha8Rng,
    rules: Rules,
    know: Knowledge,
    step: f64,
    levels: i32,
    lot: Qty,
    active: usize,
    patience: Tick,
    reset: Tick,
    value_weight: f64,
    anchor_noise: f64,
    last_reset: Vec<Tick>,
    anchor: Vec<f64>,
    outside_since: Vec<Option<Tick>>,
}

impl Grid {
    pub fn new(s: &Start) -> Self {
        let mut rng = ChaCha8Rng::seed_from_u64(s.seed ^ 0x9e37);
        let n = s.rules.stocks.len();
        let value_weight = jittered(s, "value_weight", 0.8, 0.25).clamp(0.0, 1.0);
        let anchor_noise = jittered(s, "anchor_noise", 0.06, 0.5);
        let terminal = Knowledge::default().terminal(&s.rules, Estimate::Mid);
        let anchor = (0..n)
            .map(|k| {
                let v = hold_value(
                    &s.rules,
                    0,
                    k,
                    s.rules.stocks[k].initial_dividend,
                    &[],
                    terminal,
                );
                let p = s.rules.stocks[k].initial_price as f64;
                (value_weight * v + (1.0 - value_weight) * p)
                    * (1.0 + rng.gen_range(-1.0..1.0) * anchor_noise)
            })
            .collect();
        Self {
            rng,
            rules: s.rules.clone(),
            know: Knowledge::default(),
            step: jittered(s, "step", 0.006, 0.5),
            levels: jittered(s, "levels", 8.0, 0.4).round().max(2.0) as i32,
            lot: jittered(s, "lot", 15.0, 0.5).max(1.0) as Qty,
            active: (s.rules.max_open_orders / s.rules.stocks.len().max(1) / 2).max(1),
            patience: jittered(s, "patience", 150.0, 0.6) as Tick,
            reset: jittered(s, "reset", 250.0, 0.6) as Tick,
            value_weight,
            anchor_noise,
            last_reset: vec![0; n],
            anchor,
            outside_since: vec![None; n],
        }
    }

    fn level(&self, s: usize, k: i32) -> Price {
        round_tick(&self.rules, self.anchor[s] * (1.0 + self.step).powi(k))
    }
}

impl Bot for Grid {
    fn on_tick(&mut self, obs: &Observation) -> Vec<Action> {
        self.know.update(obs);
        let pending: Vec<_> = self.know.pending().collect();
        let terminal = self.know.terminal(&self.rules, Estimate::Mid);
        let mut want: Vec<(usize, Side, Price, Qty)> = vec![];
        for (s, b) in obs.books.iter().enumerate() {
            let p = mid_or_last(b);
            if p <= 0 {
                continue;
            }
            let (lo, hi) = (self.level(s, -self.levels), self.level(s, self.levels));
            if p < lo || p > hi {
                self.outside_since[s].get_or_insert(obs.tick);
            } else {
                self.outside_since[s] = None;
            }
            let stale = self.outside_since[s].is_some_and(|t| obs.tick - t > self.patience);
            // every so often, or once the price has left the ladder for a while, the owner re-lays
            // it around a blend of what they think the stock is worth and where it trades
            if stale || obs.tick - self.last_reset[s] > self.reset {
                let v = hold_value(&self.rules, obs.tick, s, b.dividend, &pending, terminal);
                let jitter = 1.0 + self.rng.gen_range(-1.0..1.0) * self.anchor_noise;
                self.anchor[s] =
                    (self.value_weight * v + (1.0 - self.value_weight) * p as f64) * jitter;
                self.last_reset[s] = obs.tick;
                self.outside_since[s] = None;
            }
            let mut held = obs.account.shares[s];
            // a ladder holds one lot for each level above the price; after the ladder moves down,
            // the shares it no longer wants are sold near the current price
            let target = self.lot
                * (-self.levels..=self.levels)
                    .filter(|&k| self.level(s, k) > p)
                    .count() as Qty;
            if held > target {
                let n = (held - target).min(2 * self.lot);
                let price = b.bids.first().map_or(p, |l| l.price.max(p - p * 5 / 1000));
                want.push((s, Side::Sell, price, n));
                held -= n;
            }
            let mut sells = 0;
            let mut buys = 0;
            // nearest levels above the price: sells, while there are shares to sell
            for k in -self.levels..=self.levels {
                let q = self.level(s, k);
                if q > p && sells < self.active && held > 0 {
                    let n = self.lot.min(held);
                    want.push((s, Side::Sell, q, n));
                    held -= n;
                    sells += 1;
                }
            }
            for k in (-self.levels..=self.levels).rev() {
                let q = self.level(s, k);
                if q < p && buys < self.active {
                    want.push((s, Side::Buy, q, self.lot));
                    buys += 1;
                }
            }
        }
        // keep resting orders that are still wanted, cancel the rest, place what is missing
        let mut acts = vec![];
        let mut budget = self.rules.max_actions_per_tick;
        for o in &obs.open_orders {
            if let Some(i) = want
                .iter()
                .position(|w| w.0 == o.stock && w.1 == o.side && w.2 == o.price)
            {
                want.swap_remove(i);
            } else if budget > 0 {
                acts.push(Action::Cancel {
                    order_id: o.order_id,
                });
                budget -= 1;
            }
        }
        let mut room =
            (obs.account.buying_power.min(obs.account.cash.max(0)) as f64 * 0.9) as Money;
        for (s, side, price, qty) in want {
            if budget == 0 {
                break;
            }
            let qty = if side == Side::Buy {
                let q = qty.min(affordable(&self.rules, room, price));
                room -= q * self.rules.buy_reserve_per_share(price);
                q
            } else {
                qty
            };
            if qty > 0 {
                acts.push(Action::limit(s, side, price, qty));
                budget -= 1;
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
            margin: jittered(s, "margin", 0.03, 0.7),
            max_leverage: jittered(s, "max_leverage", 1.0, 0.5),
            lot: jittered(s, "lot", 20.0, 0.5) as Qty,
        }
    }
}

impl Bot for Value {
    fn on_tick(&mut self, obs: &Observation) -> Vec<Action> {
        self.know.update(obs);
        let pending: Vec<_> = self.know.pending().collect();
        let mut acts = vec![Action::CancelAll { stock: None }];
        for (s, b) in obs.books.iter().enumerate() {
            let v = hold_value(
                &self.rules,
                obs.tick,
                s,
                b.dividend,
                &pending,
                self.know.terminal(&self.rules, Estimate::Mid),
            );
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

/// Fast/slow moving-average trend follower. Stops buying above `cap` times value and takes
/// profit above `exit` times value, so it cannot pump a stock forever on its own buying.
pub struct Momentum {
    rules: Rules,
    know: Knowledge,
    cap: f64,
    exit: f64,
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
            know: Knowledge::default(),
            cap: jittered(s, "cap", 1.05, 0.1),
            exit: jittered(s, "exit", 1.4, 0.15),
            fast: p.clone(),
            slow: p,
            fa: 2.0 / (jittered(s, "fast", 20.0, 0.6) + 1.0),
            sa: 2.0 / (jittered(s, "slow", 120.0, 0.5) + 1.0),
            threshold: jittered(s, "threshold", 0.004, 0.6),
            lot: jittered(s, "lot", 15.0, 0.5) as Qty,
            every: jittered(s, "every", 5.0, 0.6) as Tick,
            max_leverage: jittered(s, "max_leverage", 1.5, 0.3),
        }
    }
}

impl Bot for Momentum {
    fn on_tick(&mut self, obs: &Observation) -> Vec<Action> {
        self.know.update(obs);
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
        let pending: Vec<_> = self.know.pending().collect();
        let mut acts = Vec::new();
        for (s, b) in obs.books.iter().enumerate() {
            let v = hold_value(
                &self.rules,
                obs.tick,
                s,
                b.dividend,
                &pending,
                self.know.terminal(&self.rules, Estimate::Mid),
            );
            let sig = self.fast[s] / self.slow[s] - 1.0;
            let price = mid_or_last(b) as f64;
            if sig > self.threshold && price < v * self.cap {
                if let Some(ask) = b.best_ask() {
                    let px = round_tick(&self.rules, ask as f64 * 1.002);
                    let q =
                        self.lot
                            .min(buy_room(&obs.account, &self.rules, px, self.max_leverage));
                    if q > 0 {
                        acts.push(Action::ioc(s, Side::Buy, px, q));
                    }
                }
            } else if sig < -self.threshold || price > v * self.exit {
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
            lot: jittered(s, "lot", 30.0, 0.5) as Qty,
            min_change: jittered(s, "min_change", 0.03, 0.6),
            plans: Vec::new(),
            seen: Default::default(),
        }
    }
}

impl Bot for News {
    fn on_tick(&mut self, obs: &Observation) -> Vec<Action> {
        for n in &obs.news {
            for c in &n.changes {
                let s = c.stock;
                let rel = c.mid() / obs.books[s].dividend.max(1) as f64;
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

/// Long-term holder: spends its cash on stocks priced near value, holds them for the
/// dividends, and keeps an ask well above value for a quarter of each holding.
pub struct Hold {
    rules: Rules,
    know: Knowledge,
    every: Tick,
    buy_below: f64,
    sell_above: f64,
}

impl Hold {
    pub fn new(s: &Start) -> Self {
        Self {
            rules: s.rules.clone(),
            know: Knowledge::default(),
            every: jittered(s, "every", 50.0, 0.5) as Tick,
            buy_below: jittered(s, "buy_below", 0.95, 0.1),
            sell_above: jittered(s, "sell_above", 1.3, 0.15),
        }
    }
}

impl Bot for Hold {
    fn on_tick(&mut self, obs: &Observation) -> Vec<Action> {
        self.know.update(obs);
        if obs.tick < 10 || !obs.tick.is_multiple_of(self.every) {
            return vec![];
        }
        let pending: Vec<_> = self.know.pending().collect();
        let n = obs.books.len() as i64;
        let mut acts = vec![Action::CancelAll { stock: None }];
        let budget = obs.account.cash.max(0) / n;
        for (s, b) in obs.books.iter().enumerate() {
            let v = hold_value(
                &self.rules,
                obs.tick,
                s,
                b.dividend,
                &pending,
                self.know.terminal(&self.rules, Estimate::Mid),
            );
            let cap = round_tick(&self.rules, v * self.buy_below);
            if let Some(ask) = b.best_ask() {
                let px = round_tick(&self.rules, ask as f64 * 1.01).min(cap);
                let q = affordable(&self.rules, budget, px);
                if q > 0 {
                    acts.push(Action::limit(s, Side::Buy, px, q));
                }
            }
            // a standing ask well above value for part of the holding
            let floor =
                round_tick(&self.rules, v * self.sell_above).max(cap + self.rules.tick_size);
            {
                let q = (obs.account.shares[s] / 4)
                    .max(1)
                    .min(obs.account.shares[s]);
                if q > 0 {
                    acts.push(Action::limit(s, Side::Sell, floor, q));
                }
            }
        }
        acts
    }
}
