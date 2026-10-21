//! Hidden world: market seasons, three-layer news and the dividend path.
//!
//! Everything is generated from the seed before trading starts. Bots only
//! see news once it is delivered to them and season changes as they happen.

use crate::config::Config;
use bot_api::{Change, ForecastAccess, Money, NewsItem, NewsLayer, Season, Tick};
mod forecast;
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use rand_distr::{Distribution, Normal};

mod dividend_risk;
mod dragon;
pub use dragon::DragonOutcome;

#[cfg(test)]
mod dragon_tests;

#[derive(Clone, Debug)]
pub struct Event {
    pub id: u32,
    /// Stable internal random key, independent of added events and public numbering.
    pub(crate) sampling_key: u64,
    pub layer: NewsLayer,
    pub sector: Option<usize>,
    /// True (stock, change in dividend).
    pub changes: Vec<(usize, Money)>,
    /// Standard deviation of each change, aligned with `changes`; scales the private ranges.
    pub sigma: Vec<f64>,
    pub first_reveal: Tick,
    pub public_tick: Tick,
    pub effective: Tick,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Delivery {
    pub tick: Tick,
    pub event: u32,
    /// Disclosure stage; the last stage is the public one.
    pub stage: usize,
    pub forecast: bool,
}

pub struct World {
    /// Hidden until the final report; never included in Start or observations.
    pub dragon: Option<DragonOutcome>,
    pub turbulent: Vec<bool>,
    pub season_changes: Vec<(Tick, bool)>,
    pub rate_mult: f64,
    pub size_mult: f64,
    pub events: Vec<Event>,
    /// Per stock: (effective tick, new dividend), ascending.
    pub timelines: Vec<Vec<(Tick, Money)>>,
    pub initial_dividends: Vec<Money>,
    /// Internal lookup only: callers still request their own personal tick.
    /// Large worlds retain the sparse timeline instead of allocating an unbounded table.
    dividend_cache: Vec<Vec<Money>>,
    /// Per account, ascending by tick.
    pub deliveries: Vec<Vec<Delivery>>,
    pub news_multipliers: Vec<f64>,
    pub forecast_access: Vec<ForecastAccess>,
    /// Opening price of each stock.
    pub opening_prices: Vec<Money>,
    /// Number of terminal payments, drawn from the public range and not announced.
    pub terminal_payments: i64,
    terminal_hints: Vec<(Tick, i64)>,
    seed: u64,
    range_widths: Vec<f64>,
    public_stage: usize,
}

fn mix(mut x: u64) -> u64 {
    x ^= x >> 30;
    x = x.wrapping_mul(0xbf58476d1ce4e5b9);
    x ^= x >> 27;
    x = x.wrapping_mul(0x94d049bb133111eb);
    x ^ (x >> 31)
}

pub(crate) fn unit(seed: u64, a: u64, b: u64) -> f64 {
    (mix(seed ^ mix(a.wrapping_mul(0x9e3779b97f4a7c15) ^ mix(b))) >> 11) as f64
        / (1u64 << 53) as f64
}

fn cache_dividends(
    initial: &[Money],
    timelines: &[Vec<(Tick, Money)>],
    ticks: Tick,
) -> Vec<Vec<Money>> {
    const MAX_CACHE_BYTES: usize = 8 * 1024 * 1024;
    let len = ticks as usize;
    if len
        .checked_mul(initial.len())
        .and_then(|n| n.checked_mul(std::mem::size_of::<Money>()))
        .is_none_or(|bytes| bytes > MAX_CACHE_BYTES)
    {
        return Vec::new();
    }
    initial
        .iter()
        .zip(timelines)
        .map(|(&initial, timeline)| {
            let mut values = Vec::with_capacity(len);
            let mut value = initial;
            for &(tick, next) in timeline {
                values.resize((tick as usize).min(len), value);
                value = next;
                if tick as usize >= len {
                    break;
                }
            }
            values.resize(len, value);
            values
        })
        .collect()
}

impl World {
    pub fn generate(cfg: &Config, seed: u64, n_accounts: usize) -> World {
        Self::generate_with_news_access(cfg, seed, &vec![1.0; n_accounts])
    }

