use bot_api::*;
use market::config::Config;

#[test]
fn endgame_sells_into_a_premium_and_stops_buying_above_value() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../scenarios/baseline.toml");
    let mut cfg = Config::load(std::path::Path::new(path)).unwrap();
    cfg.stocks.truncate(1);
    cfg.combos.clear();
    let start = Start {
        account: "test".into(),
        owner: "test".into(),
        account_index: 0,
        n_accounts: 10,
        seed: 1,
        cash: 2_000_000,
        shares: vec![300],
        rules: cfg.rules(&[10000]),
        params: Default::default(),
    };
    let mut bot = bots::factory("ebb", &start).unwrap();
    // Dividend 100 with at most ~5 payments and 10–50 terminal payments left: the low
    // estimate of holding value is about 1,500, far below the 7,000 price.
    let book = |p: Price| BookView {
        bids: vec![Level {
            price: p - 1,
            qty: 1000,
        }],
        asks: vec![Level {
            price: p + 1,
            qty: 1000,
        }],
        last: p,
        mark: p,
        dividend: 100,
    };
    let mut obs = Observation {
        tick: 0,
        ticks_total: 3000,
        reference_tick: 0,
        season: Season {
            turbulent: false,
            rate_mult: 1.0,
            size_mult: 1.0,
        },
        account: AccountView {
            cash: 2_000_000,
            shares: vec![300],
            sellable: vec![300],
            equity: 4_100_000,
            gross: 2_100_000,
            leverage: 0.5,
            buying_power: 5_000_000,
            ..Default::default()
        },
        open_orders: vec![OpenOrder {
            order_id: 7,
            stock: 0,
            side: Side::Buy,
            price: 6900,
            remaining: 10,
            placed_tick: 2600,
        }],
        books: vec![book(7000)],
        trades: vec![vec![]],
        skipped: 0,
        fills: vec![],
        acks: vec![],
        rejects: vec![],
        news: vec![],
        season_changes: vec![],
        income: vec![],
        terminal_hints: vec![],
    };
    for tick in (2000..2800).step_by(10) {
        obs.tick = tick;
        bot.on_tick(&obs);
    }
    obs.tick = 2800;
    let acts = bot.on_tick(&obs);
    assert!(acts.contains(&Action::Cancel { order_id: 7 }), "{acts:?}");
    assert!(acts.iter().any(|a| matches!(
        a,
        Action::Limit {
            side: Side::Sell,
            price: 6999,
            tif: Tif::Ioc,
            ..
        }
    )));
    assert!(!acts.iter().any(|a| matches!(
        a,
        Action::Limit {
            side: Side::Buy,
            ..
        } | Action::Market {
            side: Side::Buy,
            ..
        }
    )));
}
