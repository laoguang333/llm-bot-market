//! Osprey: sustained cash-flow and price leadership with ordinary-account execution.
//! No seed, identity, dragon metadata, external IO or undelivered information.
use crate::common::{payments_after, Estimate, Knowledge};
use crate::kestrel::Kestrel;
use bot_api::*;

#[derive(Default)]
struct Trend {
    rate: f64,
    price: f64,
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
            premium_cap: s.param("trend_premium", 0.025).clamp(0.0, 0.05),
            min_rate_multiple: s.param("min_rate_multiple", 0.0).max(0.0),
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
        for (s, tr) in self.trends.iter_mut().enumerate() {
            let b = &obs.books[s];
            let mid = b.mid().unwrap_or(b.last).max(1) as f64;
            let rate = b.dividend.max(1) as f64;
            let price = mid / trend_duration.max(1.0);
            let dt = tr.tick.map_or(1, |t| obs.tick.saturating_sub(t)).max(1) as f64;
            if tr.tick.is_none() {
                tr.rate = rate;
                tr.price = price;
            }
            let decay = (-dt / self.horizon).exp();
            tr.rate = decay * tr.rate + (1.0 - decay) * rate;
            tr.price = decay * tr.price + (1.0 - decay) * price;
            tr.tick = Some(obs.tick);
            let growth = (rate / tr.rate.max(1.0)).ln();
            let momentum = (price / tr.price.max(1e-9)).ln();
            let pending: f64 = self
                .know
                .pending()
                .filter(|(i, _, _)| *i == s)
                .map(|(_, d, _)| d as f64)
                .sum();
            let forward = (rate + pending).max(0.0);
            let flat = forward / r.dividend_scale as f64
                * (payments + r.terminal_dividend_payments(terminal));
            // Require fundamental confirmation; do not buy price-only bubbles.
            let initial = r.stocks[s].initial_dividend.max(1) as f64;
            if obs.tick >= self.horizon as Tick
                && rate > initial * self.min_rate_multiple
                && growth > 0.08
                && momentum > 0.025
            {
                tr.active = true;
            }
            if rate < initial * self.min_rate_multiple * (5.0 / 6.0)
                || growth < -0.025
                || momentum < -0.06
                || forward < rate * 0.9
                || mid > flat * 1.5
            {
                tr.active = false;
            }
            if tr.active {
                scores[s] = (growth + momentum).max(0.001);
            }
        }
        let mut order: Vec<_> = (0..scores.len()).collect();
        order.sort_by(|a, b| scores[*b].total_cmp(&scores[*a]));
        let fade = (r.ticks_total.saturating_sub(obs.tick) as f64 / 700.0).min(1.0);
        let mut premiums = vec![0.0; scores.len()];
        for s in order.into_iter().take(2) {
            if scores[s] > 0.0 {
                premiums[s] = self.strength * fade * (scores[s] * 0.08).min(self.premium_cap);
            }
        }
        premiums
    }
}

impl Bot for Osprey {
    fn on_tick(&mut self, obs: &Observation) -> Vec<Action> {
        let premiums = self.premiums(obs);
        self.core.decide(obs, &premiums)
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
        assert!(premium[0] > 0.0);
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
