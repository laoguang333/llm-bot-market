use market::config::Config;
use market::league::{attribution, league_owners, league_percentiles};

fn short_baseline() -> Config {
    let mut cfg = Config::load(std::path::Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../scenarios/baseline.toml"
    )))
    .unwrap();
    cfg.ticks = 600;
    cfg.schedule.threads = 2;
    cfg.validate().unwrap();
    cfg
}

#[test]
fn league_ranks_only_named_owners_and_attribution_sums_to_the_return() {
    let cfg = short_baseline();
    let owners = league_owners(&cfg);
    assert!(owners.iter().any(|o| o == "tide-incumbent"));
    assert!(owners.iter().all(|o| !o.starts_with("grid")));
    let r = market::report::run_match(&cfg, 3, &bots::factory, None).unwrap();

    let league = league_percentiles(&r, &owners);
    assert_eq!(league.len(), owners.len());
    let mut pct: Vec<f64> = league.iter().map(|(_, p)| *p).collect();
    pct.sort_by(f64::total_cmp);
    assert_eq!(pct[0], 0.0);
    assert_eq!(*pct.last().unwrap(), 100.0);
    // the league order agrees with the whole-market order
    for (a, pa) in &league {
        for (b, pb) in &league {
            let ma = r
                .accounts
                .iter()
                .find(|x| &x.owner == a)
                .unwrap()
                .percentile;
            let mb = r
                .accounts
                .iter()
                .find(|x| &x.owner == b)
                .unwrap()
                .percentile;
            if pa > pb {
                assert!(ma > mb, "{a} {pa} {ma} vs {b} {pb} {mb}");
            }
        }
    }

    for a in &r.accounts {
        let x = attribution(a);
        let parts = x.dividends + x.combos
            - x.trade_fees
            - x.idle_cash_fees
            - x.idle_stock_fees
            - x.interest
            + x.capital;
        assert!((parts - x.net).abs() < 1e-9, "{}: {x:?}", a.account);
        assert!((x.net - (a.return_ratio - 1.0)).abs() < 1e-9);
    }
}
