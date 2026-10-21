//! Phased log cash-flow growth. A stochastic business path, never a price path.
//! The selected business differs only after the configured disclosure boundary.
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
    let tr = &cfg.world.dragon.trend;
    let lead = cfg.world.news.disclosure.lead_ticks;
    let end = cfg.ticks.saturating_sub(lead + 1);
    let duration = cfg.world_duration();
    let start = cfg.world.dragon.start_fraction;
    let mut episodes = Vec::new();
    for stock in 0..cfg.stocks.len() {
        let mut rng = ChaCha8Rng::seed_from_u64(seed ^ mix(stock as u64) ^ 0xd2a8_b051);
        // Draw these for EVERY stock, so selected identity never shifts its RNG stream.
        let peak_log = rng.gen_range(tr.growth_log_bias.0..=tr.growth_log_bias.1);
        let peak_at = rng.gen_range(0.87..0.97);
        let dip_at = rng.gen_range(0.67..0.81);
        let dip_width = rng.gen_range(0.035..0.07);
        let dip_depth = rng.gen_range(0.30..0.55);
        let early = rng.gen_range(0.60..0.95);
        let mut phase_end = 0;
        let mut phase_start = 0;
        let mut from = 0.0;
        let mut to = 0.0;
        let mut business = 0.0;
        let mut released: f64 = 0.0;
        let mut shock: f64 = 0.0;
        let mut previous = 0;
        let mut step = 0u64;
        let initial = cfg.stocks[stock].dividend;
        let normal = Normal::new(0.0, 1.0).unwrap();
        for tick in 0..=end {
            if tick >= phase_end {
                from = business;
                to = rng.gen_range(-0.35..0.55);
                phase_start = tick;
                phase_end = tick + (duration * rng.gen_range(0.025..0.09)).max(1.0) as Tick;
            }
            let w = (tick - phase_start) as f64 / (phase_end - phase_start) as f64;
            business = from + (to - from) * w;
            let x = tick as f64 / duration;
            let early_path = if candidates.contains(&stock) {
                early * (x / start).min(1.0)
            } else {
                0.25
            };
            let growth = if tick >= divergence && selected == Some(stock) {
                let progress = ((x - start) / (peak_at - start)).clamp(0.0, 1.0);
                early + (peak_log - early) * progress
                    - dip_depth * (1.0 - ((x - dip_at) / dip_width).abs()).max(0.0)
            } else {
                early_path
            };
            let storm = turbulent[tick as usize];
            let rm = if storm {
                cfg.world.seasons.turbulent_rate_mult
            } else {
                1.0
            };
            if rng.gen::<f64>() >= tr.events_per_1000_ticks / 1000.0 * rm {
                continue;
            }
            let sm = if storm {
                cfg.world.seasons.turbulent_size_mult
            } else {
                1.0
            };
            shock = 0.75 * shock + normal.sample(&mut rng) * tr.step_sd * sm;
            let setback = rng.gen::<f64>() < 0.055;
            if setback {
                shock -= rng.gen_range(0.12..0.28);
            }
            let desired = (growth + business + shock).clamp(0.0, 3.2);
            let cap = tr.max_step * rng.gen_range(0.6..1.0);
            released += (desired - released).clamp(if setback { -0.28 } else { -cap }, cap);
            let extra = (initial as f64 * released.exp_m1().max(0.0)).round() as Money;
            let delta = extra - previous;
            previous = extra;
            episodes.push(DragonEpisode {
                stock,
                first_reveal: tick,
                effective: tick + lead,
                extra_rate: extra,
            });
            let key = 0xd2a8_0000_0000_0000 | (stock as u64) << 32 | step;
            step += 1;
            if delta == 0 {
                continue;
            }
            let tl = &base_timelines[stock];
            let pos = tl.partition_point(|(t, _)| *t <= tick + lead);
            let base = if pos == 0 { initial } else { tl[pos - 1].1 };
            // Scale uncertainty like ordinary shocks at the current cash-flow level.
            // It reveals neither selected identity nor the event's true delta.
            let sigma =
                (base + extra).max(initial / 2).max(1) as f64 * cfg.world.news.stock.change_sd * sm;
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
