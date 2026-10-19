//! Long cash-flow research: only delivered observations and public rules.
//! No hidden seed reconstruction, identity inference, filesystem or network access.
use crate::common::{hold_value, mid_or_last, round_tick, Estimate, Knowledge};
use crate::strategic::{requote, Want};
use bot_api::*;

pub struct Horizon {
    rules: Rules,
    knowledge: Knowledge,
    premium: Vec<f64>,
    last: Option<Tick>,
    leverage: f64,
    cap: f64,
    clip: f64,
}

impl Horizon {
    pub fn new(s: &Start) -> Self {
        Self {
            rules: s.rules.clone(),
            knowledge: Knowledge::default(),
            premium: vec![1.0; s.rules.stocks.len()],
            last: None,
            leverage: s.param("leverage", 1.2).clamp(0.0, s.rules.max_leverage),
            cap: s.param("cap", 0.35).clamp(0.0, 1.0),
            clip: s.param("clip", 0.02).clamp(0.0, 0.1),
        }
    }

    /// Integrate the legally known rate path, flooring each intermediate rate.
    /// Return conservative value, research waiting time and nearest negative event.
    fn thesis(&self, s: usize, obs: &Observation, terminal: f64) -> (f64, f64, Option<Tick>) {
        let r = &self.rules;
        let mut changes: Vec<_> = self
            .knowledge
            .heard
            .values()
            .filter_map(|h| {
                h.ranges.get(&s).map(|&(lo, hi)| {
                    (
                        h.effective,
                        (lo + hi) as f64 / 2.0 - 0.20 * (hi - lo) as f64,
                    )
                })
            })
            .collect();
        changes.sort_by_key(|c| c.0);
        let mut at = obs.tick;
        let mut rate = obs.books[s].dividend as f64;
        let mut value = 0.0;
        let mut weighted_wait = 0.0;
        let mut size = 0.0;
        let mut negative = None;
        for (effective, delta) in changes {
            if effective >= r.ticks_total {
                continue;
            }
            let until = effective.saturating_sub(1);
            value += rate.max(0.0) / r.dividend_scale as f64
                * crate::common::payments_after(r, at, until) as f64;
            rate = (rate + delta).max(0.0);
            at = until;
            size += delta.abs();
            weighted_wait += delta.abs() * effective.saturating_sub(obs.tick) as f64;
            if delta < 0.0 && negative.is_none() {
                negative = Some(effective);
            }
        }
        value += rate / r.dividend_scale as f64
            * (crate::common::payments_after(r, at, r.ticks_total) as f64
                + r.terminal_dividend_payments(terminal));
        (
            value,
            if size > 0.0 {
                weighted_wait / size
            } else {
                50.0
            },
            negative,
        )
    }
}

