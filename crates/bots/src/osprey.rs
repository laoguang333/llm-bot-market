//! Osprey: sustained cash-flow and price leadership with ordinary-account execution.
//! No seed, identity, dragon metadata, external IO or undelivered information.
use crate::common::{payments_after, Estimate, Knowledge};
use crate::kestrel::Kestrel;
use bot_api::*;

#[derive(Default)]
struct Trend {
    rate: f64,
    price: f64,
    fast_rate: f64,
    peak: f64,
    cooldown_until: Tick,
    tick: Option<Tick>,
    active: bool,
}

pub struct Osprey {
    core: Kestrel,
    rules: Rules,
    know: Knowledge,
    trends: Vec<Trend>,
    strength: f64,
    horizon: f64,
    premium_cap: f64,
    min_rate_multiple: f64,
}

impl Osprey {
    pub fn new(s: &Start) -> Self {
        Self {
            core: Kestrel::new(s),
            rules: s.rules.clone(),
            know: Knowledge::default(),
            trends: (0..s.shares.len()).map(|_| Trend::default()).collect(),
            strength: s.param("strength", 1.0).clamp(0.0, 1.0),
            horizon: s.param("trend_horizon", 600.0).max(1.0),
            premium_cap: s.param("trend_premium", 0.04).clamp(0.0, 0.05),
            min_rate_multiple: s.param("min_rate_multiple", 1.2).max(0.0),
        }
    }

    fn premiums(&mut self, obs: &Observation) -> Vec<f64> {
        self.know.update(obs);
        let r = &self.rules;
        let terminal = self.know.terminal(r, Estimate::Mid);
        let payments = payments_after(r, obs.tick, r.ticks_total) as f64;
        // A fixed public terminal midpoint avoids trend jumps on private hint updates.
        let trend_duration = payments
            + r.terminal_dividend_payments(
                (r.terminal_payments_min + r.terminal_payments_max) as f64 * 0.5,
            );
        let mut scores = vec![0.0; self.trends.len()];
        let mut adjustments = vec![0.0; self.trends.len()];
        for (s, tr) in self.trends.iter_mut().enumerate() {
            let b = &obs.books[s];
            let mid = b.mid().unwrap_or(b.last).max(1) as f64;
            let rate = b.dividend.max(1) as f64;
            let price = mid / trend_duration.max(1.0);
            let dt = tr.tick.map_or(1, |t| obs.tick.saturating_sub(t)).max(1) as f64;
            if tr.tick.is_none() {
                tr.rate = rate;
                tr.price = price;
                tr.fast_rate = rate;
            }
            let decay = (-dt / self.horizon).exp();
            tr.rate = decay * tr.rate + (1.0 - decay) * rate;
            tr.price = decay * tr.price + (1.0 - decay) * price;
            let fast = (-dt / 200.0).exp();
            tr.fast_rate = fast * tr.fast_rate + (1.0 - fast) * rate;
            tr.tick = Some(obs.tick);
            let growth = (rate / tr.rate.max(1.0)).ln();
            let momentum = (price / tr.price.max(1e-9)).ln();
            let pending: f64 = self
                .know
                .pending()
                .filter(|(i, _, effective)| *i == s && *effective < r.ticks_total)
                .map(|(_, d, _)| d as f64)
                .sum();
            let forward = (rate + pending).max(0.0);
            let flat = forward / r.dividend_scale as f64
                * (payments + r.terminal_dividend_payments(terminal));
            // Require fundamental confirmation; do not buy price-only bubbles.
            let initial = r.stocks[s].initial_dividend.max(1) as f64;
            if obs.tick >= self.horizon as Tick
                && obs.tick >= tr.cooldown_until
                && rate > initial * self.min_rate_multiple
                && growth > 0.08
                && momentum > 0.025
                && tr.fast_rate > tr.rate * 1.03
            {
                if !tr.active {
                    tr.peak = price;
                }
                tr.active = true;
            }
            if tr.active {
                tr.peak = tr.peak.max(price);
            }
            let stopped = tr.active && price < tr.peak * 0.80;
            if rate < initial * self.min_rate_multiple * (5.0 / 6.0)
                || growth < -0.025
                || momentum < -0.06
                || forward < rate * 0.9
                || mid > flat * 1.5
                || stopped
            {
                tr.active = false;
                if stopped {
                    tr.cooldown_until = obs.tick.saturating_add(100);
                }
            }
            if tr.active {
                scores[s] = (growth + momentum + 0.1 * (rate / initial).ln()).max(0.001);
                // Tolerate a smaller valuation edge only on inventory already held.
                // Reduce this tolerance between 10% and 20% normalized drawdown.
                let drawdown_fade = ((price / tr.peak - 0.80) / 0.10).clamp(0.0, 1.0);
                adjustments[s] = (scores[s] * 0.12).min(self.premium_cap) * drawdown_fade;
            }
        }
        let mut order: Vec<_> = (0..scores.len()).collect();
        order.sort_by(|a, b| scores[*b].total_cmp(&scores[*a]));
        let fade = (r.ticks_total.saturating_sub(obs.tick) as f64 / 700.0).min(1.0);
        let mut premiums = vec![0.0; scores.len()];
        for s in order.into_iter().take(2) {
            if scores[s] > 0.0 {
                premiums[s] = self.strength * fade * adjustments[s];
            }
        }
        premiums
    }
}