    pub fn generate_with_news_access(cfg: &Config, seed: u64, news_multipliers: &[f64]) -> World {
        Self::generate_with_information(
            cfg,
            seed,
            news_multipliers,
            &vec![ForecastAccess::default(); news_multipliers.len()],
        )
    }

    pub fn generate_with_information(
        cfg: &Config,
        seed: u64,
        news_multipliers: &[f64],
        forecasts: &[ForecastAccess],
    ) -> World {
        assert_eq!(news_multipliers.len(), forecasts.len());
        assert!(news_multipliers.iter().all(|m| m.is_finite() && *m >= 0.0));
        let n_accounts = news_multipliers.len();
        let mut rng = ChaCha8Rng::seed_from_u64(seed ^ 0x5eed_0f_3011d);
        let ticks = cfg.ticks;
        let s = &cfg.world.seasons;
        let mut turbulent = Vec::with_capacity(ticks as usize);
        let mut season_changes = Vec::new();
        let mut state = false;
        for t in 0..ticks {
            let p = if state {
                s.turbulent_to_calm
            } else {
                s.calm_to_turbulent
            };
            if t > 0 && rng.gen::<f64>() < p {
                state = !state;
                season_changes.push((t, state));
            }
            turbulent.push(state);
        }

        let n_stocks = cfg.stocks.len();
        let sector_of: Vec<usize> = cfg
            .stocks
            .iter()
            .map(|st| {
                cfg.world
                    .sectors
                    .iter()
                    .position(|x| *x == st.sector)
                    .unwrap()
            })
            .collect();
        let initial: Vec<Money> = cfg.stocks.iter().map(|st| st.dividend).collect();
        let mut div = initial.clone();
        let mut timelines = vec![Vec::new(); n_stocks];
        let news = &cfg.world.news;
        let lead = news.disclosure.lead_ticks;
        let public_offset = news.disclosure.stages.last().unwrap().0;
        let min_div = cfg.world.min_dividend;
        let mut events = Vec::new();

        for t in 0..ticks {
            let (rm, sm) = if turbulent[t as usize] {
                (s.turbulent_rate_mult, s.turbulent_size_mult)
            } else {
                (1.0, 1.0)
            };
            // (layer, sector, stocks affected, loadings, sd)
            let mut candidates: Vec<(NewsLayer, Option<usize>, Vec<(usize, f64)>, f64)> =
                Vec::new();
            if rng.gen::<f64>() < news.market.events_per_1000_ticks / 1000.0 * rm {
                let aff = (0..n_stocks)
                    .map(|i| (i, cfg.stocks[i].market_loading))
                    .collect();
                candidates.push((NewsLayer::Market, None, aff, news.market.change_sd));
            }
            for sec in 0..cfg.world.sectors.len() {
                if rng.gen::<f64>() < news.sector.events_per_1000_ticks / 1000.0 * rm {
                    let aff = (0..n_stocks)
                        .filter(|&i| sector_of[i] == sec)
                        .map(|i| (i, cfg.stocks[i].sector_loading))
                        .collect();
                    candidates.push((NewsLayer::Sector, Some(sec), aff, news.sector.change_sd));
                }
            }
            for i in 0..n_stocks {
                if rng.gen::<f64>() < news.stock.events_per_1000_ticks / 1000.0 * rm {
                    candidates.push((
                        NewsLayer::Stock,
                        Some(sector_of[i]),
                        vec![(i, 1.0)],
                        news.stock.change_sd,
                    ));
                }
            }
            for (layer, sector, aff, sd) in candidates {
                let x = Normal::new(0.0, sd * sm).unwrap().sample(&mut rng);
                let mut changes = Vec::new();
                let mut sigma = Vec::new();
                for (i, loading) in aff {
                    let multiplicative = cfg.world.dividend_risk.is_some();
                    let base = if multiplicative {
                        div[i] as f64
                    } else {
                        div[i].max(initial[i] / 2).max(1) as f64
                    };
                    // A lognormal shock has no reflecting zero boundary or
                    // initial-dividend-sized rebound. Subtract its variance/2
                    // so the unrounded ordinary shock has unchanged expectation.
                    let mut delta = if multiplicative {
                        let variance = (loading * sd * sm).powi(2);
                        (base * (loading * x - variance / 2.0).exp()).round() as Money - div[i]
                    } else {
                        (base * loading * x).round() as Money
                    };
                    delta = delta.max(min_div - div[i]);
                    if delta != 0 {
                        div[i] += delta;
                        changes.push((i, delta));
                        sigma.push(if multiplicative {
                            base * (loading * sd * sm).powi(2).exp_m1().sqrt()
                        } else {
                            base * loading.abs() * sd * sm
                        });
                        timelines[i].push((t + lead, div[i]));
                    }
                }
                if changes.is_empty() {
                    continue;
                }
                events.push(Event {
                    id: events.len() as u32,
                    sampling_key: events.len() as u64,
                    layer,
                    sector: if layer == NewsLayer::Market {
                        None
                    } else {
                        sector
                    },
                    changes,
                    sigma,
                    first_reveal: t,
                    public_tick: t + public_offset,
                    effective: t + lead,
                });
            }
        }

        // Generate separately: enabling the regime cannot perturb normal shock draws,
        // initial prices, terminal units, or their private disclosure lottery.
        let dragon = dragon::overlay(
            cfg,
            seed,
            &sector_of,
            &turbulent,
            &mut events,
            &mut timelines,
        );

        dividend_risk::overlay(cfg, seed, &sector_of, &mut events, &mut timelines);

        // A privately informed account hears at its first stage and again at every later one.
        let stages = &news.disclosure.stages;
        let public_stage = stages.len() - 1;
        let mut deliveries = vec![Vec::new(); n_accounts];
        for (a, list) in deliveries.iter_mut().enumerate() {
            for ev in &events {
                let u = unit(seed, ev.sampling_key, a as u64);
                let first = stages[..public_stage]
                    .iter()
                    .position(|(_, frac)| u < (*frac * news_multipliers[a]).min(1.0));
                if let Some(k) = first {
                    for (j, (off, _)) in stages.iter().enumerate().take(public_stage).skip(k) {
                        let tick = ev.first_reveal + off;
                        if tick < ev.public_tick {
                            list.push(Delivery {
                                tick,
                                event: ev.id,
                                stage: j,
                                forecast: false,
                            });
                        }
                    }
                }
                list.push(Delivery {
                    tick: ev.public_tick,
                    event: ev.id,
                    stage: public_stage,
                    forecast: false,
                });
            }
            forecast::schedule(cfg, seed, a, forecasts[a], &events, list);
            list.sort_by_key(|d| (d.tick, d.event, d.stage));
        }

        World {
            dragon,
            turbulent,
            season_changes,
            rate_mult: s.turbulent_rate_mult,
            size_mult: s.turbulent_size_mult,
            events,
            dividend_cache: cache_dividends(&initial, &timelines, ticks),
            timelines,
            initial_dividends: initial,
            deliveries,
            news_multipliers: news_multipliers.to_vec(),
            forecast_access: forecasts.to_vec(),
            opening_prices: cfg
                .stocks
                .iter()
                .enumerate()
                .map(|(i, st)| match cfg.market.opening_spread {
                    None => st.initial_price,
                    Some(spread) => {
                        let (lo, hi) = cfg.terminal_payments_range();
                        // In-game payments expected; integer for a fixed length, as before.
                        let in_game = if cfg.scoring.random_end.is_some() {
                            (cfg.expected_ticks() - 1.0).max(0.0) / cfg.world.dividend_period as f64
                        } else {
                            ((ticks.saturating_sub(1)) / cfg.world.dividend_period) as f64
                        };
                        let payments = in_game
                            + (lo + hi) as f64 / 2.0
                                * (cfg.terminal_payment_ticks() / cfg.world.dividend_period) as f64;
                        let u = unit(seed ^ 0x09e7, i as u64, 1) * 2.0 - 1.0;
                        let t = cfg.market.tick_size.max(1);
                        let p = st.dividend as f64 / cfg.world.dividend_scale as f64
                            * payments
                            * (1.0 + spread * u);
                        ((p / t as f64).round() as Money * t).max(t)
                    }
                })
                .collect(),
            terminal_payments: {
                let (lo, hi) = cfg.terminal_payments_range();
                lo + (unit(seed ^ 0x7e57_1a1, 0, 0) * (hi - lo + 1) as f64) as i64
            }
            .min(cfg.terminal_payments_range().1),
            terminal_hints: cfg.terminal_hints(),
            seed,
            range_widths: news.disclosure.range_widths.clone(),
            public_stage,
        }
    }

