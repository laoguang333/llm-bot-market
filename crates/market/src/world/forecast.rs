//! Additional research has independent lotteries and never changes the world.
use super::{unit, Config, Delivery, Event, ForecastAccess, World};
use bot_api::{Change, NewsItem};

pub(super) fn schedule(
    cfg: &Config,
    seed: u64,
    account: usize,
    access: ForecastAccess,
    events: &[Event],
    list: &mut Vec<Delivery>,
) {
    let lead = cfg.world.news.disclosure.lead_ticks;
    if access.horizon <= lead || access.coverage <= 0.0 {
        return;
    }
    let cutoff = (cfg.ticks as u64 * 55 / 100) as u32;
    for e in events {
        if e.effective >= cfg.ticks
            || unit(seed ^ 0xf0ca_57, e.sampling_key, account as u64) >= access.coverage
        {
            continue;
        }
        let ahead = lead
            + 1
            + (unit(seed ^ 0xf0ca_58, e.sampling_key, account as u64)
                * (access.horizon - lead) as f64) as u32;
        let mut tick = e.effective.saturating_sub(ahead);
        // All future events, including ordinary ones, obey this embargo. Otherwise
        // even merged event numbering could reveal winner-dependent future paths.
        if cfg.world.dragon.enabled && e.first_reveal >= cutoff {
            tick = tick.max(cutoff);
        }
        if tick < e.first_reveal {
            list.push(Delivery {
                tick,
                event: e.id,
                stage: 0,
                forecast: true,
            });
        }
    }
}

impl World {
    pub fn delivery_view(&self, delivery: Delivery, account: usize) -> NewsItem {
        if !delivery.forecast {
            return self.view(delivery.event, account, delivery.stage);
        }
        let ev = &self.events[delivery.event as usize];
        let changes = ev
            .changes
            .iter()
            .zip(&ev.sigma)
            .enumerate()
            .map(|(k, (&(stock, delta), &sigma))| {
                let width = (self.forecast_access[account].range_width * sigma).round() as i64;
                let below = (unit(
                    self.seed ^ 0xf0ca_59,
                    ev.sampling_key,
                    (account as u64) << 32 | k as u64,
                ) * (width + 1) as f64) as i64;
                Change {
                    stock,
                    low: delta - below.min(width),
                    high: delta - below.min(width) + width,
                }
            })
            .collect();
        NewsItem {
            id: ev.id,
            layer: ev.layer,
            sector: ev.sector,
            changes,
            stage: 0,
            forecast: true,
            first_reveal_tick: ev.first_reveal,
            public_tick: ev.public_tick,
            effective_tick: ev.effective,
            public: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn config() -> Config {
        Config::load(std::path::Path::new("../../scenarios/baseline-v8.toml")).unwrap()
    }
    #[test]
    fn research_respects_time_truth_and_does_not_change_other_accounts_or_world() {
        let cfg = config();
        let plain = World::generate(&cfg, 71, 2);
        let access = ForecastAccess {
            horizon: 1000,
            coverage: 0.6,
            range_width: 3.0,
        };
        let informed = World::generate_with_information(
            &cfg,
            71,
            &[1., 1.],
            &[access, ForecastAccess::default()],
        );
        assert_eq!(plain.timelines, informed.timelines);
        assert_eq!(plain.deliveries[1], informed.deliveries[1]);
        assert_eq!(
            plain.deliveries[0],
            informed.deliveries[0]
                .iter()
                .filter(|d| !d.forecast)
                .copied()
                .collect::<Vec<_>>()
        );
        let mut count = 0;
        for &d in &informed.deliveries[0] {
            if !d.forecast {
                continue;
            }
            count += 1;
            let e = &informed.events[d.event as usize];
            assert!(d.tick < e.first_reveal && e.effective - d.tick <= access.horizon);
            let v = informed.delivery_view(d, 0);
            assert!(v.forecast && !v.public);
            for (c, &(_, truth)) in v.changes.iter().zip(&e.changes) {
                assert!(c.low <= truth && truth <= c.high);
            }
        }
        assert!(count > 100);
    }
    #[test]
    fn even_exact_long_research_cannot_distinguish_dragon_before_cutoff() {
        let mut cfg = config();
        let access = [ForecastAccess {
            horizon: 1000,
            coverage: 1.,
            range_width: 0.,
        }];
        for model in [bot_api::DragonModel::Trend, bot_api::DragonModel::Growth] {
            cfg.world.dragon.model = model;
            cfg.world.dragon.probability = 0.;
            let a = World::generate_with_information(&cfg, 7, &[1.], &access);
            cfg.world.dragon.probability = 1.;
            let b = World::generate_with_information(&cfg, 7, &[1.], &access);
            let views = |w: &World| {
                w.deliveries[0]
                    .iter()
                    .filter(|d| d.tick < 5500)
                    .map(|d| {
                        (
                            d.tick,
                            serde_json::to_string(&w.delivery_view(*d, 0)).unwrap(),
                        )
                    })
                    .collect::<Vec<_>>()
            };
            assert_eq!(views(&a), views(&b));
        }
    }
}
