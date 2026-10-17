//! Helpers shared by the built-in bots.

use bot_api::*;
use std::collections::HashMap;

/// What this bot knows about one news item.
pub struct Heard {
    /// stock -> (low, high): the intersection of every range received so far.
    pub ranges: HashMap<usize, (Money, Money)>,
    pub effective: Tick,
    pub public: bool,
}

/// Tracks news this bot has heard, and derives dividend expectations. Each later delivery of
/// the same item narrows the range; the intersection of all of them still contains the truth.
#[derive(Default)]
pub struct Knowledge {
    pub heard: HashMap<u32, Heard>,
}

/// Which estimate of a pending change to use.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Estimate {
    Low,
    Mid,
    High,
}

impl Knowledge {
    pub fn update(&mut self, obs: &Observation) {
        for n in &obs.news {
            let h = self.heard.entry(n.id).or_insert_with(|| Heard {
                ranges: HashMap::new(),
                effective: n.effective_tick,
                public: false,
            });
            h.public |= n.public;
            for c in &n.changes {
                let r = h.ranges.entry(c.stock).or_insert((c.low, c.high));
                *r = (r.0.max(c.low), r.1.min(c.high));
                if r.0 > r.1 {
                    *r = (c.low, c.high); // cannot happen with honest ranges; trust the newest
                }
            }
        }
        let t = obs.tick;
        self.heard.retain(|_, v| v.effective > t);
    }

    /// Changes not yet in effect, as (stock, estimated delta, effective tick).
    pub fn pending_with(&self, est: Estimate) -> impl Iterator<Item = (usize, Money, Tick)> + '_ {
        self.heard.values().flat_map(move |h| {
            h.ranges.iter().map(move |(&s, &(lo, hi))| {
                let d = match est {
                    Estimate::Low => lo,
                    Estimate::Mid => (lo + hi) / 2,
                    Estimate::High => hi,
                };
                (s, d, h.effective)
            })
        })
    }

    /// Midpoint estimates of changes not yet in effect.
    pub fn pending(&self) -> impl Iterator<Item = (usize, Money, Tick)> + '_ {
        self.pending_with(Estimate::Mid)
    }
}

/// Number of in-game dividend payments strictly after `tick`.
pub fn payments_after(rules: &Rules, tick: Tick, until: Tick) -> i64 {
    let p = rules.dividend_period;
    let last = (rules.ticks_total.saturating_sub(1) / p) as i64; // payments at p, 2p, ..., < ticks_total
    let end = (until / p) as i64;
    (last.min(end) - (tick / p) as i64).max(0)
}

/// Value of one share held to the end, given the current dividend and known pending changes.
pub fn hold_value(
    rules: &Rules,
    tick: Tick,
    stock: usize,
    dividend: Money,
    pending: &[(usize, Money, Tick)],
) -> f64 {
    let total = rules.ticks_total;
    let mut d = dividend as f64;
    let mut changes: Vec<(Tick, Money)> = pending
        .iter()
        .filter(|p| p.0 == stock)
        .map(|p| (p.2, p.1))
        .collect();
    changes.sort();
    let mut v = 0.0;
    let mut from = tick;
    for (eff, delta) in changes {
        if eff >= total {
            break;
        }
        // payments in (from, eff) use the old dividend; a payment exactly at eff already uses the new one
        v += d * payments_after(rules, from, eff.saturating_sub(1)) as f64;
        from = eff.saturating_sub(1).max(from);
        d += delta as f64;
    }
    v += d * payments_after(rules, from, total) as f64;
    v + d * rules.terminal_payments as f64
}

pub fn mid_or_last(b: &BookView) -> Price {
    b.mid().unwrap_or(b.last)
}

pub fn round_tick(rules: &Rules, p: f64) -> Price {
    let t = rules.tick_size;
    ((p / t as f64).round() as Price * t).max(t)
}

/// Shares affordable at `price` with this much buying power.
pub fn affordable(rules: &Rules, power: Money, price: Price) -> Qty {
    if price <= 0 {
        return 0;
    }
    power / rules.buy_reserve_per_share(price)
}

/// Quantity this account can buy in `stock` at `price` while staying under `target_leverage`.
pub fn buy_room(acct: &AccountView, rules: &Rules, price: Price, target_leverage: f64) -> Qty {
    let cap = (target_leverage * acct.equity as f64) as Money - acct.gross - acct.reserved_buy;
    affordable(rules, cap.min(acct.buying_power), price).max(0)
}

/// Like `Start::param`, but an unset parameter gets its default scaled by a per-account
/// factor in [1 - spread, 1 + spread], so copies of one bot do not all act on the same tick.
pub fn jittered(start: &Start, key: &str, default: f64, spread: f64) -> f64 {
    if let Some(v) = start.params.get(key) {
        return *v;
    }
    let mut h = start.seed ^ 0x9e37_79b9_7f4a_7c15;
    for b in key.bytes() {
        h = (h ^ b as u64).wrapping_mul(0x100_0000_01b3);
    }
    h ^= h >> 29;
    let u = (h % 1_000_000) as f64 / 1_000_000.0;
    default * (1.0 + spread * (2.0 * u - 1.0))
}