    /// Number of terminal-payment hints due by `tick`.
    pub fn hints_due(&self, tick: Tick) -> usize {
        self.terminal_hints.iter().filter(|h| h.0 <= tick).count()
    }

    /// Hint `k` for `account`: a range of the given width containing the true number of
    /// terminal payments, the truth at a random position.
    pub fn terminal_hint(&self, account: usize, k: usize) -> (i64, i64) {
        let w = self.terminal_hints[k].1;
        let below = (unit(self.seed ^ 0x41d7, account as u64, k as u64) * (w + 1) as f64) as i64;
        let low = self.terminal_payments - below.min(w);
        (low, low + w)
    }

    /// A news item as `account` sees it at disclosure `stage`: a range that contains the true
    /// change, with width from the stage and the truth at a random position, or the exact
    /// change at the public stage.
    pub fn view(&self, event: u32, account: usize, stage: usize) -> NewsItem {
        let ev = &self.events[event as usize];
        let public = stage >= self.public_stage;
        let changes = ev
            .changes
            .iter()
            .zip(&ev.sigma)
            .enumerate()
            .map(|(k, (&(stock, delta), &sigma))| {
                if public {
                    return Change {
                        stock,
                        low: delta,
                        high: delta,
                    };
                }
                let width = (self.range_widths[stage] * sigma).round() as Money;
                let salt = (account as u64) << 32 | (stage as u64) << 16 | k as u64;
                let below = (unit(self.seed ^ 0x7a9e_5b1d, ev.sampling_key, salt)
                    * (width + 1) as f64) as Money;
                let low = delta - below.min(width);
                Change {
                    stock,
                    low,
                    high: low + width,
                }
            })
            .collect();
        NewsItem {
            id: ev.id,
            layer: ev.layer,
            sector: ev.sector,
            changes,
            stage,
            forecast: false,
            first_reveal_tick: ev.first_reveal,
            public_tick: ev.public_tick,
            effective_tick: ev.effective,
            public,
        }
    }

