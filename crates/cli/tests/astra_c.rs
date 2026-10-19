use bot_api::*;
use market::config::Config;

fn fixture(
    dividend: Money,
    history_price: Price,
    params: &[(&str, f64)],
) -> (Box<dyn Bot>, Observation) {
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
        params: [
            ("combo", 0.0),
            ("cap", 10.0),
            ("edge", 100.0),
            ("skew", 0.0),
        ]
        .into_iter()
        .chain(params.iter().copied())
        .map(|(k, v)| (k.to_owned(), v))
        .collect(),
    };
    let mut bot = bots::factory("astra_c", &start).unwrap();
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

fn bid(actions: &[Action]) -> Price {
    actions
        .iter()
        .find_map(|a| match a {
            Action::Limit {
                side: Side::Buy,
                price,
                ..
            } => Some(*price),
            _ => None,
        })
        .expect("buy quote")
}

#[test]
fn settlement_removes_a_stale_resale_premium() {
    let (mut converging, obs) = fixture(100, 10000, &[("settlement", 300.0), ("guard", 10.0)]);
    let (mut historical, _) = fixture(100, 10000, &[("settlement", 0.0), ("guard", 10.0)]);
    assert!(bid(&converging.on_tick(&obs)) < bid(&historical.on_tick(&obs)));
}

#[test]
fn value_guard_caps_stale_history_even_before_settlement() {
    let (mut bot, mut obs) = fixture(100, 10000, &[("settlement", 0.0), ("guard", 0.03)]);
    obs.terminal_hints = vec![(30, 30)];
    assert!(bid(&bot.on_tick(&obs)) <= 3090);
}

#[test]
fn more_inventory_lowers_the_bid() {
    let (mut light, mut obs) = fixture(100, 10000, &[("skew", 0.1), ("consensus", 0.0)]);
    let (mut heavy, _) = fixture(100, 10000, &[("skew", 0.1), ("consensus", 0.0)]);
    obs.account.shares[0] = 20;
    let low_inventory_bid = bid(&light.on_tick(&obs));
    obs.account.shares[0] = 200;
    assert!(bid(&heavy.on_tick(&obs)) < low_inventory_bid);
}

#[test]
fn public_consensus_respects_private_terminal_interval() {
    let (mut bot, mut obs) = fixture(100, 10000, &[("consensus", 1.0), ("settlement", 300.0)]);
    obs.terminal_hints = vec![(29, 30)];
    // The observed book implies 70 payments, but the account knows at most 30.
    assert!(bid(&bot.on_tick(&obs)) <= 3100);
}

#[test]
fn taking_liquidity_does_not_cross_own_resting_orders() {
    for (dividend, history) in [(10, 10000), (1000, 1000)] {
        let (mut bot, obs) = fixture(dividend, history, &[("edge", 0.02)]);
        let actions = bot.on_tick(&obs);
        assert!(actions
            .iter()
            .any(|a| matches!(a, Action::Limit { tif: Tif::Ioc, qty, .. } if *qty > 0)));
        assert!(actions.len() <= 20);
        let mut live = obs.open_orders.clone();
        for action in actions {
            match action {
                Action::Cancel { order_id } => live.retain(|o| o.order_id != order_id),
                Action::Limit {
                    stock,
                    side,
                    price,
                    qty,
                    tif,
                    ..
                } => {
                    assert!(!live.iter().any(|o| o.stock == stock
                        && o.side != side
                        && if side == Side::Buy {
                            price >= o.price
                        } else {
                            price <= o.price
                        }));
                    if tif == Tif::Gtc {
                        live.push(OpenOrder {
                            stock,
                            side,
                            price,
                            remaining: qty,
                            ..Default::default()
                        });
                    }
                }
                _ => {}
            }
        }
    }
}
