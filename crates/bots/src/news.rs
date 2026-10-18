//! Event lifecycle repair. The legacy News stays in frozen benchmark crowds.
use bot_api::*;
use std::collections::BTreeMap;

#[derive(Clone, Copy)]
pub(crate) struct Event {
    pub low: Money,
    pub high: Money,
    pub public_tick: Tick,
}

#[derive(Default)]
pub(crate) struct Events(pub BTreeMap<(u32, usize), Event>);

impl Events {
    pub fn update(&mut self, obs: &Observation, tail: Tick) {
        for n in &obs.news {
            for c in &n.changes {
                let e = self.0.entry((n.id, c.stock)).or_insert(Event {
                    low: c.low,
                    high: c.high,
                    public_tick: n.public_tick,
                });
                e.low = e.low.max(c.low);
                e.high = e.high.min(c.high);
                if e.low > e.high {
                    (e.low, e.high) = (c.low, c.high);
                }
                e.public_tick = n.public_tick;
            }
        }
        self.0
            .retain(|_, e| obs.tick < e.public_tick.saturating_add(tail));
    }

    /// Uses only received ranges. Public direction is exact but loses weight over time.
    pub fn impulse(&self, stock: usize, tick: Tick, dividend: Money) -> f64 {
        self.0
            .iter()
            .filter(|((_, s), _)| *s == stock)
            .map(|(_, e)| {
                let mid = (e.low as f64 + e.high as f64) * 0.5;
                let width = (e.high as f64 - e.low as f64).max(0.0) * 0.5;
                let confidence = mid.abs() / (mid.abs() + width).max(1.0);
                let age_weight = if tick < e.public_tick {
                    1.0
                } else {
                    0.5 * (-((tick - e.public_tick) as f64) / 30.0).exp()
                };
                (mid / (dividend.max(1) as f64 * 0.1)).clamp(-1.0, 1.0) * confidence * age_weight
            })
            .sum::<f64>()
            .clamp(-1.0, 1.0)
    }
}

pub struct NewsFixed {
    rules: Rules,
    anchor: Vec<Qty>,
    events: Events,
    lot: Qty,
    min_change: f64,
}

impl NewsFixed {
    pub fn new(s: &Start) -> Self {
        Self {
            rules: s.rules.clone(),
            anchor: s.shares.clone(),
            events: Events::default(),
            lot: crate::common::jittered(s, "lot", 30.0, 0.5).max(1.0) as Qty,
            min_change: crate::common::jittered(s, "min_change", 0.03, 0.6).max(0.0),
        }
    }
}

impl Bot for NewsFixed {
    fn on_tick(&mut self, obs: &Observation) -> Vec<Action> {
        self.events.update(obs, 0);
        let mut targets = self.anchor.clone();
        for ((_, s), e) in &self.events.0 {
            let mid = (e.low as f64 + e.high as f64) * 0.5;
            let rel = mid / obs.books[*s].dividend.max(1) as f64;
            if rel.abs() >= self.min_change {
                targets[*s] += if rel > 0.0 {
                    3 * self.lot
                } else {
                    -3 * self.lot
                };
            }
        }
        let mut budget = ((obs.account.equity.max(0) as f64 * 2.0_f64.min(self.rules.max_leverage)
            - obs.account.gross as f64
            - obs.account.reserved_buy as f64)
            .max(0.0) as Money)
            .min(obs.account.buying_power.max(0));
        let mut actions = Vec::new();
        // IOC net target is reconciled to actual shares on every observation.
        // No-fill, rejected and partial orders never count as acquired inventory.
        for k in 0..targets.len() {
            if actions.len() >= self.rules.max_actions_per_tick {
                break;
            }
            let s = (k + obs.tick as usize) % targets.len();
            let b = &obs.books[s];
            let target = targets[s].max(0);
            let have = obs.account.shares[s];
            if have > target {
                if let Some(bid) = b.best_bid() {
                    let q = (have - target).min(obs.account.sellable[s]);
                    if q > 0 {
                        actions.push(Action::ioc(s, Side::Sell, bid, q));
                    }
                }
            } else if have < target {
                if let Some(ask) = b.best_ask() {
                    let cost = self.rules.buy_reserve_per_share(ask).max(1);
                    let q = (target - have).min(self.lot).min(budget / cost);
                    if q > 0 {
                        actions.push(Action::ioc(s, Side::Buy, ask, q));
                        budget -= q * cost;
                    }
                }
            }
        }
        actions
    }
}

#[cfg(test)]
#[path = "news_tests.rs"]
mod tests;
