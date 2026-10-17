//! Hidden world: market seasons, three-layer news and the dividend path.
//!
//! Everything is generated from the seed before trading starts. Bots only
//! see news once it is delivered to them and season changes as they happen.

use crate::config::Config;
use bot_api::{Money, NewsItem, NewsLayer, Season, Tick};
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use rand_distr::{Distribution, Normal};

#[derive(Clone, Debug)]
pub struct Event {
    pub id: u32,
    pub layer: NewsLayer,
    pub sector: Option<usize>,
    pub changes: Vec<(usize, Money)>,
    pub first_reveal: Tick,
    pub public_tick: Tick,
    pub effective: Tick,
}

impl Event {
    pub fn view(&self, public: bool) -> NewsItem {
        NewsItem {
            id: self.id,
            layer: self.layer,
            sector: self.sector,
            changes: self.changes.clone(),
            first_reveal_tick: self.first_reveal,
            public_tick: self.public_tick,
            effective_tick: self.effective,
            public,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Delivery {
    pub tick: Tick,
    pub event: u32,
    pub public: bool,
}

pub struct World {
    pub turbulent: Vec<bool>,
    pub season_changes: Vec<(Tick, bool)>,
    pub rate_mult: f64,
    pub size_mult: f64,
    pub events: Vec<Event>,
    /// Per stock: (effective tick, new dividend), ascending.
    pub timelines: Vec<Vec<(Tick, Money)>>,
    pub initial_dividends: Vec<Money>,
    /// Per account, ascending by tick.
    pub deliveries: Vec<Vec<Delivery>>,
}

fn mix(mut x: u64) -> u64 {
    x ^= x >> 30;
    x = x.wrapping_mul(0xbf58476d1ce4e5b9);
    x ^= x >> 27;
    x = x.wrapping_mul(0x94d049bb133111eb);
    x ^ (x >> 31)
}

fn unit(seed: u64, a: u64, b: u64) -> f64 {
    (mix(seed ^ mix(a.wrapping_mul(0x9e3779b97f4a7c15) ^ mix(b))) >> 11) as f64
        / (1u64 << 53) as f64
}

impl World {
    pub fn generate(cfg: &Config, seed: u64, n_accounts: usize) -> World {
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
                for (i, loading) in aff {
                    let base = div[i].max(initial[i] / 2).max(1) as f64;
                    let mut delta = (base * loading * x).round() as Money;
                    delta = delta.max(min_div - div[i]);
                    if delta != 0 {
                        div[i] += delta;
                        changes.push((i, delta));
                        timelines[i].push((t + lead, div[i]));
                    }
                }
                if changes.is_empty() {
                    continue;
                }
                events.push(Event {
                    id: events.len() as u32,
                    layer,
                    sector: if layer == NewsLayer::Market {
                        None
                    } else {
                        sector
                    },
                    changes,
                    first_reveal: t,
                    public_tick: t + public_offset,
                    effective: t + lead,
                });
            }
        }

        let mut deliveries = vec![Vec::new(); n_accounts];
        for (a, list) in deliveries.iter_mut().enumerate() {
            for ev in &events {
                let u = unit(seed, ev.id as u64, a as u64);
                let private = news
                    .disclosure
                    .stages
                    .iter()
                    .find(|(_, frac)| *frac < 1.0 && u < *frac)
                    .map(|(off, _)| ev.first_reveal + off);
                if let Some(pt) = private {
                    if pt < ev.public_tick {
                        list.push(Delivery {
                            tick: pt,
                            event: ev.id,
                            public: false,
                        });
                    }
                }
                list.push(Delivery {
                    tick: ev.public_tick,
                    event: ev.id,
                    public: true,
                });
            }
            list.sort_by_key(|d| (d.tick, d.event));
        }

        World {
            turbulent,
            season_changes,
            rate_mult: s.turbulent_rate_mult,
            size_mult: s.turbulent_size_mult,
            events,
            timelines,
            initial_dividends: initial,
            deliveries,
        }
    }

    pub fn dividend(&self, stock: usize, tick: Tick) -> Money {
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
