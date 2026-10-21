//! Persistent dividend cuts and suspensions, expressed solely as ordinary news.
//! Independent of accounts, prices, orders and the selected growth stock.
use super::{mix, Config, Event};
use bot_api::{Money, NewsLayer, Tick};
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;

#[derive(Clone, Copy)]
enum State {
    Paying,
    Impaired,
    Suspended,
}

struct Release {
    tick: Tick,
    stock: usize,
    retention: f64,
    key: u64,
}

fn releases(cfg: &Config, seed: u64) -> Vec<Release> {
    let r = cfg.world.dividend_risk.as_ref().unwrap();
    let duration = cfg.ticks as f64;
    let mut result = Vec::new();
    for stock in 0..cfg.stocks.len() {
        let mut rng = ChaCha8Rng::seed_from_u64(seed ^ mix(stock as u64) ^ 0xd171_d3ad);
        let mut tick: Tick = 0;
        let mut state = State::Paying;
        let mut retention = 1.0;
        let mut anchor = 1.0;
        let mut step = 0u32;
        loop {
            let spacing = (duration * rng.gen_range(r.review_fraction.0..=r.review_fraction.1))
                .round()
                .max(1.0) as Tick;
            let Some(next) = tick.checked_add(spacing).filter(|t| *t < cfg.ticks) else {
                break;
            };
            tick = next;
            let u = rng.gen::<f64>();
            let low = rng.gen_range(r.impaired_retention.0..=r.impaired_retention.1);
            let recovery = rng.gen_range(r.recovered_retention.0..=r.recovered_retention.1);
            let target = if u < r.suspension_probability {
                if matches!(state, State::Paying) {
                    anchor = retention;
                }
                state = State::Suspended;
                0.0
            } else {
                match state {
                    State::Paying if u < r.suspension_probability + r.impairment_probability => {
                        anchor = retention;
                        state = State::Impaired;
                        anchor * low
                    }
                    State::Impaired if u < r.suspension_probability + r.recovery_probability => {
                        state = State::Paying;
                        anchor * recovery
                    }
                    State::Suspended if u < r.suspension_probability + r.recovery_probability => {
                        state = State::Impaired;
                        anchor * low
                    }
                    _ => retention,
                }
            };
            // A review does not guarantee a change or a recovery. Smooth restarts
            // and cuts avoid a single transient payment becoming the whole terminal value.
            for part in 1..=4 {
                let offset = (duration * r.adjustment_fraction * part as f64 / 4.0).round() as Tick;
                if let Some(at) = tick.checked_add(offset).filter(|t| *t < cfg.ticks) {
                    result.push(Release {
                        tick: at,
                        stock,
                        retention: if part == 4 {
                            target
                        } else {
                            retention + (target - retention) * part as f64 / 4.0
                        },
                        key: 0xd171_0000_0000_0000 | (stock as u64) << 32 | u64::from(step),
                    });
                }
                step += 1;
            }
            retention = target;
        }
    }
    result.sort_by_key(|r| (r.tick, r.stock, r.key));
    result
}

pub(super) fn overlay(
    cfg: &Config,
    seed: u64,
    sector_of: &[usize],
    events: &mut Vec<Event>,
    timelines: &mut [Vec<(Tick, Money)>],
) {
    if cfg.world.dividend_risk.is_none() {
        return;
    }
    let updates = releases(cfg, seed);
    let base_events = std::mem::take(events);
    let mut base: Vec<_> = cfg.stocks.iter().map(|s| s.dividend).collect();
    let mut paid = base.clone();
    let mut retention = vec![1.0; base.len()];
    let mut updates = updates.into_iter().peekable();
    let mut base_events = base_events.into_iter().peekable();
    let lead = cfg.world.news.disclosure.lead_ticks;
    let public_offset = cfg.world.news.disclosure.stages.last().unwrap().0;
    for timeline in timelines.iter_mut() {
        timeline.clear();
    }
    while base_events.peek().is_some() || updates.peek().is_some() {
        // Preserve the legacy ordering of simultaneous base events, then apply
        // retention releases. Every emitted delta is rebased against actual payout.
        let base_first = base_events
            .peek()
            .is_some_and(|e| updates.peek().is_none_or(|r| e.first_reveal <= r.tick));
        let mut event = if base_first {
            let mut event = base_events.next().unwrap();
            let mut changes = Vec::new();
            let mut sigma = Vec::new();
            for (&(stock, delta), &uncertainty) in event.changes.iter().zip(&event.sigma) {
                base[stock] += delta;
                let next = (base[stock] as f64 * retention[stock]).round() as Money;
                let change = next - paid[stock];
                if change != 0 {
                    changes.push((stock, change));
                    sigma.push(uncertainty * retention[stock]);
                    paid[stock] = next;
                }
            }
            event.changes = changes;
            event.sigma = sigma;
            event
        } else {
            let release = updates.next().unwrap();
            let stock = release.stock;
            retention[stock] = release.retention;
            let next = (base[stock] as f64 * release.retention).round() as Money;
            let delta = next - paid[stock];
            let sigma = (paid[stock].max(next) as f64 * cfg.world.news.stock.change_sd)
                .max(delta.abs() as f64 * 0.5);
            paid[stock] = next;
            Event {
                id: 0,
                sampling_key: release.key,
                layer: NewsLayer::Stock,
                sector: Some(sector_of[stock]),
                changes: if delta == 0 {
                    vec![]
                } else {
                    vec![(stock, delta)]
                },
                sigma: vec![sigma],
                first_reveal: release.tick,
                public_tick: release.tick + public_offset,
                effective: release.tick + lead,
            }
        };
        if event.changes.is_empty() {
            continue;
        }
        event.id = events.len() as u32;
        for &(stock, _) in &event.changes {
            timelines[stock].push((event.effective, paid[stock]));
        }
        events.push(event);
    }
}