    pub fn dividend(&self, stock: usize, tick: Tick) -> Money {
        if let Some(value) = self
            .dividend_cache
            .get(stock)
            .and_then(|values| values.get(tick as usize))
        {
            return *value;
        }
        let tl = &self.timelines[stock];
        let i = tl.partition_point(|(t, _)| *t <= tick);
        if i == 0 {
            self.initial_dividends[stock]
        } else {
            tl[i - 1].1
        }
    }

    pub fn dividends(&self, tick: Tick) -> Vec<Money> {
        (0..self.timelines.len())
            .map(|i| self.dividend(i, tick))
            .collect()
    }

    pub fn season(&self, tick: Tick) -> Season {
        let turbulent = self.turbulent.get(tick as usize).copied().unwrap_or(false);
        Season {
            turbulent,
            rate_mult: if turbulent { self.rate_mult } else { 1.0 },
            size_mult: if turbulent { self.size_mult } else { 1.0 },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn news_access_changes_only_selected_accounts_private_delivery_probability() {
        let cfg = Config::load(std::path::Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../scenarios/baseline.toml"
        )))
        .unwrap();
        let ordinary = World::generate(&cfg, 7, 3);
        let advantaged = World::generate_with_news_access(&cfg, 7, &[1.0, 10.0, 1.0]);
        assert_eq!(
            format!("{:?}", ordinary.events),
            format!("{:?}", advantaged.events)
        );
        assert_eq!(ordinary.timelines, advantaged.timelines);
        assert_eq!(ordinary.opening_prices, advantaged.opening_prices);
        assert_eq!(ordinary.turbulent, advantaged.turbulent);
        assert_eq!(ordinary.terminal_payments, advantaged.terminal_payments);
        for a in 0..3 {
            for k in 0..ordinary.terminal_hints.len() {
                assert_eq!(ordinary.terminal_hint(a, k), advantaged.terminal_hint(a, k));
            }
            if a != 1 {
                assert_eq!(ordinary.deliveries[a], advantaged.deliveries[a]);
            }
        }
        let before = &ordinary.deliveries[1];
        let after = &advantaged.deliveries[1];
        assert!(after.len() > before.len());
        for d in before {
            assert!(after.contains(d));
            assert_eq!(
                ordinary.view(d.event, 1, d.stage).changes,
                advantaged.view(d.event, 1, d.stage).changes
            );
        }
        let public = |xs: &[Delivery]| {
            xs.iter()
                .filter(|d| d.stage == ordinary.public_stage)
                .copied()
                .collect::<Vec<_>>()
        };
        assert_eq!(public(before), public(after));
        let all = World::generate_with_news_access(&cfg, 7, &[20.0, 0.0]);
        assert_eq!(
            all.deliveries[0].iter().filter(|d| d.stage == 0).count(),
            all.events.len()
        );
        assert!(all.deliveries[1]
            .iter()
            .all(|d| d.stage == all.public_stage));
    }

