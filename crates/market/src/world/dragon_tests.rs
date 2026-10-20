use super::*;
use std::path::Path;

fn config() -> Config {
    Config::load(Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../scenarios/baseline-v7.toml"
    )))
    .unwrap()
}

#[test]
fn configurable_growth_start_preserves_legacy_and_advances_disclosure() {
    let mut cfg = Config::load(Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../scenarios/baseline-v14.toml"
    )))
    .unwrap();
    assert_eq!(cfg.dragon_start_tick(), 5500);
    cfg.world.dragon.probability = 1.0;
    let old = World::generate(&cfg, 7, 0);
    cfg.world.dragon.start_fraction = 1.0 / 3.0;
    cfg.validate().unwrap();
    assert_eq!(cfg.rules(&[1000; 10]).dragon.unwrap().divergence_tick, 3333);
    let early = World::generate(&cfg, 7, 0);
    let (a, b) = (old.dragon.as_ref().unwrap(), early.dragon.as_ref().unwrap());
    assert_eq!(a.selected_stock, b.selected_stock);
    assert_eq!(a.candidates, b.candidates);
    assert_eq!(b.divergence_tick, 3333);
    assert_eq!(old.opening_prices, early.opening_prices);
    assert_eq!(old.terminal_payments, early.terminal_payments);
    let selected = b.selected_stock.unwrap();
    let sum = |w: &World| (3333..5500).map(|t| w.dividend(selected, t)).sum::<i64>();
    assert!(sum(&early) > sum(&old));
    for bad in [f64::NAN, 0.0, 0.9] {
        cfg.world.dragon.start_fraction = bad;
        assert!(cfg.validate().is_err());
    }
}

#[test]
fn winner_is_hidden_through_midgame_and_normal_news_lottery_is_preserved() {
    for seed in [7, 11, 19] {
        let mut cfg = config();
        cfg.world.dragon.enabled = false;
        let base = World::generate(&cfg, seed, 3);
        cfg.world.dragon.enabled = true;
        cfg.world.dragon.probability = 0.0;
        let none = World::generate(&cfg, seed, 3);
        cfg.world.dragon.probability = 1.0;
        let winner = World::generate(&cfg, seed, 3);
        let cutoff = winner.dragon.as_ref().unwrap().divergence_tick;
        assert!(none.dragon.as_ref().unwrap().selected_stock.is_none());
        assert!(winner.dragon.as_ref().unwrap().selected_stock.is_some());
        assert_eq!(
            none.dragon.as_ref().unwrap().candidates,
            winner.dragon.as_ref().unwrap().candidates
        );
        for w in [&none, &winner] {
            assert_eq!(base.opening_prices, w.opening_prices);
            assert_eq!(base.turbulent, w.turbulent);
            assert_eq!(base.terminal_payments, w.terminal_payments);
            for e in &base.events {
                let added = w
                    .events
                    .iter()
                    .find(|other| other.sampling_key == e.sampling_key)
                    .unwrap();
                assert_eq!(e.changes, added.changes);
                assert_eq!(e.sigma, added.sigma);
                assert_eq!(e.first_reveal, added.first_reveal);
                for account in 0..3 {
                    let a: Vec<_> = base.deliveries[account]
                        .iter()
                        .filter(|x| x.event == e.id)
                        .map(|x| (x.tick, x.stage))
                        .collect();
                    let b: Vec<_> = w.deliveries[account]
                        .iter()
                        .filter(|x| x.event == added.id)
                        .map(|x| (x.tick, x.stage))
                        .collect();
                    assert_eq!(a, b);
                    for stage in 0..cfg.world.news.disclosure.stages.len() {
                        assert_eq!(
                            base.view(e.id, account, stage).changes,
                            w.view(added.id, account, stage).changes
                        );
                    }
                }
            }
        }
        for account in 0..3 {
            let prefix = |w: &World| {
                w.deliveries[account]
                    .iter()
                    .filter(|d| d.tick < cutoff)
                    .map(|d| serde_json::to_value(w.view(d.event, account, d.stage)).unwrap())
                    .collect::<Vec<_>>()
            };
            assert_eq!(prefix(&none), prefix(&winner));
        }
        for tick in 0..cutoff {
            assert_eq!(none.dividends(tick), winner.dividends(tick));
        }
    }
}

#[test]
fn legacy_pulse_overlay_is_additive_nonnegative_delayed_and_has_a_real_pullback() {
    for probability in [0.0, 1.0] {
        let mut cfg = config();
        cfg.world.dragon.model = bot_api::DragonModel::Pulses;
        cfg.world.dragon.enabled = false;
        let base = World::generate(&cfg, 19, 1);
        cfg.world.dragon.enabled = true;
        cfg.world.dragon.probability = probability;
        let world = World::generate(&cfg, 19, 1);
        let regime = world.dragon.as_ref().unwrap();
        for stock in 0..cfg.stocks.len() {
            for tick in 0..cfg.ticks {
                let extra = regime
                    .episodes
                    .iter()
                    .rev()
                    .find(|e| e.stock == stock && e.effective <= tick)
                    .map_or(0, |e| e.extra_rate);
                assert!(extra >= 0);
                assert_eq!(
                    world.dividend(stock, tick),
                    base.dividend(stock, tick) + extra
                );
            }
        }
        for e in &regime.episodes {
            assert_eq!(
                e.effective,
                e.first_reveal + cfg.world.news.disclosure.lead_ticks
            );
            assert!(e.effective < cfg.ticks);
        }
        if let Some(stock) = regime.selected_stock {
            let rates: Vec<_> = regime
                .episodes
                .iter()
                .filter(|e| e.stock == stock)
                .map(|e| e.extra_rate)
                .collect();
            assert!(rates[4] >= cfg.stocks[stock].dividend * 2);
            assert!(rates[5] < rates[4] * 6 / 10);
            assert!(rates[6] > rates[5]);
        } else {
            assert_eq!(
                world.dividends(cfg.ticks - 1),
                base.dividends(cfg.ticks - 1)
            );
        }
        for event in &world.events {
            for stage in 0..cfg.world.news.disclosure.stages.len() {
                let item = world.view(event.id, 0, stage);
                for (c, &(_, truth)) in item.changes.iter().zip(&event.changes) {
                    assert!(c.low <= truth && truth <= c.high);
                    if item.public {
                        assert_eq!(c.low, c.high);
                    }
                }
            }
        }
    }
}