impl Bot for Osprey {
    fn on_tick(&mut self, obs: &Observation) -> Vec<Action> {
        let premiums = self.premiums(obs);
        self.core.decide_with_holding(obs, &[], &premiums)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::news_test_support::{observation, start};

    #[test]
    fn disabled_overlay_matches_kestrel_and_ignores_identity() {
        let mut s = start();
        s.params.insert("strength".into(), 0.0);
        let mut a = Osprey::new(&s);
        s.seed = u64::MAX;
        s.account_index = 999;
        s.owner = "other".into();
        let mut b = Kestrel::new(&s);
        for tick in 0..1200 {
            let mut o = observation(tick, tick % 3 == 0);
            o.books[0].dividend += tick as i64 / 10;
            assert_eq!(a.on_tick(&o), b.on_tick(&o));
        }
    }

    #[test]
    fn confirmed_growth_builds_and_bad_growth_releases_premium() {
        let s = start();
        let mut bot = Osprey::new(&s);
        let mut premium = vec![];
        for tick in 0..1200 {
            let mut o = observation(tick, true);
            o.books[0].dividend = 100 + tick as i64 / 4;
            let price = 10000 + tick as i64 * 15;
            o.books[0].bids[0].price = price - 10;
            o.books[0].asks[0].price = price + 10;
            premium = bot.premiums(&o);
        }
        assert!(bot.trends[0].active);
        assert!(premium[0] > 0.0, "hold a confirmed leader more patiently");
        assert_eq!(premium[1], 0.0);
        let mut crash = observation(1200, false);
        crash.books[0].dividend = 10;
        assert_eq!(bot.premiums(&crash)[0], 0.0);
    }

    #[test]
    fn price_only_rally_does_not_activate_overlay() {
        let mut bot = Osprey::new(&start());
        for tick in 0..1200 {
            let mut o = observation(tick, true);
            o.books[0].bids[0].price += tick as i64 * 20;
            o.books[0].asks[0].price += tick as i64 * 20;
            assert!(bot.premiums(&o).iter().all(|w| *w == 0.0));
        }
    }

    #[test]
    fn drawdown_removes_premium_then_recovery_can_reenter_after_cooldown() {
        let mut bot = Osprey::new(&start());
        let mut o = observation(0, true);
        for tick in 0..1200 {
            o.tick = tick;
            o.books[0].dividend = 100 + tick as i64 / 4;
            o.books[0].bids[0].price = 9990 + tick as i64 * 15;
            o.books[0].asks[0].price = 10010 + tick as i64 * 15;
            bot.premiums(&o);
        }
        assert!(bot.trends[0].active);
        o.tick = 1200;
        o.books[0].bids[0].price = 21990;
        o.books[0].asks[0].price = 22010;
        assert_eq!(bot.premiums(&o)[0], 0.0);
        assert_eq!(bot.trends[0].cooldown_until, 1300);
        o.tick = 1201;
        o.books[0].bids[0].price = 29990;
        o.books[0].asks[0].price = 30010;
        assert_eq!(bot.premiums(&o)[0], 0.0);
        o.tick = 1400;
        bot.premiums(&o);
        assert!(bot.trends[0].active);
        o.tick = 1401;
        let duration = payments_after(&bot.rules, o.tick, bot.rules.ticks_total) as f64
            + bot.rules.terminal_dividend_payments(30.0);
        let pullback = (bot.trends[0].peak * duration * 0.95) as i64;
        o.books[0].bids[0].price = pullback - 1;
        o.books[0].asks[0].price = pullback + 1;
        assert!(bot.premiums(&o)[0] > 0.0);
    }

    #[test]
    fn rebound_below_initial_rate_is_not_a_growth_leader() {
        let mut s = start();
        s.params.insert("min_rate_multiple".into(), 1.5);
        let mut bot = Osprey::new(&s);
        for tick in 0..1200 {
            let mut o = observation(tick, true);
            o.books[0].dividend = 10 + tick as i64 / 20;
            o.books[0].bids[0].price += tick as i64 * 20;
            o.books[0].asks[0].price += tick as i64 * 20;
            assert_eq!(bot.premiums(&o)[0], 0.0);
        }
    }

    #[test]
    fn active_overlay_ignores_seed_and_respects_cash_budget() {
        let s = start();
        let mut other = s.clone();
        other.seed = 9999;
        other.owner = "different".into();
        other.rules.dragon = None;
        let (mut a, mut b) = (Osprey::new(&s), Osprey::new(&other));
        for tick in 0..1200 {
            let mut o = observation(tick, true);
            o.books[0].dividend += tick as i64 / 4;
            o.books[0].bids[0].price += tick as i64 * 15;
            o.books[0].asks[0].price += tick as i64 * 15;
            o.account.buying_power = 120000;
            let acts = a.on_tick(&o);
            assert_eq!(acts, b.on_tick(&o));
            let cost: i64 = acts
                .iter()
                .map(|a| match a {
                    Action::Limit {
                        side: Side::Buy,
                        price,
                        qty,
                        ..
                    } => s.rules.buy_reserve_per_share(*price) * qty,
                    _ => 0,
                })
                .sum();
            assert!(cost <= o.account.buying_power);
        }
    }
}