impl Bot for Horizon {
    fn on_tick(&mut self, obs: &Observation) -> Vec<Action> {
        self.knowledge.update(obs);
        let r = &self.rules;
        let terminal = self.knowledge.terminal(r, Estimate::Mid);
        let n = self.premium.len();
        let eq = obs.account.equity.max(0) as f64;
        let remaining = r.ticks_total.saturating_sub(obs.tick) as f64;
        let dt = self.last.map_or(1, |t| obs.tick.saturating_sub(t)).max(1) as f64;
        let decay = (-dt / 1500.0).exp();
        let mut weights = vec![0.0; n];
        let mut fairs = vec![0.0; n];
        for s in 0..n {
            let book = &obs.books[s];
            let mid = mid_or_last(book).max(r.tick_size) as f64;
            let plain = hold_value(r, obs.tick, s, book.dividend, &[], terminal).max(1.0);
            // Slow market calibration uses CURRENT rates only. New private future
            // changes are not immediately normalized away by an anchor update.
            self.premium[s] =
                decay * self.premium[s] + (1.0 - decay) * (mid / plain).clamp(0.3, 3.0);
            let (value, wait, negative) = self.thesis(s, obs, terminal);
            let calibration = 1.0 + 0.65 * (remaining / 1000.0).min(1.0) * (self.premium[s] - 1.0);
            let carrying = r.idle_rate * wait.min(1000.0) / r.idle_period as f64
                + r.loan_rate * (self.leverage - 1.0).max(0.0) * wait.min(1000.0)
                    / r.interest_period as f64;
            let fair = value * calibration / (1.0 + carrying);
            fairs[s] = fair;
            let costs = 2.0 * r.trade_fee_bps as f64 / 10_000.0 + 0.003;
            weights[s] = (4.0 * (fair / mid - 1.0 - costs)).clamp(0.0, self.cap);
            // A known temporary adverse event permits a starter, with most of the
            // intended inventory reserved for after its effect has been observed.
            if negative.is_some_and(|t| t > obs.tick && t - obs.tick < 400) {
                weights[s] *= 0.30;
            }
        }
        self.last = Some(obs.tick);
        let scale = (self.leverage / weights.iter().sum::<f64>().max(0.001)).min(1.0);
        let mut budget =
            (eq * self.leverage - obs.account.gross as f64 - obs.account.reserved_buy as f64)
                .max(0.0)
                .min(obs.account.buying_power.max(0) as f64);
        let mut want = vec![Want::default(); n];
        for k in 0..n {
            let s = (k + obs.tick as usize) % n;
            let b = &obs.books[s];
            let mid = mid_or_last(b).max(r.tick_size) as f64;
            let target = (eq * weights[s] * scale / mid) as Qty;
            let have = obs.account.shares[s];
            let lot = (eq * self.clip / mid).max(1.0) as Qty;
            if have < target {
                let limit = round_tick(r, (fairs[s] * 0.997).min(mid * 1.003));
                let q = (target - have)
                    .min(lot)
                    .min((budget / r.buy_reserve_per_share(limit) as f64) as Qty)
                    .max(0);
                if q == 0 {
                    continue;
                }
                if b.best_ask().is_some_and(|p| p <= limit) {
                    want[s].take = Some((Side::Buy, limit, q));
                } else {
                    want[s].bid = Some((limit.min(b.best_bid().unwrap_or(limit) + r.tick_size), q));
                }
                budget -= q as f64 * r.buy_reserve_per_share(limit) as f64;
            } else if have > target + (target / 10) {
                let q = (have - target).min(lot).min(obs.account.sellable[s]);
                if q <= 0 {
                    continue;
                }
                // A rejected thesis must actually unwind, not leave an ask at its
                // obsolete fair value. Small marketable slices bound price impact.
                let limit = round_tick(r, mid * 0.997);
                if b.best_bid().is_some_and(|p| p >= limit) {
                    want[s].take = Some((Side::Sell, limit, q));
                } else {
                    want[s].ask = Some((b.best_ask().unwrap_or(limit).max(limit), q));
                }
            }
        }
        requote(obs, r, &want, 0.0005)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::news_test_support::{observation, start};
    #[test]
    fn same_observations_same_actions_regardless_of_seed_or_permission() {
        let s = start();
        let mut other = s.clone();
        other.seed = u64::MAX;
        other.account_index = 73;
        other.forecast_access = ForecastAccess {
            horizon: 1000,
            coverage: 1.,
            range_width: 0.,
        };
        let (mut a, mut b) = (Horizon::new(&s), Horizon::new(&other));
        for t in 0..100 {
            let o = observation(t, true);
            assert_eq!(a.on_tick(&o), b.on_tick(&o));
        }
    }
    #[test]
    fn future_recovery_can_outweigh_near_bad_news_and_is_removed_when_effective() {
        let s = start();
        let mut bot = Horizon::new(&s);
        let mut o = observation(1, true);
        let initial = bot.thesis(0, &o, 30.).0;
        let item = |id, delta, effective| NewsItem {
            id,
            layer: NewsLayer::Stock,
            sector: Some(0),
            changes: vec![Change {
                stock: 0,
                low: delta,
                high: delta,
            }],
            stage: 0,
            forecast: true,
            first_reveal_tick: 10,
            public_tick: 70,
            effective_tick: effective,
            public: false,
        };
        o.news = vec![item(1, -5, 100), item(2, 30, 300)];
        bot.knowledge.update(&o);
        let (v, wait, bad) = bot.thesis(0, &o, 30.);
        assert!(v > initial && wait > 100.);
        assert_eq!(bad, Some(100));
        o.tick = 301;
        o.news.clear();
        bot.knowledge.update(&o);
        assert_eq!(bot.knowledge.heard.len(), 0);
    }
    #[test]
    fn insolvent_account_never_adds_risk() {
        let mut o = observation(1, true);
        o.account.equity = -1;
        assert!(!Horizon::new(&start()).on_tick(&o).iter().any(|a| matches!(
            a,
            Action::Limit {
                side: Side::Buy,
                ..
            } | Action::Market {
                side: Side::Buy,
                ..
            }
        )));
    }
}
