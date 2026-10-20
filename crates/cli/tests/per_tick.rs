use bots::common::{combo_payments_after, hold_value, payments_after};
use market::config::Config;
use std::path::Path;

fn scenario(name: &str) -> Config {
    Config::load(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("../../scenarios/{name}.toml")),
    )
    .unwrap()
}

#[test]
fn continuous_value_counts_effective_tick_and_converts_terminal_units() {
    let cfg = scenario("baseline-per-tick");
    let r = cfg.rules(&vec![1000; cfg.stocks.len()]);
    assert_eq!(payments_after(&r, 100, 3000), 2899);
    assert_eq!(combo_payments_after(&r, 100, 3000), 2899);
    // 100 rate units = 2 cents/tick, 20 terminal units = 1000 ticks.
    // A public +25 change only adds 0.5 cent at ticks 120..2999 and at terminal.
    let expected = 2.0 * (2899.0 + 1000.0) + 0.5 * (2880.0 + 1000.0);
    assert_eq!(hold_value(&r, 100, 0, 100, &[(0, 25, 120)], 20.0), expected);
    assert_eq!(
        hold_value(&r, 119, 0, 100, &[(0, 25, 120)], 20.0),
        hold_value(&r, 120, 0, 125, &[], 20.0) + 2.5
    );
    assert_eq!(hold_value(&r, 2999, 0, 100, &[(0, 25, 3000)], 20.0), 2000.0);
    assert_eq!(r.terminal_dividend_payments(20.0), 1000.0);
    assert_eq!(r.terminal_combo_payments(20.0), 1000.0);
    assert_eq!(r.combo_cents(20), 0.4);
    assert_eq!(r.idle_rate, 0.006);
}

#[test]
fn legacy_defaults_and_invalid_dividend_units() {
    let old = scenario("baseline-v6-five-swift");
    assert_eq!(old.world.dividend_scale, 1);
    assert_eq!(old.combo_period(), 50);
    assert_eq!(old.world.combo_scale, 1);
    assert_eq!(old.terminal_payment_ticks(), 50);
    let cfg = scenario("baseline-per-tick");
    for scale in [0, -1] {
        let mut bad = cfg.clone();
        bad.world.dividend_scale = scale;
        assert!(bad.validate().is_err());
        let mut bad = cfg.clone();
        bad.world.combo_scale = scale;
        assert!(bad.validate().is_err());
    }
    for period in [0, 3] {
        let mut bad = cfg.clone();
        bad.world.combo_period = Some(period);
        assert!(bad.validate().is_err());
    }
    let mut bad = cfg.clone();
    bad.scoring.terminal_payment_ticks = Some(0);
    assert!(bad.validate().is_err());
    let mut bad = cfg;
    bad.world.dividend_period = 0;
    assert!(bad.validate().is_err());
}

#[test]
fn combo_terminal_keeps_time_scale_even_when_one_tick_rounds_to_zero() {
    let old = scenario("baseline-v6-periodic-combos");
    let new = scenario("baseline-per-tick");
    let a = old.rules(&vec![1000; old.stocks.len()]);
    let b = new.rules(&vec![1000; new.stocks.len()]);
    for sets in [1, 2, 10] {
        for rule in &b.combos {
            let mut shares = vec![0; new.stocks.len()];
            shares[rule.a] = sets;
            shares[rule.b] = sets;
            assert_eq!(
                a.terminal_value(&shares, &[0; 10], 27),
                b.terminal_value(&shares, &[0; 10], 27)
            );
        }
    }
    assert_eq!(b.combo_payment(1, 20), 0);
    assert_eq!(b.combo_payment(2, 20), 1);
    assert_eq!(b.combo_payment(10, 20), 4);
    assert_eq!(b.combo_payment(1, 25), 1);
    assert_eq!(b.combo_payment(1, -20), 0);
    assert_eq!(b.combo_payment(2, -20), -1);
    assert_eq!(b.combo_payment(10, -20), -4);
    assert_eq!(b.combo_payment(1, -25), -1);
    // Old recorded rules without combo_scale retain integer-cent combo rewards.
    let mut json = serde_json::to_value(&a).unwrap();
    json.as_object_mut().unwrap().remove("combo_scale");
    let legacy: bot_api::Rules = serde_json::from_value(json).unwrap();
    assert_eq!(legacy.combo_payment(2, 20), 40);
}
