//! Capacity-oriented portfolio rebalancing around news-adjusted market anchors.
//! Inputs are only Rules and delivered observations. No seed, identities or hidden world.
use crate::common::{hold_value, mid_or_last, round_tick, Estimate, Knowledge};
use crate::strategic::{requote, Want};
use bot_api::*;

#[derive(Default)]
struct Anchor {
    premium: f64,
    volume: f64,
    signed: f64,
    tick: Option<Tick>,
}

pub struct Titan {
    rules: Rules,
    knowledge: Knowledge,
    anchors: Vec<Anchor>,
    horizon: f64,
    base: f64,
    gain: f64,
    cap: f64,
    leverage: f64,
    flow: f64,
    margin: f64,
    clip: f64,
    sweep: f64,
}

impl Titan {
    pub fn new(s: &Start) -> Self {
        Self {
            rules: s.rules.clone(),
            knowledge: Knowledge::default(),
            anchors: (0..s.rules.stocks.len())
                .map(|_| Anchor::default())
                .collect(),
            horizon: s.param("horizon", 150.0).max(1.0),
            base: s.param("base", 0.0).max(0.0),
            gain: s.param("gain", 12.0).max(0.0),
            cap: s.param("cap", 0.45).max(0.0),
            leverage: s.param("leverage", 2.0).clamp(0.0, s.rules.max_leverage),
            flow: s.param("flow", 0.02),
            margin: s.param("margin", 0.0015).max(0.0),
            clip: s.param("clip", 0.025).max(0.0),
            sweep: s.param("sweep", 0.005).max(0.0),
        }
    }
}

impl Bot for Titan {
    fn on_tick(&mut self, obs: &Observation) -> Vec<Action> {
        self.knowledge.update(obs);
        let r = &self.rules;
        let pending: Vec<_> = self.knowledge.pending().collect();
        let terminal = self.knowledge.terminal(r, Estimate::Mid);
        let eq = obs.account.equity.max(0) as f64;
        let n = self.anchors.len();
        let mut fairs = vec![0.0; n];
        let mut weights = vec![0.0; n];
        for (s, b) in obs.books.iter().enumerate().take(n) {
            let mid = mid_or_last(b).max(r.tick_size) as f64;
            let value = hold_value(r, obs.tick, s, b.dividend, &pending, terminal).max(1.0);
            let anchor = &mut self.anchors[s];
            let dt = anchor.tick.map_or(1, |t| obs.tick.saturating_sub(t)).max(1) as f64;
            if anchor.tick.is_none() {
                anchor.premium = mid / value;
            } else {
                let a = (-dt / self.horizon).exp();
                anchor.premium = a * anchor.premium + (1.0 - a) * mid / value;
            }
            let a = (-dt / 20.0).exp();
            anchor.volume *= a;
            anchor.signed *= a;
            for trade in &obs.trades[s] {
                let q = trade.qty.max(0) as f64;
                anchor.volume += q;
                anchor.signed += if trade.aggressor == Side::Buy { q } else { -q };
            }
            anchor.tick = Some(obs.tick);
            // Near settlement the market anchor gives way to legally inferred cash flows.
            let remaining = r.ticks_total.saturating_sub(obs.tick) as f64;
            let blend = (remaining / 400.0).min(1.0);
            let premium = 1.0 + blend * (anchor.premium.clamp(0.3, 3.0) - 1.0);
            let flow = anchor.signed / anchor.volume.max(1.0);
            let fair = value * premium * (1.0 + self.flow * flow);
            fairs[s] = fair;
            weights[s] = (self.base + self.gain * (fair / mid - 1.0)).clamp(0.0, self.cap);
        }
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
            let target = (weights[s] * scale * eq / mid) as Qty;
            let have = obs.account.shares[s];
            let lot = (eq * self.clip / mid).max(1.0) as Qty;
            let fair = fairs[s];
            if have < target {
                let limit = round_tick(
                    r,
                    (fair * (1.0 - self.margin)).min(mid * (1.0 + self.sweep)),
                );
                let q = (target - have)
                    .min(lot)
                    .min((budget / r.buy_reserve_per_share(limit) as f64) as Qty)
                    .max(0);
                if q == 0 {
                    continue;
                }
                if b.best_ask().is_some_and(|a| a <= limit) {
                    want[s].take = Some((Side::Buy, limit, q));
                } else {
                    let p = limit.min(b.best_bid().unwrap_or(limit).saturating_add(r.tick_size));
                    want[s].bid = Some((p, q));
                }
                budget -= q as f64 * r.buy_reserve_per_share(limit) as f64;
            } else if have > target {
                let limit = round_tick(
                    r,
                    (fair * (1.0 + self.margin)).max(mid * (1.0 - self.sweep)),
                );
                let q = (have - target).min(lot).min(obs.account.sellable[s]);
                if q == 0 {
                    continue;
                }
                if b.best_bid().is_some_and(|bid| bid >= limit) {
                    want[s].take = Some((Side::Sell, limit, q));
                } else {
                    let p = limit.max(b.best_ask().unwrap_or(limit).saturating_sub(r.tick_size));
                    want[s].ask = Some((p, q));
                }
            }
        }
        requote(obs, r, &want, 0.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::news_test_support::{observation, start};

    #[test]
    fn seed_and_account_identity_do_not_change_decisions() {
        let s = start();
        let mut other = s.clone();
        other.seed = u64::MAX;
        other.account_index = 57;
        other.account = "unrelated".into();
        other.news_multiplier = 10.0;
        let (mut a, mut b) = (Titan::new(&s), Titan::new(&other));
        for tick in 0..100 {
            let o = observation(tick, tick < 50);
            assert_eq!(
                format!("{:?}", a.on_tick(&o)),
                format!("{:?}", b.on_tick(&o))
            );
        }
    }

    #[test]
    fn delivered_positive_news_raises_target_before_effective_tick() {
        let mut s = start();
        // Isolate news: a strong simultaneous tape signal can already fill the order cap.
        s.params.insert("flow".into(), 0.0);
        let (mut plain, mut informed) = (Titan::new(&s), Titan::new(&s));
        plain.on_tick(&observation(0, true));
        informed.on_tick(&observation(0, true));
        let o = observation(1, true);
        let mut with_news = o.clone();
        with_news.news.push(NewsItem {
            id: 1,
            layer: NewsLayer::Stock,
            sector: Some(0),
            changes: vec![Change {
                stock: 0,
                low: 20,
                high: 30,
            }],
            stage: 0,
            first_reveal_tick: 1,
            public_tick: 61,
            effective_tick: 121,
            public: false,
        });
        let a = plain.on_tick(&o);
        let b = informed.on_tick(&with_news);
        assert_ne!(format!("{a:?}"), format!("{b:?}"));
        assert!(b.iter().any(|a| matches!(
            a,
            Action::Limit {
                stock: 0,
                side: Side::Buy,
                ..
            }
        )));
    }
}
