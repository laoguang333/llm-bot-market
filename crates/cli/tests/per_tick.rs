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
    assert_eq!(combo_payments_after(&r, 100, 3000), 57);
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
    assert_eq!(r.terminal_combo_payments(20.0), 20.0);
}

#[test]
fn legacy_defaults_and_invalid_dividend_units() {
    let old = scenario("baseline-v6-five-swift");
    assert_eq!(old.world.dividend_scale, 1);
    assert_eq!(old.combo_period(), 50);
    assert_eq!(old.terminal_payment_ticks(), 50);
    let cfg = scenario("baseline-per-tick");
    for scale in [0, -1] {
        let mut bad = cfg.clone();
        bad.world.dividend_scale = scale;
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
