use market::{config::Config, world::World};

fn config() -> Config {
    Config::load(std::path::Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../scenarios/baseline-v10.toml"
    )))
    .unwrap()
}

#[test]
fn duration_and_hints_follow_length_and_legacy_remains_exact() {
    let mut c = config();
    for (ticks, range, hints) in [
        (3000, (10, 50), vec![(0, 30), (1000, 16), (2000, 6)]),
        (5000, (17, 83), vec![(0, 50), (1666, 27), (3333, 10)]),
        (10000, (33, 167), vec![(0, 100), (3333, 53), (6666, 20)]),
    ] {
        c.ticks = ticks;
        c.validate().unwrap();
        assert_eq!(c.terminal_payments_range(), range);
        assert_eq!(c.terminal_hints(), hints);
        let r = c.rules(&vec![100; c.stocks.len()]);
        assert_eq!(r.terminal_hints, hints);
        assert_eq!((r.terminal_payments_min, r.terminal_payments_max), range);
    }
    let old = Config::load(std::path::Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../scenarios/baseline-v9.toml"
    )))
    .unwrap();
    assert_eq!(old.terminal_payments_range(), (10, 50));
    assert_eq!(old.terminal_hints(), vec![(0, 30), (3333, 16), (6666, 6)]);
}

#[test]
fn conflicts_and_nonfinite_fractional_values_fail() {
    let mutations: &[fn(&mut Config)] = &[
        |c| c.scoring.terminal_payments = Some((1, 5)),
        |c| c.scoring.terminal_hints = vec![(0, 2)],
        |c| c.scoring.terminal_ratio_range = Some((0., f64::NAN)),
        |c| c.scoring.terminal_ratio_range = Some((0.9, 0.1)),
        |c| c.scoring.terminal_hint_fractions = Some(vec![(0., -1.)]),
        |c| c.scoring.terminal_hint_fractions = Some(vec![(1., 0.1)]),
        |c| c.scoring.terminal_hint_fractions = Some(vec![(0.8, 0.1), (0.2, 0.1)]),
    ];
    for mutate in mutations {
        let mut c = config();
        mutate(&mut c);
        assert!(c.validate().is_err());
    }
}

#[test]
fn hints_contain_truth_and_terminal_payout_uses_resolved_units() {
    let c = config();
    let r = c.rules(&vec![100; c.stocks.len()]);
    for seed in [7, 11, 31001, 31020] {
        let w = World::generate(&c, seed, 3);
        assert!((33..=167).contains(&w.terminal_payments));
        for a in 0..3 {
            for (k, &(tick, width)) in r.terminal_hints.iter().enumerate() {
                assert_eq!(w.hints_due(tick), k + 1);
                let (lo, hi) = w.terminal_hint(a, k);
                assert!(lo <= w.terminal_payments && w.terminal_payments <= hi);
                assert_eq!(hi - lo, width);
            }
        }
        let mut shares = vec![0; c.stocks.len()];
        shares[0] = 3;
        let rates: Vec<_> = c.stocks.iter().map(|s| s.dividend).collect();
        assert_eq!(
            r.terminal_value(&shares, &rates, w.terminal_payments),
            3 * 100 * w.terminal_payments
        );
    }
}

#[test]
fn scaled_terminal_does_not_change_news_or_dividends() {
    let c = config();
    let mut old = c.clone();
    old.scoring.terminal_ratio_range = None;
    old.scoring.terminal_hint_fractions = None;
    old.scoring.terminal_payments = Some((10, 50));
    old.scoring.terminal_hints = vec![(0, 30), (3333, 16), (6666, 6)];
    let (a, b) = (
        World::generate(&c, 31001, 3),
        World::generate(&old, 31001, 3),
    );
    assert_eq!(a.timelines, b.timelines);
    assert_eq!(a.deliveries, b.deliveries);
    for e in &a.events {
        assert_eq!(
            serde_json::to_string(&a.view(e.id, 0, 2)).unwrap(),
            serde_json::to_string(&b.view(e.id, 0, 2)).unwrap()
        );
    }
    assert_ne!(a.opening_prices, b.opening_prices);
}