#[test]
fn public_rules_contain_distribution_only_and_invalid_regimes_are_rejected() {
    let cfg = config();
    let rules = cfg.rules(&vec![100; cfg.stocks.len()]);
    let json = serde_json::to_value(&rules).unwrap();
    let public = json["dragon"].as_object().unwrap();
    assert_eq!(public.len(), 6);
    assert!(public.contains_key("probability") && public.contains_key("divergence_tick"));
    assert!(!public.contains_key("selected_stock") && !public.contains_key("episodes"));
    let mut legacy = json;
    legacy.as_object_mut().unwrap().remove("dragon");
    assert!(serde_json::from_value::<bot_api::Rules>(legacy)
        .unwrap()
        .dragon
        .is_none());
    for value in [-0.1, 1.1, f64::NAN] {
        let mut bad = cfg.clone();
        bad.world.dragon.probability = value;
        assert!(bad.validate().is_err());
    }
    for count in [0, 1, cfg.stocks.len() + 1] {
        let mut bad = cfg.clone();
        bad.world.dragon.candidates = count;
        assert!(bad.validate().is_err());
    }
    let mut bad = cfg.clone();
    bad.world.dragon.peak_extra = (4.0, 2.0);
    assert!(bad.validate().is_err());
    let mut bad = cfg;
    bad.ticks = 300;
    assert!(bad.validate().is_err());
}

#[test]
fn half_probability_is_one_world_draw_and_all_stocks_can_be_selected() {
    let mut cfg = config();
    cfg.world.dragon.model = bot_api::DragonModel::Pulses;
    let mut selected = 0;
    let mut seen = vec![false; cfg.stocks.len()];
    for seed in 0..400 {
        let result = dragon::overlay(
            &cfg,
            seed,
            &vec![0; cfg.stocks.len()],
            &[],
            &mut vec![],
            &mut vec![vec![]; cfg.stocks.len()],
        )
        .unwrap();
        assert_eq!(result.candidates.len(), 3);
        if let Some(stock) = result.selected_stock {
            selected += 1;
            seen[stock] = true;
        }
    }
    assert!((160..240).contains(&selected), "selected {selected}/400");
    assert!(seen.into_iter().all(|x| x));
}

#[test]
fn trend_updates_are_small_irregular_and_apply_to_all_stocks() {
    let mut cfg = config();
    cfg.world.dragon.probability = 1.0;
    let mut found_decline = false;
    let mut final_growth = Vec::new();
    for seed in 27001..27021 {
        let mut plain_cfg = cfg.clone();
        plain_cfg.world.dragon.enabled = false;
        let base = World::generate(&plain_cfg, seed, 0);
        let world = World::generate(&cfg, seed, 0);
        let regime = world.dragon.as_ref().unwrap();
        assert_eq!(regime.model, bot_api::DragonModel::Trend);
        for stock in 0..cfg.stocks.len() {
            let path: Vec<_> = regime
                .episodes
                .iter()
                .filter(|e| e.stock == stock)
                .collect();
            assert!(path.len() > 15);
            let mut previous = 0;
            for e in path {
                let delta = e.extra_rate - previous;
                let basis = cfg.stocks[stock].dividend + previous;
                assert!(
                    delta as f64 <= basis as f64 * cfg.world.dragon.trend.max_step.exp_m1() + 1.1
                );
                found_decline |= delta < 0;
                previous = e.extra_rate;
            }
            for tick in (0..cfg.ticks).step_by(17) {
                let extra = regime
                    .episodes
                    .iter()
                    .rev()
                    .find(|e| e.stock == stock && e.effective <= tick)
                    .map_or(0, |e| e.extra_rate);
                assert_eq!(
                    world.dividend(stock, tick),
                    base.dividend(stock, tick) + extra
                );
            }
        }
        let chosen = regime.selected_stock.unwrap();
        let final_extra = regime
            .episodes
            .iter()
            .rev()
            .find(|e| e.stock == chosen)
            .unwrap()
            .extra_rate;
        final_growth.push(final_extra as f64 / cfg.stocks[chosen].dividend as f64);
    }
    assert!(found_decline);
    // Distribution sanity, not a guaranteed target for any particular world.
    assert!(final_growth.iter().sum::<f64>() / final_growth.len() as f64 > 0.5);
    assert!(
        final_growth
            .iter()
            .copied()
            .fold(f64::NEG_INFINITY, f64::max)
            - final_growth.iter().copied().fold(f64::INFINITY, f64::min)
            > 0.5
    );
}
