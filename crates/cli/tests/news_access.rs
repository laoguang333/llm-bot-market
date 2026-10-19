use market::{config::Config, engine::Shared};
use std::path::Path;

#[test]
fn per_group_news_access_tracks_realized_population_and_preserves_other_accounts() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scenarios/baseline-v6.toml");
    let mut cfg = Config::load(&path).unwrap();
    let mut g = cfg.bots[0].clone();
    g.bot = "titan".into();
    g.owner = Some("challenger".into());
    g.count = 1;
    g.count_max = None;
    g.cash = 20_000_000;
    g.shares_each = 200;
    g.news_multiplier = 10.0;
    cfg.bots.push(g);
    let mut ordinary_cfg = cfg.clone();
    ordinary_cfg.bots.last_mut().unwrap().news_multiplier = 1.0;
    for seed in [7, 11, 19] {
        let factory = |name: &str, start: &bot_api::Start| {
            if start.owner == "challenger" {
                let mut legacy = serde_json::to_value(start).unwrap();
                legacy.as_object_mut().unwrap().remove("news_multiplier");
                let legacy: bot_api::Start = serde_json::from_value(legacy).unwrap();
                assert_eq!(legacy.news_multiplier, 1.0);
            }
            assert_eq!(
                start.news_multiplier,
                if start.owner == "challenger" {
                    10.0
                } else {
                    1.0
                }
            );
            bots::factory(name, start)
        };
        let advantaged = Shared::new(&cfg, seed, &factory).unwrap();
        let ordinary = Shared::new(&ordinary_cfg, seed, &bots::factory).unwrap();
        let last = advantaged.n_accounts() - 1;
        assert_eq!(advantaged.accounts[last].lock().owner, "challenger");
        assert_eq!(advantaged.world.news_multipliers[last], 10.0);
        assert!(advantaged.world.news_multipliers[..last]
            .iter()
            .all(|m| *m == 1.0));
        assert_eq!(
            advantaged.world.deliveries[..last],
            ordinary.world.deliveries[..last]
        );
        assert!(advantaged.world.deliveries[last].len() > ordinary.world.deliveries[last].len());
    }
}

#[test]
fn forecast_permissions_follow_realized_groups_and_legacy_starts_disable_them() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scenarios/baseline-v9.toml");
    let mut cfg = Config::load(&path).unwrap();
    let mut group = cfg.bots[0].clone();
    group.bot = "horizon".into();
    group.owner = Some("researcher".into());
    group.count = 1;
    group.count_max = None;
    group.forecast_access = bot_api::ForecastAccess {
        horizon: 1000,
        coverage: 0.5,
        range_width: 3.0,
    };
    cfg.bots.push(group);
    let shared = Shared::new(&cfg, 7, &|name, start| {
        assert_eq!(
            start.forecast_access.horizon,
            if start.owner == "researcher" { 1000 } else { 0 }
        );
        let mut old = serde_json::to_value(start).unwrap();
        old.as_object_mut().unwrap().remove("forecast_access");
        assert_eq!(
            serde_json::from_value::<bot_api::Start>(old)
                .unwrap()
                .forecast_access,
            bot_api::ForecastAccess::default()
        );
        bots::factory(name, start)
    })
    .unwrap();
    let last = shared.n_accounts() - 1;
    assert!(shared.world.deliveries[last].iter().any(|d| d.forecast));
    assert!(shared.world.deliveries[..last]
        .iter()
        .all(|ds| ds.iter().all(|d| !d.forecast)));
    for bad in [-1.0, 1.1, f64::NAN] {
        cfg.bots.last_mut().unwrap().forecast_access.coverage = bad;
        assert!(cfg.validate().is_err());
    }
}
