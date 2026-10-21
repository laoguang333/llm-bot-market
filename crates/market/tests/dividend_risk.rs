use bot_api::{DividendRiskRules, ForecastAccess};
use market::{config::Config, world::World};

fn config() -> Config {
    Config::load(std::path::Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../scenarios/dividend-v20.toml"
    )))
    .unwrap()
}

#[test]
fn invalid_risk_parameters_are_rejected_and_rules_only_publish_the_distribution() {
    let c = config();
    let rules = c.rules(&vec![100; c.stocks.len()]);
    assert_eq!(
        serde_json::to_value(&rules.dividend_risk).unwrap(),
        serde_json::to_value(&c.world.dividend_risk).unwrap()
    );
    for change in [
        |r: &mut DividendRiskRules| r.review_fraction = (0.3, 0.1),
        |r: &mut DividendRiskRules| r.recovery_probability = f64::NAN,
        |r: &mut DividendRiskRules| r.suspension_probability = 0.9,
        |r: &mut DividendRiskRules| r.impaired_retention = (0.0, 0.2),
        |r: &mut DividendRiskRules| r.recovered_retention = (0.1, 0.2),
        |r: &mut DividendRiskRules| r.adjustment_fraction = 0.00001,
        |r: &mut DividendRiskRules| r.adjustment_fraction = 0.4,
    ] {
        let mut invalid = c.clone();
        change(invalid.world.dividend_risk.as_mut().unwrap());
        assert!(invalid.validate().is_err());
    }
    let mut invalid = c;
    invalid.world.min_dividend = 1;
    assert!(invalid.validate().is_err());
}

#[test]
fn suspension_blocks_positive_growth_and_shocks_and_pays_no_ordinary_terminal_value() {
    let mut c = config();
    let r = c.world.dividend_risk.as_mut().unwrap();
    r.review_fraction = (0.1, 0.1);
    r.suspension_probability = 1.0;
    r.impairment_probability = 0.0;
    r.recovery_probability = 0.0;
    c.validate().unwrap();
    let w = World::generate(&c, 7, 1);
    let stopped = 1250 + c.world.news.disclosure.lead_ticks;
    for stock in 0..c.stocks.len() {
        for tick in stopped..c.ticks {
            assert_eq!(w.dividend(stock, tick), 0);
        }
    }
    let mut rules = c.rules(&vec![100; c.stocks.len()]);
    // Existing independent combo contracts are tested separately by the engine.
    rules.combos.clear();
    assert_eq!(
        rules.terminal_value(
            &vec![100; c.stocks.len()],
            &w.dividends(c.ticks - 1),
            w.terminal_payments
        ),
        0
    );
}

#[test]
fn ordinary_shocks_cannot_resurrect_a_zero_dividend_without_growth_or_recovery() {
    let mut c = config();
    c.world.dragon.enabled = false;
    for s in &mut c.stocks {
        s.dividend = 0;
    }
    let w = World::generate(&c, 19, 0);
    assert!(w.events.is_empty());
    assert!(w.timelines.iter().all(Vec::is_empty));
}

#[test]
fn every_rebased_news_range_contains_truth_and_reconstructs_the_paid_path() {
    let c = config();
    let forecasts = [ForecastAccess {
        horizon: 1000,
        coverage: 1.0,
        range_width: 3.0,
    }];
    for seed in 88001..88006 {
        let w = World::generate_with_information(&c, seed, &[1.0], &forecasts);
        let plain = World::generate(&c, seed, 0);
        assert_eq!(w.timelines, plain.timelines);
        assert_eq!(w.opening_prices, plain.opening_prices);
        let mut values = w.initial_dividends.clone();
        let mut timelines = vec![vec![]; values.len()];
        let mut keys = std::collections::HashSet::new();
        for (id, e) in w.events.iter().enumerate() {
            assert_eq!(e.id as usize, id);
            assert!(keys.insert(e.id));
            assert_eq!(e.changes.len(), e.sigma.len());
            assert!(e.sigma.iter().all(|x| x.is_finite() && *x >= 0.0));
            for &(s, delta) in &e.changes {
                values[s] += delta;
                assert!(values[s] >= 0);
                timelines[s].push((e.effective, values[s]));
            }
        }
        assert_eq!(w.timelines, timelines);
        for &d in &w.deliveries[0] {
            let e = &w.events[d.event as usize];
            let view = w.delivery_view(d, 0);
            assert!(d.tick <= e.effective);
            for (v, &(s, truth)) in view.changes.iter().zip(&e.changes) {
                assert_eq!(v.stock, s);
                assert!(v.low <= truth && truth <= v.high);
                if view.public {
                    assert_eq!(v.low, v.high);
                }
            }
        }
        for s in 0..values.len() {
            for t in 0..c.ticks {
                let pos = timelines[s].partition_point(|(tick, _)| *tick <= t);
                let expected = if pos == 0 {
                    w.initial_dividends[s]
                } else {
                    timelines[s][pos - 1].1
                };
                assert_eq!(w.dividend(s, t), expected);
            }
        }
    }
}

