//! A hidden cash-flow regime overlaid on otherwise identical normal news.
//! All amounts are nonnegative extra dividend rates, not price targets.

use super::{mix, unit, Event};
use crate::config::Config;
use bot_api::{DragonModel, Money, NewsLayer, Tick};
use rand::{seq::SliceRandom, Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use serde::Serialize;

mod growth;
mod trend;

#[derive(Clone, Debug, Serialize)]
pub struct DragonEpisode {
    pub stock: usize,
    pub first_reveal: Tick,
    pub effective: Tick,
    pub extra_rate: Money,
}

/// Post-game ground truth only; never expose this through the bot API.
#[derive(Clone, Debug, Serialize)]
pub struct DragonOutcome {
    pub model: DragonModel,
    pub candidates: Vec<usize>,
    pub selected_stock: Option<usize>,
    pub divergence_tick: Tick,
    pub episodes: Vec<DragonEpisode>,
}

pub(super) fn overlay(
    cfg: &Config,
    seed: u64,
    sector_of: &[usize],
    turbulent: &[bool],
    events: &mut Vec<Event>,
    timelines: &mut [Vec<(Tick, Money)>],
) -> Option<DragonOutcome> {
    let d = &cfg.world.dragon;
    if !d.enabled {
        return None;
    }
    let lead = cfg.world.news.disclosure.lead_ticks;
    let ordinary = events.len();
    let divergence = cfg.dragon_start_tick();
    let last_reveal = cfg.ticks.saturating_sub(lead + 1);
    // Config::validate rejects horizons too short for the regime.
    let span = last_reveal.saturating_sub(divergence);
    let mut rng = ChaCha8Rng::seed_from_u64(seed ^ 0xd2a6_0ca7_d1da);
    let mut candidates: Vec<_> = (0..cfg.stocks.len()).collect();
    candidates.shuffle(&mut rng);
    candidates.truncate(d.candidates);
    let selected_stock = (unit(seed ^ 0xd2a6_a991, 0, 0) < d.probability)
        .then(|| candidates[(unit(seed ^ 0xd2a6_a991, 0, 1) * candidates.len() as f64) as usize]);
    let mut episodes = Vec::new();
    if d.model == DragonModel::Trend || d.model == DragonModel::Growth {
        let generator = if d.model == DragonModel::Growth {
            growth::events
        } else {
            trend::events
        };
        episodes = generator(
            cfg,
            seed,
            sector_of,
            turbulent,
            &candidates,
            selected_stock,
            divergence,
            events,
            timelines,
        );
    } else {
        for &stock in &candidates {
            // Draw a full candidate path regardless of whether it becomes the winner.
            // Changing probability leaves all pre-divergence information identical.
            let mut rng = ChaCha8Rng::seed_from_u64(seed ^ mix(stock as u64) ^ 0xd2a6_9a74);
            let early_ticks = [(0.20, 0.27), (0.33, 0.39), (0.44, 0.49)]
                .map(|(lo, hi)| (cfg.ticks as f64 * rng.gen_range(lo..hi)) as Tick);
            let late_ticks = [
                (0.03, 0.10),
                (0.23, 0.33),
                (0.44, 0.54),
                (0.63, 0.74),
                (0.85, 0.96),
            ]
            .map(|(lo, hi)| divergence + (span as f64 * rng.gen_range(lo..hi)) as Tick);
            let early = [
                rng.gen_range(0.20..0.35),
                rng.gen_range(0.40..0.65),
                rng.gen_range(0.15..0.35),
            ];
            let peak = rng.gen_range(d.peak_extra.0..=d.peak_extra.1);
            let winner = [
                peak * 0.4,
                peak,
                peak * rng.gen_range(0.35..0.55),
                peak * rng.gen_range(0.70..0.95),
                peak * rng.gen_range(0.50..0.95),
            ];
            let fade = [
                early[2] * 0.9,
                early[2] * 0.5,
                early[2] * 0.3,
                early[2] * 0.1,
                0.0,
            ];
            let late = if selected_stock == Some(stock) {
                winner
            } else {
                fade
            };
            let initial = cfg.stocks[stock].dividend as f64;
            let mut previous = 0;
            for (step, (tick, multiple)) in early_ticks
                .into_iter()
                .chain(late_ticks)
                .zip(early.into_iter().chain(late))
                .enumerate()
            {
                let extra = (initial * multiple).round() as Money;
                let delta = extra - previous;
                previous = extra;
                episodes.push(DragonEpisode {
                    stock,
                    first_reveal: tick,
                    effective: tick + lead,
                    extra_rate: extra,
                });
                if delta == 0 {
                    continue;
                }
                events.push(Event {
                    id: 0, // Assigned chronologically after merging normal and regime news.
                    sampling_key: 0xd2a6_0000_0000_0000 | (stock as u64) << 16 | step as u64,
                    layer: NewsLayer::Stock,
                    sector: Some(sector_of[stock]),
                    changes: vec![(stock, delta)],
                    sigma: vec![
                        (delta.abs() as f64 * 0.5).max(initial * cfg.world.news.stock.change_sd)
                    ],
                    first_reveal: tick,
                    public_tick: tick + cfg.world.news.disclosure.stages.last().unwrap().0,
                    effective: tick + lead,
                });
            }
        }
    }
    scale_ordinary_sigma(cfg, &mut events[..ordinary], &episodes);
    events.sort_by_key(|e| e.first_reveal);
    for (id, event) in events.iter_mut().enumerate() {
        event.id = id as u32;
    }
    // Normal deltas were calculated against the independent normal path. Adding a
    // nonnegative extra component preserves its floor, including during pullbacks.
    let mut dividend: Vec<_> = cfg.stocks.iter().map(|s| s.dividend).collect();
    for timeline in timelines.iter_mut() {
        timeline.clear();
    }
    for event in events.iter() {
        for &(stock, delta) in &event.changes {
            dividend[stock] += delta;
            timelines[stock].push((event.effective, dividend[stock]));
        }
    }
    Some(DragonOutcome {
        model: d.model,
        candidates,
        selected_stock,
        divergence_tick: divergence,
        episodes,
    })
}

/// Ordinary news uncertainty was sized from the ordinary path alone. Regime news is
/// sized from ordinary plus extra, so a private range on ordinary news would reveal how
/// much of a stock's dividend is regime extra. Size both from the whole cash flow known
/// at the reveal tick.
fn scale_ordinary_sigma(cfg: &Config, events: &mut [Event], episodes: &[DragonEpisode]) {
    let initial: Vec<Money> = cfg.stocks.iter().map(|s| s.dividend).collect();
    let multiplicative = cfg.world.dividend_risk.is_some();
    let mut extras = vec![Vec::new(); initial.len()];
    for e in episodes {
        extras[e.stock].push((e.first_reveal, e.extra_rate));
    }
    for x in &mut extras {
        x.sort_by_key(|(t, _)| *t);
    }
    // Replays the ordinary path in generation order, as the generator sized each change.
    let mut dividend = initial.clone();
    for event in events.iter_mut() {
        for (&(stock, delta), sigma) in event.changes.iter().zip(event.sigma.iter_mut()) {
            let known = &extras[stock];
            let pos = known.partition_point(|(t, _)| *t <= event.first_reveal);
            let extra = if pos == 0 { 0 } else { known[pos - 1].1 };
            // Multiplicative shocks (dividend risk) are sized from the current rate with no floor.
            let floor = if multiplicative {
                1
            } else {
                (initial[stock] / 2).max(1)
            };
            let base = dividend[stock].max(floor);
            let total = (dividend[stock] + extra).max(floor);
            *sigma *= total as f64 / base as f64;
            dividend[stock] += delta;
        }
    }
}
