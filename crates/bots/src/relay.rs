//! Inventory-circulating, cash-only market maker. Inventory and quote width
//! respond to observed volatility and adverse dividend news; no wash roundtrips.
use crate::common::{hold_value, round_tick, Estimate, Knowledge};
use crate::ecology_execution::{combo_value, execute, negative_pair_blocked, Quote};
use bot_api::*;

pub struct Relay {
    rules: Rules,
    knowledge: Knowledge,
    last: Vec<f64>,
    variance: Vec<f64>,
    next: Tick,
}

impl Relay {
    pub fn new(start: &Start) -> Self {
        let n = start.rules.stocks.len();
        Self {
            rules: start.rules.clone(),
            knowledge: Knowledge::default(),
            last: vec![0.0; n],
            variance: vec![0.0; n],
            next: 0,
        }
    }
}

impl Bot for Relay {
    fn on_tick(&mut self, obs: &Observation) -> Vec<Action> {
        self.knowledge.update(obs);
        if obs.tick < self.next {
            return vec![];
        }
        self.next = obs.tick + 5;
        let r = &self.rules;
        let eq = obs.account.equity.max(0) as f64;
        let terminal = self.knowledge.terminal(r, Estimate::Mid);
        let pending: Vec<_> = self.knowledge.pending().collect();
        let mut quotes = vec![Quote::default(); obs.books.len()];
        for (s, b) in obs.books.iter().enumerate() {
            let mid = b.mid().unwrap_or(b.last).max(r.tick_size) as f64;
            let change = if self.last[s] > 0.0 {
                (mid / self.last[s]).ln().clamp(-1.0, 1.0)
            } else {
                0.0
            };
            self.variance[s] = 0.9 * self.variance[s] + 0.1 * change * change;
            self.last[s] = mid;
            let width = (0.006 + 2.0 * self.variance[s].sqrt()).min(0.12);
            let value = hold_value(r, obs.tick, s, b.dividend, &pending, terminal).max(0.0);
            let have = obs.account.shares[s];
            let lot = (eq * 0.009 / mid).max(1.0) as Qty;
            let max_qty = (eq * 0.10 / mid) as Qty;
            let inventory = (have as f64 * mid / eq.max(1.0) / 0.055).min(3.0);
            let center = mid * (1.0 - 0.012 * (inventory - 1.0));
            let buy_value = (value + combo_value(r, obs, s, lot)).max(0.0);
            if have < max_qty && !negative_pair_blocked(r, obs, s) {
                let cap = (buy_value * 0.88).min(center * (1.0 - width));
                if cap >= r.tick_size as f64 {
                    let p =
                        round_tick(r, cap).min(b.best_ask().map_or(i64::MAX, |a| a - r.tick_size));
                    quotes[s].bid = Some((p, lot.min(max_qty - have)));
                }
            }
            if have > 0 {
                let q = lot.min(have);
                let contract = combo_value(r, obs, s, -q);
                // Contract-backed inventory is offered at its own marginal value;
                // excess ordinary inventory may be sold below our holding estimate.
                let floor = (0.55 * value + contract).max(r.tick_size as f64);
                let p = round_tick(r, (center * (1.0 + width)).max(floor))
                    .max(b.best_bid().map_or(r.tick_size, |b| b + r.tick_size))
                    .max(quotes[s].bid.map_or(0, |x| x.0) + r.tick_size);
                quotes[s].ask = Some((p, q));
            }
        }
        execute(obs, r, &quotes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::news_test_support::{observation, start};
    #[test]
    fn inventory_is_offered_even_without_ordinary_dividends() {
        let s = start();
        let mut obs = observation(100, true);
        for b in &mut obs.books {
            b.dividend = 0;
        }
        let actions = Relay::new(&s).on_tick(&obs);
        assert!(actions.iter().any(|a| matches!(
            a,
            Action::Limit {
                side: Side::Sell,
                ..
            }
        )));
        assert!(!actions.iter().any(|a| matches!(
            a,
            Action::Limit {
                side: Side::Buy,
                ..
            }
        )));
    }
}