#[test]
fn risk_does_not_bypass_growth_information_embargo_or_change_opening_lottery() {
    let mut c = config();
    let access = [ForecastAccess {
        horizon: 10000,
        coverage: 1.0,
        range_width: 0.0,
    }];
    c.world.dragon.probability = 0.0;
    let a = World::generate_with_information(&c, 11, &[1.0], &access);
    c.world.dragon.probability = 1.0;
    let b = World::generate_with_information(&c, 11, &[1.0], &access);
    let visible = |w: &World| {
        w.deliveries[0]
            .iter()
            .filter(|d| d.tick < c.dragon_start_tick())
            .map(|d| {
                (
                    d.tick,
                    serde_json::to_string(&w.delivery_view(*d, 0)).unwrap(),
                )
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(visible(&a), visible(&b));
    c.world.dividend_risk = None;
    let legacy = World::generate(&c, 11, 0);
    assert_eq!(b.opening_prices, legacy.opening_prices);
    assert_eq!(b.terminal_payments, legacy.terminal_payments);
    assert_eq!(b.turbulent, legacy.turbulent);
}

#[test]
fn impairment_and_recovery_both_occur_without_guaranteeing_recovery() {
    let c = config();
    let mut recovered = 0;
    let mut persistent = 0;
    let mut suspended = 0;
    for seed in 88101..88141 {
        let w = World::generate(&c, seed, 0);
        for s in 0..c.stocks.len() {
            let initial = w.initial_dividends[s];
            let cut = (0..c.ticks / 2).find(|&t| w.dividend(s, t) * 4 <= initial);
            if let Some(cut) = cut {
                if (cut..c.ticks).any(|t| w.dividend(s, t) * 4 >= initial * 3) {
                    recovered += 1;
                } else {
                    persistent += 1;
                }
            }
            if (0..c.ticks).any(|t| w.dividend(s, t) == 0) {
                suspended += 1;
            }
        }
    }
    assert!(
        recovered > 0 && persistent > recovered && suspended > 0,
        "recovered={recovered}, persistent={persistent}, suspended={suspended}"
    );
}

#[test]
fn stopped_dividends_settle_at_effective_ticks_and_preserve_cash_and_shares() {
    struct Holder;
    impl bot_api::Bot for Holder {
        fn on_tick(&mut self, _: &bot_api::Observation) -> Vec<bot_api::Action> {
            vec![]
        }
    }
    let mut c = config();
    c.bots.truncate(1);
    c.bots[0].count = 1;
    c.bots[0].count_max = None;
    c.capital_redistribution = None;
    c.fees.idle_rate = 0.0;
    c.schedule.threads = 1;
    let r = c.world.dividend_risk.as_mut().unwrap();
    r.suspension_probability = 1.0;
    r.impairment_probability = 0.0;
    r.recovery_probability = 0.0;
    c.validate().unwrap();
    let w = World::generate(&c, 88200, 1);
    let result =
        market::report::run_match(&c, 88200, &|_, _| Some(Box::new(Holder)), None).unwrap();
    let a = &result.accounts[0];
    let rules = c.rules(&a.initial_shares);
    let mut dividends = 0;
    let mut combos = 0;
    for tick in 1..c.ticks {
        for (s, &q) in a.initial_shares.iter().enumerate() {
            dividends += rules.dividend_payment(q, w.dividend(s, tick));
        }
        for combo in &rules.combos {
            if combo.active(tick) {
                combos += rules.combo_payment(combo.sets(&a.initial_shares), combo.bonus);
            }
        }
    }
    assert_eq!(a.stats.dividends, dividends);
    assert_eq!(a.stats.combo_dividends, combos);
    assert_eq!(a.shares, a.initial_shares);
    assert_eq!(a.cash, a.initial_cash + dividends + combos);
    assert_eq!(result.treasury + a.cash, a.initial_cash);
    assert!(result.shares_burned.iter().all(|q| *q == 0));
    assert_eq!(result.final_dividends, vec![0; c.stocks.len()]);
    assert_eq!(
        a.terminal_value,
        rules.terminal_value(&a.shares, &result.final_dividends, result.terminal_payments)
    );
    assert_eq!(result.audits, 20);
}
