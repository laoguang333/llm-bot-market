//! Irregular small news updates driven by changing hidden business conditions.
//! All stocks use the same process; selected growth changes persistence, not a
//! fixed schedule or a promised dividend target. Prices are never prescribed.

use super::{mix, Config, DragonEpisode, Event};
use bot_api::{Money, NewsLayer, Tick};
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use rand_distr::{Distribution, Normal};

#[allow(clippy::too_many_arguments)]
pub(super) fn events(
    cfg: &Config,
    seed: u64,
    sector_of: &[usize],
    turbulent: &[bool],
    candidates: &[usize],
    selected: Option<usize>,
    divergence: Tick,
    events: &mut Vec<Event>,
    base_timelines: &[Vec<(Tick, Money)>],
) -> Vec<DragonEpisode> {
    let trend = &cfg.world.dragon.trend;
    let lead = cfg.world.news.disclosure.lead_ticks;
    let last_reveal = cfg.ticks.saturating_sub(lead + 1);
    let duration = cfg.world_duration();
    let mut episodes = Vec::new();
    for stock in 0..cfg.stocks.len() {
        let mut rng = ChaCha8Rng::seed_from_u64(seed ^ mix(stock as u64) ^ 0xd2a7_b051);
        let growth = rng.gen_range(trend.growth_log_bias.0..=trend.growth_log_bias.1);
        let mut next_regime = 0;
        let mut strength = 0.0;
        let mut latent: f64 = 0.0;
        let mut disclosed: f64 = 0.0;
        let mut previous = 0;
        let mut step = 0u64;
        let initial = cfg.stocks[stock].dividend;
        let normal = Normal::new(0.0, 1.0).unwrap();
        for tick in 0..=last_reveal {
            if tick >= next_regime {
                strength = rng.gen_range(-2.0..2.0);
                let width = rng.gen_range(0.035..0.10) * duration;
                next_regime = tick.saturating_add((width as Tick).max(1));
            }
            let bias = if tick < divergence && candidates.contains(&stock) {
                0.65 * growth
            } else if tick >= divergence && selected == Some(stock) {
                1.35 * growth
            } else {
                0.0
            };
            latent += (bias + strength) / duration;
            let storm = turbulent.get(tick as usize).copied().unwrap_or(false);
            let rm = if storm {
                cfg.world.seasons.turbulent_rate_mult
            } else {
                1.0
            };
            if rng.gen::<f64>() >= trend.events_per_1000_ticks / 1000.0 * rm {
                continue;
            }
            let sm = if storm {
                cfg.world.seasons.turbulent_size_mult
            } else {
                1.0
            };
            latent += normal.sample(&mut rng) * trend.step_sd * sm;
            // Shared by every stock. Rare sharp setbacks can interrupt a good trend.
            let setback = rng.gen::<f64>() < 0.025;
            if setback {
                latent -= rng.gen_range(0.06..0.13);
            }
            latent = latent.clamp(-2.0, 4.0);
            // Vary the limit as well as the underlying change, avoiding a repeated
            // exact increment. Unreleased growth is not delivered to any account.
            let cap = trend.max_step * rng.gen_range(0.45..1.0);
            disclosed += (latent - disclosed).clamp(if setback { -0.13 } else { -cap }, cap);
            let extra = (initial as f64 * disclosed.exp_m1().max(0.0)).round() as Money;
            let delta = extra - previous;
            previous = extra;
            episodes.push(DragonEpisode {
                stock,
                first_reveal: tick,
                effective: tick + lead,
                extra_rate: extra,
            });
            let key = 0xd2a7_0000_0000_0000 | (stock as u64) << 32 | step;
            step += 1;
            if delta == 0 {
                continue;
            }
            // Ordinary stock-news uncertainty, independent of this delta's exact
            // magnitude. The old pulse abs(delta)/2 width was an avoidable clue.
            let timeline = &base_timelines[stock];
            let pos = timeline.partition_point(|(t, _)| *t <= tick + lead);
            let base = if pos == 0 {
                initial
            } else {
                timeline[pos - 1].1
            };
            let sigma = base.max(initial / 2).max(1) as f64 * cfg.world.news.stock.change_sd * sm;
            events.push(Event {
                id: 0,
                sampling_key: key,
                layer: NewsLayer::Stock,
                sector: Some(sector_of[stock]),
                changes: vec![(stock, delta)],
                sigma: vec![sigma],
                first_reveal: tick,
                public_tick: tick + cfg.world.news.disclosure.stages.last().unwrap().0,
                effective: tick + lead,
            });
        }
    }
    episodes
}
