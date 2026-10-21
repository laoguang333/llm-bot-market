//! Risk-aware cash-flow investor. Observed dividend history is evidence, not a
//! guarantee of recovery. No company state, world RNG or future path is accessed.
use crate::common::{payments_after, round_tick, Estimate, Knowledge};
use crate::ecology_execution::{combo_value, execute, negative_pair_blocked, Quote};
use bot_api::*;

pub struct Harbor {
    rules: Rules,
    knowledge: Knowledge,
    peak: Vec<f64>,
    next: Tick,
}

impl Harbor {
    pub fn new(start: &Start) -> Self {
        Self {
            rules: start.rules.clone(),
            knowledge: Knowledge::default(),
            peak: start
                .rules
                .stocks
                .iter()
                .map(|s| s.initial_dividend as f64)
                .collect(),
            next: 0,
        }
    }

    /// A coarse public-distribution expectation, deliberately not a reconstruction
    /// of the hidden generator. Damage is inferred from the observed peak ratio.
    fn estimate(&self, obs: &Observation, stock: usize, terminal: f64) -> f64 {
        let r = &self.rules;
        let current = obs.books[stock].dividend.max(0) as f64;
        let anchor = self.peak[stock].max(current).max(1.0);
        let pending: Vec<_> = self
            .knowledge
            .pending()
            .filter(|(s, _, t)| *s == stock && *t < r.ticks_total)
            .collect();
        let remaining = r.ticks_total.saturating_sub(obs.tick + 1);
        let steps = 24_u32.min(remaining).max(1);
        let mut boundaries: Vec<_> = (1..=steps)
            .map(|i| obs.tick + remaining * i / steps)
            .chain(pending.iter().map(|(_, _, eff)| eff.saturating_sub(1)))
            .filter(|t| *t > obs.tick)
            .collect();
        boundaries.sort_unstable();
        boundaries.dedup();
        let mut level = current;
        let mut value = 0.0;
        let mut last = obs.tick;
        for end in boundaries {
            for &(_, d, eff) in &pending {
                if eff == last + 1 {
                    level = (level + d as f64).max(0.0);
                }
            }
            let dt = end - last;
            let before = level;
            if let Some(risk) = &r.dividend_risk {
                let reviews = dt as f64
                    / (r.ticks_total as f64
                        * (risk.review_fraction.0 + risk.review_fraction.1)
                        * 0.5);
                let cut = (risk.impaired_retention.0 + risk.impaired_retention.1) * 0.5;
                let recovery = (risk.recovered_retention.0 + risk.recovered_retention.1) * 0.5;
                // A low observed rate can recover probabilistically, but a prior
                // peak is discounted as it may have belonged to a transitory boom.
                let damaged = (1.0 - level / (anchor * 0.5)).clamp(0.0, 1.0);
                let recovery_target = if current == 0.0 {
                    anchor * cut * 0.5
                } else {
                    anchor * recovery * 0.65
                };
                let upside =
                    damaged * risk.recovery_probability * (recovery_target - level).max(0.0);
                let downside = level
                    * (risk.suspension_probability
                        + (1.0 - damaged) * risk.impairment_probability * (1.0 - cut));
                level = (level + reviews.min(1.0) * (upside - downside)).max(0.0);
            }
            value += (before + level) * 0.5 / r.dividend_scale as f64
                * payments_after(r, last, end) as f64;
            last = end;
        }
        value + level / r.dividend_scale as f64 * r.terminal_dividend_payments(terminal)
    }
}

impl Bot for Harbor {
    fn on_tick(&mut self, obs: &Observation) -> Vec<Action> {
        self.knowledge.update(obs);
        for (p, b) in self.peak.iter_mut().zip(&obs.books) {
            *p = p.max(b.dividend as f64);
        }
        if obs.tick < self.next {
            return vec![];
        }
        self.next = obs.tick + 5;
        let r = &self.rules;
        let terminal = self.knowledge.terminal(r, Estimate::Mid);
        let eq = obs.account.equity.max(0) as f64;
        let mut quotes = vec![Quote::default(); obs.books.len()];
        for (s, b) in obs.books.iter().enumerate() {
            let value = self.estimate(obs, s, terminal);
            let mark = b.mark.max(r.tick_size) as f64;
            let lot = (eq * 0.012 / mark).max(1.0) as Qty;
            let have = obs.account.shares[s];
            let edge = (value / mark - 1.0).max(0.0);
            let target = (eq * (edge * 0.10).min(0.16) / mark) as Qty;
            let buy_value = value + combo_value(r, obs, s, lot);
            if have < target && !negative_pair_blocked(r, obs, s) && buy_value > r.tick_size as f64
            {
                let cap = round_tick(r, buy_value * 0.82);
                let p = b.best_bid().map_or(cap, |p| p + r.tick_size).min(cap);
                quotes[s].bid = Some((p, lot.min(target - have)));
            }
            if have > 0 {
                let q = lot.min(have);
                let sell_value = (value + combo_value(r, obs, s, -q)).max(r.tick_size as f64);
                let floor = round_tick(r, sell_value * if have > target { 0.94 } else { 1.12 });
                let p = b.best_ask().map_or(floor, |p| (p - r.tick_size).max(floor));
                quotes[s].ask = Some((p.max(quotes[s].bid.map_or(0, |x| x.0) + r.tick_size), q));
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
    fn risk_reduces_healthy_value_and_zero_is_not_guaranteed_recovery() {
        let mut s = start();
        let mut obs = observation(100, true);
        let plain = Harbor::new(&s).estimate(&obs, 0, 30.0);
        s.rules.dividend_risk = Some(DividendRiskRules::default());
        let h = Harbor::new(&s);
        assert!(h.estimate(&obs, 0, 30.0) < plain);
        obs.books[0].dividend = 0;
        let suspended = h.estimate(&obs, 0, 30.0);
        assert!(suspended > 0.0 && suspended < plain * 0.15);
        obs.tick = 2999;
        assert_eq!(h.estimate(&obs, 0, 30.0), 0.0);
    }

    #[test]
    fn known_changes_follow_effective_ticks_and_ignore_post_game_events() {
        let s = start();
        let mut obs = observation(100, true);
        obs.news = [(1, 149, -30), (2, 165, 20), (3, 3000, 900)]
            .into_iter()
            .map(|(id, effective_tick, delta)| NewsItem {
                id,
                layer: NewsLayer::Stock,
                sector: None,
                changes: vec![Change {
                    stock: 0,
                    low: delta,
                    high: delta,
                }],
                stage: 0,
                forecast: false,
                first_reveal_tick: 100,
                public_tick: 100,
                effective_tick,
                public: true,
            })
            .collect();
        let mut h = Harbor::new(&s);
        h.knowledge.update(&obs);
        let expected =
            crate::common::hold_value(&s.rules, 100, 0, 100, &[(0, -30, 149), (0, 20, 165)], 30.0);
        assert!((h.estimate(&obs, 0, 30.0) - expected).abs() < 0.00001);
    }
}
