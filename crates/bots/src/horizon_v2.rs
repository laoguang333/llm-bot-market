//! Horizon v2: cash-flow news repricing with uncertainty in position size.
//! Only public rules and delivered observations; no seed, identity or hidden world.
use crate::common::{mid_or_last, round_tick, Estimate, Knowledge};
use crate::strategic::{requote, Want};
use bot_api::*;

#[derive(Default)]
struct Anchor {
    premium: f64,
    signed: f64,
    volume: f64,
    tick: Option<Tick>,
}
#[derive(Debug)]
struct Thesis {
    mean: f64,
    uncertainty: f64,
}
pub struct Horizon {
    rules: Rules,
    knowledge: Knowledge,
    anchors: Vec<Anchor>,
    leverage: f64,
    cap: f64,
    clip: f64,
    anchor_window: f64,
    research_window: Tick,
    risk: f64,
    flow: f64,
}
impl Horizon {
    pub fn new(s: &Start) -> Self {
        Self {
            rules: s.rules.clone(),
            knowledge: Knowledge::default(),
            anchors: (0..s.rules.stocks.len())
                .map(|_| Anchor::default())
                .collect(),
            // Match default Titan's portfolio capacity controls.
            leverage: s.param("leverage", 2.0).clamp(0.0, s.rules.max_leverage),
            cap: s.param("cap", 0.45).clamp(0.0, 1.0),
            clip: s.param("clip", 0.025).clamp(0.0, 0.1),
            anchor_window: s.param("anchor_window", 150.0).max(1.0),
            // Receiving information is not an obligation to expose capital now.
            // Keep distant research, activate it as the event approaches.
            research_window: s.param("research_window", 300.0).max(0.0) as Tick,
            risk: s.param("risk", 4.0).max(0.0),
            flow: s.param("flow", 0.02).clamp(0.0, 0.1),
        }
    }
    /// Means are not penalized per event. RSS is a sizing heuristic, not a
    /// claim that delivered intervals are calibrated independent posteriors.
    fn thesis(&self, stock: usize, obs: &Observation, terminal: f64, public_only: bool) -> Thesis {
        let r = &self.rules;
        let mut changes: Vec<_> = self
            .knowledge
            .heard
            .values()
            .filter(|h| {
                (!public_only || h.public)
                    && (h.public || h.effective.saturating_sub(obs.tick) <= self.research_window)
                    && h.effective < r.ticks_total
            })
            .filter_map(|h| {
                h.ranges.get(&stock).map(|&(lo, hi)| {
                    (
                        h.effective,
                        (lo as f64 + hi as f64) / 2.0,
                        (hi as f64 - lo as f64) / 12.0_f64.sqrt(),
                    )
                })
            })
            .collect();
        changes.sort_by(|a, b| {
            a.0.cmp(&b.0)
                .then(a.1.total_cmp(&b.1))
                .then(a.2.total_cmp(&b.2))
        });
        let terminal_count = r.terminal_dividend_payments(terminal);
        let mut rate = obs.books[stock].dividend as f64;
        let mut at = obs.tick;
        let mut mean = 0.0;
        let mut variance = 0.0;
        let mut i = 0;
        while i < changes.len() {
            let effective = changes[i].0;
            let until = effective.saturating_sub(1);
            mean += rate.max(r.min_dividend as f64) / r.dividend_scale as f64
                * crate::common::payments_after(r, at, until) as f64;
            let mut delta = 0.0;
            while i < changes.len() && changes[i].0 == effective {
                delta += changes[i].1;
                let sensitivity = (crate::common::payments_after(r, until, r.ticks_total) as f64
                    + terminal_count)
                    / r.dividend_scale as f64;
                variance += (changes[i].2 * sensitivity).powi(2);
                i += 1;
            }
            // Delivered deltas already include the world's actual floor. Missing
            // events may make this partial path negative: retain the deficit so
            // a later recovery cannot create fictitious additional growth.
            rate += delta;
            at = until;
        }
        mean += rate.max(r.min_dividend as f64) / r.dividend_scale as f64
            * (crate::common::payments_after(r, at, r.ticks_total) as f64 + terminal_count);
        Thesis {
            mean,
            uncertainty: variance.sqrt(),
        }
    }
    fn position_weight(&self, fair: f64, mid: f64, uncertainty: f64, mean: f64) -> f64 {
        let confidence = 1.0 / (1.0 + self.risk * uncertainty / mean.max(1.0));
        let friction = 2.0 * self.rules.trade_fee_bps as f64 / 10_000.0;
        (12.0 * (fair / mid - 1.0 - friction)).clamp(0.0, self.cap) * confidence
    }
}
impl Bot for Horizon {
    fn on_tick(&mut self, obs: &Observation) -> Vec<Action> {
        self.knowledge.update(obs);
        let r = &self.rules;
        let terminal = self.knowledge.terminal(r, Estimate::Mid);
        let eq = obs.account.equity.max(0) as f64;
        let n = self.anchors.len();
        let remaining = r.ticks_total.saturating_sub(obs.tick) as f64;
        let mut weights = vec![0.0; n];
        let mut fairs = vec![0.0; n];
        for s in 0..n {
            let mid = mid_or_last(&obs.books[s]).max(r.tick_size) as f64;
            let thesis = self.thesis(s, obs, terminal, false);
            let public = self.thesis(s, obs, terminal, true).mean.max(1.0);
            let value = thesis.mean.max(1.0);
            let anchor = &mut self.anchors[s];
            let dt = anchor.tick.map_or(1, |t| obs.tick.saturating_sub(t)).max(1) as f64;
            if anchor.tick.is_none() {
                // Tick-zero research must not disappear into initialization.
                anchor.premium = (mid / public).clamp(0.3, 3.0);
            } else {
                let decay = (-dt / self.anchor_window).exp();
                // Reprice new information, then learn the market's capitalization.
                anchor.premium =
                    decay * anchor.premium + (1.0 - decay) * (mid / value).clamp(0.3, 3.0);
            }
            let decay = (-dt / 20.0).exp();
            anchor.signed *= decay;
            anchor.volume *= decay;
            for trade in &obs.trades[s] {
                let qty = trade.qty.max(0) as f64;
                anchor.volume += qty;
                anchor.signed += if trade.aggressor == Side::Buy {
                    qty
                } else {
                    -qty
                };
            }
            anchor.tick = Some(obs.tick);
            let premium = 1.0 + (remaining / 400.0).min(1.0) * (anchor.premium - 1.0);
            let flow = anchor.signed / anchor.volume.max(1.0);
            // Cash also pays idle fees. Only actually financed inventory gets a
            // loan haircut; there is no stock-only idle/waiting charge.
            let financed =
                (-(obs.account.cash as f64) / (obs.account.gross as f64).max(1.0)).clamp(0.0, 1.0);
            let financing = financed * r.loan_rate * self.anchor_window.min(remaining)
                / r.interest_period as f64;
            let fair = value * premium * (1.0 + self.flow * flow) / (1.0 + financing);
            fairs[s] = fair;
            // Negative news is in the path, with no extra sign-based position cut.
            weights[s] = self.position_weight(fair, mid, thesis.uncertainty, value);
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
            let target = (eq * weights[s] * scale / mid) as Qty;
            let have = obs.account.shares[s];
            let lot = (eq * self.clip / mid).max(1.0) as Qty;
            if have < target {
                let limit = round_tick(r, (fairs[s] * 0.9985).min(mid * 1.005));
                let qty = (target - have)
                    .min(lot)
                    .min((budget / r.buy_reserve_per_share(limit) as f64) as Qty)
                    .max(0);
                if qty == 0 {
                    continue;
                }
                if b.best_ask().is_some_and(|p| p <= limit) {
                    want[s].take = Some((Side::Buy, limit, qty));
                } else {
                    want[s].bid = Some((
                        limit.min(b.best_bid().unwrap_or(limit).saturating_add(r.tick_size)),
                        qty,
                    ));
                }
                budget -= qty as f64 * r.buy_reserve_per_share(limit) as f64;
            } else if have > target {
                let qty = (have - target).min(lot).min(obs.account.sellable[s]);
                if qty <= 0 {
                    continue;
                }
                // A smaller target is not an instruction to dump undervalued
                // inventory. Offer near fair value; cross immediately only when
                // the thesis has repriced down, or portfolio risk needs reducing.
                let reduce_risk = eq <= 0.0 || obs.account.gross as f64 > eq * self.leverage * 1.15;
                let limit = round_tick(
                    r,
                    if reduce_risk {
                        mid * 0.995
                    } else {
                        (fairs[s] * 1.0015).max(mid * 0.995)
                    },
                );
                if b.best_bid().is_some_and(|p| p >= limit) {
                    want[s].take = Some((Side::Sell, limit, qty));
                } else {
                    want[s].ask = Some((b.best_ask().unwrap_or(limit).max(limit), qty));
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
    fn news(id: u32, low: i64, high: i64, effective: Tick) -> NewsItem {
        NewsItem {
            id,
            layer: NewsLayer::Stock,
            sector: Some(0),
            changes: vec![Change {
                stock: 0,
                low,
                high,
            }],
            stage: 0,
            forecast: true,
            first_reveal_tick: 10,
            public_tick: 70,
            effective_tick: effective,
            public: false,
        }
    }
    #[test]
    fn information_boundary_and_insolvency() {
        let s = start();
        let mut other = s.clone();
        other.seed = u64::MAX;
        other.account_index = 87;
        other.forecast_access = ForecastAccess {
            horizon: 1000,
            coverage: 1.,
            range_width: 0.,
        };
        let (mut a, mut b) = (Horizon::new(&s), Horizon::new(&other));
        for tick in 0..100 {
            let o = observation(tick, true);
            assert_eq!(a.on_tick(&o), b.on_tick(&o));
        }
        let mut o = observation(100, true);
        o.account.equity = -1;
        assert!(!a.on_tick(&o).iter().any(|a| matches!(
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
    #[test]
    fn uncertainty_does_not_reverse_positive_mean() {
        let mut bot = Horizon::new(&start());
        let mut o = observation(1, false);
        let plain = bot.thesis(0, &o, 30., false).mean;
        o.news = vec![news(1, -32, 49, 250)];
        bot.knowledge.update(&o);
        let t = bot.thesis(0, &o, 30., false);
        assert!(t.mean > plain && t.uncertainty > 0.);
        assert!(
            bot.position_weight(110., 100., 20., 100.) < bot.position_weight(110., 100., 0., 100.)
        );
        assert!(bot.position_weight(110., 100., 20., 100.) > 0.);
    }
    #[test]
    fn resizing_keeps_price_discipline_unless_portfolio_risk_requires_exit() {
        let mut s = start();
        s.params.insert("cap".into(), 0.05);
        s.params.insert("flow".into(), 0.0);
        let mut o = observation(1, false);
        o.news = vec![news(1, 80, 80, 100)];
        let actions = Horizon::new(&s).on_tick(&o);
        assert!(actions.iter().any(|a| matches!(a, Action::Limit {
            stock:0, side:Side::Sell, tif:Tif::Gtc, price, ..
        } if *price > o.books[0].best_ask().unwrap())));
        assert!(!actions.iter().any(|a| matches!(
            a,
            Action::Limit {
                stock: 0,
                side: Side::Sell,
                tif: Tif::Ioc,
                ..
            }
        )));
        o.account.equity = 100_000;
        assert!(Horizon::new(&s).on_tick(&o).iter().any(|a| matches!(
            a,
            Action::Limit {
                stock: 0,
                side: Side::Sell,
                tif: Tif::Ioc,
                ..
            }
        )));
    }
    #[test]
    fn simultaneous_changes_net_before_flooring_and_expire() {
        let mut bot = Horizon::new(&start());
        let mut o = observation(1, true);
        o.books[0].dividend = 0;
        o.news = vec![news(1, -20, -20, 100), news(2, 20, 20, 100)];
        bot.knowledge.update(&o);
        assert_eq!(bot.thesis(0, &o, 30., false).mean, 0.);
        o.tick = 100;
        o.news.clear();
        bot.knowledge.update(&o);
        assert!(bot.knowledge.heard.is_empty());
    }
    #[test]
    fn incomplete_news_path_does_not_create_growth_by_resetting_negative_state() {
        let mut bot = Horizon::new(&start());
        let mut o = observation(1, false);
        let plain = bot.thesis(0, &o, 30., false).mean;
        o.news = vec![news(1, -200, -200, 100), news(2, 200, 200, 300)];
        bot.knowledge.update(&o);
        assert!(bot.thesis(0, &o, 30., false).mean < plain);
    }
    #[test]
    fn temporary_loss_recovery_and_discounted_price_can_be_bought() {
        let mut s = start();
        s.params.insert("flow".into(), 0.);
        let mut bot = Horizon::new(&s);
        let mut o = observation(1, false);
        bot.on_tick(&observation(0, false));
        o.news = vec![news(1, -10, -10, 100), news(2, 40, 40, 300)];
        o.account.shares.fill(0);
        o.account.sellable.fill(0);
        o.account.gross = 0;
        o.books[0].bids[0].price = 7000;
        o.books[0].asks[0].price = 7010;
        assert!(bot.on_tick(&o).iter().any(|a| matches!(
            a,
            Action::Limit {
                stock: 0,
                side: Side::Buy,
                ..
            }
        )));
    }
    #[test]
    fn near_window_can_ignore_distant_research_without_changing_information_access() {
        let mut s = start();
        s.params.insert("research_window".into(), 300.);
        let mut bot = Horizon::new(&s);
        let mut o = observation(1, true);
        let plain = bot.thesis(0, &o, 30., false).mean;
        o.news = vec![news(1, 20, 20, 600)];
        bot.knowledge.update(&o);
        assert_eq!(bot.thesis(0, &o, 30., false).mean, plain);
        o.tick = 301;
        assert!(bot.thesis(0, &o, 30., false).mean > bot.thesis(0, &o, 30., true).mean);
    }
}
