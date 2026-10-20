use bot_api::*;
use bots::lattice::Lattice;
use market::config::Config;
use std::collections::BTreeMap;
use std::path::Path;

fn fixture() -> (Start, Observation) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scenarios/baseline.toml");
    let cfg = Config::load(&path).unwrap();
    let rules = cfg.rules(&vec![10_000; cfg.stocks.len()]);
    let shares = vec![20; rules.stocks.len()];
    let books: Vec<BookView> = rules
        .stocks
        .iter()
        .map(|s| BookView {
            bids: vec![Level {
                price: 29_990,
                qty: 1000,
            }],
            asks: vec![Level {
                price: 30_010,
                qty: 1000,
            }],
            last: 30_000,
            mark: 30_000,
            dividend: s.initial_dividend,
        })
        .collect();
    let start = Start {
        account: "lattice".into(),
        owner: "challenger".into(),
        account_index: 0,
        n_accounts: 150,
        news_multiplier: 1.0,
        forecast_access: ForecastAccess::default(),
        seed: 123,
        params: BTreeMap::new(),
        cash: 2_000_000,
        shares: shares.clone(),
        rules: rules.clone(),
    };
    let obs = Observation {
        tick: 0,
        ticks_total: rules.ticks_total,
        reference_tick: 0,
        season: Season {
            turbulent: false,
            rate_mult: 1.0,
            size_mult: 1.0,
        },
        account: AccountView {
            cash: 2_000_000,
            shares: shares.clone(),
            sellable: shares,
            equity: 8_000_000,
            gross: 6_000_000,
            leverage: 0.75,
            buying_power: 20_000_000,
            ..Default::default()
        },
        open_orders: vec![],
        books,
        trades: vec![vec![]; rules.stocks.len()],
        skipped: 0,
        fills: vec![],
        acks: vec![],
        rejects: vec![],
        news: vec![],
        season_changes: vec![],
        income: vec![],
        terminal_hints: vec![],
        social: None,
    };
    (start, obs)
}

fn news(effective_tick: Tick) -> NewsItem {
    NewsItem {
        id: 7,
        layer: NewsLayer::Stock,
        sector: None,
        changes: vec![Change {
            stock: 0,
            low: 500,
            high: 500,
        }],
        stage: 2,
        forecast: false,
        first_reveal_tick: effective_tick.saturating_sub(120),
        public_tick: effective_tick.saturating_sub(60),
        effective_tick,
        public: true,
    }
}

#[test]
fn seed_and_identity_do_not_drive_the_return_model() {
    let (start, mut obs) = fixture();
    let mut alternative = start.clone();
    alternative.seed = u64::MAX;
    alternative.account_index = 149;
    alternative.account = "different".into();
    alternative.owner = "different-owner".into();
    let mut a = Lattice::new(&start);
    let mut b = Lattice::new(&alternative);
    for t in 0..300 {
        obs.tick = t;
        for (s, book) in obs.books.iter_mut().enumerate() {
            let p = 30_000 + ((t as i64 * (s as i64 + 1) * 13) % 1200);
            book.bids[0].price = p - 10;
            book.asks[0].price = p + 10;
            book.mark = p;
        }
        let mut other_obs = obs.clone();
        if t == 10 {
            obs.news = vec![news(130)];
            other_obs.news = obs.news.clone();
        }
        assert_eq!(a.on_tick(&obs), b.on_tick(&other_obs));
    }
}

#[test]
fn every_batch_respects_available_budget_and_inventory() {
    let (start, mut obs) = fixture();
    let mut bot = Lattice::new(&start);
    obs.account.buying_power = 80_000;
    obs.account.idle = IdleEstimate {
        period_end_tick: 50,
        due: 40_000,
        ..Default::default()
    };
    for t in 0..1000 {
        obs.tick = t;
        let actions = bot.on_tick(&obs);
        assert!(actions.len() <= start.rules.max_actions_per_tick);
        let mut buy_cost = 0;
        let mut sells = vec![0; obs.books.len()];
        for action in actions {
            match action {
                Action::Limit {
                    stock,
                    side,
                    price,
                    qty,
                    tif,
                    ..
                } => {
                    assert_eq!(tif, Tif::Ioc);
                    assert!(price > 0 && qty > 0);
                    match side {
                        Side::Buy => buy_cost += qty * start.rules.buy_reserve_per_share(price),
                        Side::Sell => sells[stock] += qty,
                    }
                }
                other => panic!("unexpected action {other:?}"),
            }
        }
        assert!(buy_cost <= obs.account.buying_power);
        for (s, qty) in sells.iter().enumerate() {
            assert!(*qty <= obs.account.sellable[s]);
        }
    }
}

#[test]
fn news_after_the_last_tick_cannot_raise_settlement_value() {
    let (start, mut obs) = fixture();
    obs.tick = start.rules.last_tick();
    obs.terminal_hints = vec![(33, 33)];
    obs.news = vec![news(start.rules.ticks_total + 1)];
    let mut bot = Lattice::new(&start);
    let actions = bot.on_tick(&obs);
    assert!(actions.iter().all(|a| !matches!(
        a,
        Action::Limit {
            side: Side::Buy,
            ..
        }
    )));
    assert!(actions.iter().any(|a| matches!(
        a,
        Action::Limit {
            stock: 0,
            side: Side::Sell,
            ..
        }
    )));
}
