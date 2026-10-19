use bot_api::*;
use market::config::Config;

fn fixture(dividend: Money, history_price: Price) -> (Box<dyn Bot>, Observation) {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../scenarios/baseline-v6-five-swift.toml"
    );
    let mut cfg = Config::load(std::path::Path::new(path)).unwrap();
    cfg.stocks.truncate(1);
    cfg.combos.clear();
    let rules = cfg.rules(&[10000]);
    let start = Start {
        account: "test".into(),
        owner: "test".into(),
        account_index: 0,
        n_accounts: 10,
        news_multiplier: 1.0,
        seed: 1,
        cash: 2_000_000,
        shares: vec![100],
        rules,
        params: [("combo".into(), 0.0), ("cap".into(), 10.0)].into(),
    };
    let mut bot = bots::factory("coral", &start).unwrap();
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
            shares: vec![100],
            sellable: vec![95],
            equity: 3_000_000,
            gross: 700_000,
            leverage: 0.24,
            buying_power: 5_000_000,
            ..Default::default()
        },
        open_orders: vec![],
        books: vec![BookView {
            bids: vec![Level {
                price: history_price - 1,
                qty: 100,
            }],
            asks: vec![Level {
                price: history_price + 1,
                qty: 100,
            }],
            last: history_price,
            mark: history_price,
            dividend,
        }],
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
    for tick in (2400..2990).step_by(5) {
        obs.tick = tick;
        bot.on_tick(&obs);
    }
    obs.tick = 2999;
    obs.books[0].bids[0].price = 6999;
    obs.books[0].asks[0].price = 7001;
    obs.books[0].last = 7000;
    obs.books[0].mark = 7000;
    obs.open_orders = vec![
        OpenOrder {
            order_id: 1,
            stock: 0,
            side: Side::Buy,
            price: 6500,
            remaining: 5,
            placed_tick: 2990,
        },
        OpenOrder {
            order_id: 2,
            stock: 0,
            side: Side::Sell,
            price: 7500,
            remaining: 5,
            placed_tick: 2990,
        },
    ];
    (bot, obs)
}

#[test]
fn valuation_shock_can_exit_even_when_the_historical_band_wants_to_buy() {
    let (mut bot, obs) = fixture(10, 10000);
    assert_take_after_cancels(&bot.on_tick(&obs), Side::Sell);
}

#[test]
fn valuation_shock_can_buy_even_when_the_historical_band_wants_to_sell() {
    let (mut bot, obs) = fixture(1000, 1000);
    assert_take_after_cancels(&bot.on_tick(&obs), Side::Buy);
}

fn assert_take_after_cancels(actions: &[Action], side: Side) {
    let take = actions
        .iter()
        .position(|a| {
            matches!(a,
                Action::Limit { side: s, tif: Tif::Ioc, qty, .. } if *s == side && *qty > 0
            )
        })
        .expect("the signal must produce a usable immediate order");
    for id in [1, 2] {
        assert!(
            actions[..take]
                .iter()
                .any(|a| matches!(a, Action::Cancel { order_id } if *order_id == id)),
            "cancel resting orders before crossing the spread"
        );
    }
    assert!(!actions
        .iter()
        .any(|a| matches!(a, Action::Limit { tif: Tif::Gtc, .. })));
    assert!(actions.len() <= 20);
}

#[test]
fn pair_quotes_value_only_the_sets_the_order_can_complete() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../scenarios/baseline-v6-five-swift.toml"
    );
    let mut cfg = Config::load(std::path::Path::new(path)).unwrap();
    cfg.stocks.truncate(2);
    cfg.combos.truncate(1);
    // A lot contains four shares; only two of them complete a pair.
    let quote = |bonus, combo| {
        let mut cfg = cfg.clone();
        cfg.combos[0].bonus = bonus;
        let start = Start {
            account: "pair-test".into(),
            owner: "pair-test".into(),
            account_index: 0,
            n_accounts: 10,
            news_multiplier: 1.0,
            seed: 1,
            cash: 4_000_000,
            shares: vec![20, 22],
            rules: cfg.rules(&[10000, 10000]),
            params: [
                ("combo".into(), combo),
                ("cap".into(), 10.0),
                ("edge".into(), 100.0),
                ("order_frac".into(), 0.04),
            ]
            .into(),
        };
        let mut bot = bots::factory("coral", &start).unwrap();
        let obs = Observation {
            tick: 0,
            ticks_total: 3000,
            reference_tick: 0,
            season: Season {
                turbulent: false,
                rate_mult: 1.0,
                size_mult: 1.0,
            },
            account: AccountView {
                cash: 4_000_000,
                shares: vec![20, 22],
                sellable: vec![20, 22],
                equity: 5_000_000,
                gross: 2_100_000,
                buying_power: 8_000_000,
                ..Default::default()
            },
            open_orders: vec![],
            books: cfg
                .stocks
                .iter()
                .map(|s| BookView {
                    bids: vec![Level {
                        price: 49999,
                        qty: 100,
                    }],
                    asks: vec![Level {
                        price: 50001,
                        qty: 100,
                    }],
                    last: 50000,
                    mark: 50000,
                    dividend: s.dividend,
                })
                .collect(),
            trades: vec![vec![], vec![]],
            skipped: 0,
            fills: vec![],
            acks: vec![],
            rejects: vec![],
            news: vec![],
            season_changes: vec![],
            income: vec![],
            terminal_hints: vec![],
        };
        bot.on_tick(&obs)
            .into_iter()
            .find_map(|a| match a {
                Action::Limit {
                    stock: 0,
                    side: Side::Buy,
                    price,
                    qty: 4,
                    ..
                } => Some(price),
                _ => None,
            })
            .expect("four-share quote")
    };
    let plain = quote(20, 0.0);
    let incremental = (59 + 30) * 20 * 2 / 4;
    assert_eq!(quote(20, 1.0) - plain, incremental);
    assert_eq!(plain - quote(-20, 1.0), incremental);
}