    #[test]
    fn news_access_rejects_invalid_config_and_defaults_to_one() {
        let mut cfg = Config::load(std::path::Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../scenarios/baseline.toml"
        )))
        .unwrap();
        assert!(cfg.bots.iter().all(|g| g.news_multiplier == 1.0));
        for x in [f64::NAN, f64::INFINITY, -1.0] {
            cfg.bots[0].news_multiplier = x;
            assert!(cfg.validate().is_err());
        }
    }

    #[test]
    fn cached_dividends_match_timeline_at_boundaries_and_in_arbitrary_order() {
        let initial = [10, 20, 30];
        let timelines = vec![
            vec![(0, 11), (3, 15), (3, 17), (7, 12), (9, 50)],
            vec![],
            vec![(20, 99)],
        ];
        let cache = cache_dividends(&initial, &timelines, 9);
        for tick in [8, 0, 3, 2, 7, 6, 1, 5, 4] {
            for stock in 0..initial.len() {
                let expected = timelines[stock]
                    .iter()
                    .rev()
                    .find(|(t, _)| *t <= tick)
                    .map_or(initial[stock], |(_, value)| *value);
                assert_eq!(cache[stock][tick as usize], expected);
            }
        }
        assert!(cache_dividends(&initial, &timelines, Tick::MAX).is_empty());
    }

    #[test]
    fn generated_world_cache_and_sparse_fallback_agree_including_future_ticks() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../scenarios/baseline.toml");
        let cfg = Config::load(std::path::Path::new(path)).unwrap();
        let mut world = World::generate(&cfg, 7, 2);
        let cache = world.dividend_cache.clone();
        assert!(!cache.is_empty());
        for tick in (0..cfg.ticks + cfg.world.news.disclosure.lead_ticks + 1).rev() {
            for stock in 0..cfg.stocks.len() {
                let expected = world.timelines[stock]
                    .iter()
                    .rev()
                    .find(|(t, _)| *t <= tick)
                    .map_or(world.initial_dividends[stock], |(_, value)| *value);
                assert_eq!(world.dividend(stock, tick), expected);
            }
        }
        world.dividend_cache.clear();
        for (stock, values) in cache.iter().enumerate() {
            for (tick, &expected) in values.iter().enumerate() {
                assert_eq!(world.dividend(stock, tick as Tick), expected);
            }
        }
    }
}
