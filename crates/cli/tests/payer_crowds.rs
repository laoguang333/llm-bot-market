use bot_api::*;
use market::config::Config;
use std::collections::BTreeMap;
use std::path::Path;

fn start(params: &[(&str, f64)]) -> Start {
    let cfg = Config::load(Path::new("../../scenarios/baseline-v18.toml")).unwrap();
    Start {
        account: "test".into(),
        owner: "test".into(),
        account_index: 0,
        n_accounts: 1,
        news_multiplier: 1.0,
        forecast_access: ForecastAccess::default(),
        seed: 18,
        params: params.iter().map(|(k, v)| ((*k).into(), *v)).collect(),
        cash: 1_000_000,
        shares: vec![0; 10],
        rules: cfg.rules(&[1000; 10]),
    }
}

fn observation() -> Observation {
    Observation {
        tick: 0,
        ticks_total: 10000,
        reference_tick: 0,
        season: Season {
            turbulent: false,
            rate_mult: 1.0,
            size_mult: 1.0,
        },
        account: AccountView {
            cash: 1_000_000,
            shares: vec![0; 10],
            sellable: vec![0; 10],
            equity: 1_000_000,
            buying_power: 4_000_000,
            ..Default::default()
        },
        books: (0..10)
            .map(|_| BookView {
                last: 10_000,
                mark: 10_000,
                dividend: 100,
                bids: vec![Level {
                    price: 9999,
                    qty: 1000,
                }],
                asks: vec![Level {
                    price: 10001,
                    qty: 1000,
                }],
            })
            .collect(),
        open_orders: vec![],
        trades: vec![vec![]; 10],
        skipped: 0,
        fills: vec![],
        acks: vec![],
        rejects: vec![],
        news: vec![],
        season_changes: vec![],
        income: vec![],
        terminal_hints: vec![],
        social: None,
    }
}

fn news(stage: usize, change: i64) -> NewsItem {
    NewsItem {
        id: 7,
        layer: NewsLayer::Stock,
        sector: None,
        changes: vec![Change {
            stock: 0,
            low: change,
            high: change,
        }],
        stage,
        forecast: false,
        first_reveal_tick: 0,
        public_tick: 60,
        effective_tick: 120,
        // Synthetic public-feed revisions exercise deduplication independently
        // of the standard world's final-public-stage scheduling.
        public: true,
    }
}

fn price(obs: &mut Observation, p: i64, tick: u32) {
    obs.tick = tick;
    obs.reference_tick = tick;
    obs.books[0].last = p;
    obs.books[0].mark = p;
    obs.books[0].bids[0].price = p - 1;
    obs.books[0].asks[0].price = p + 1;
}

fn sides(actions: &[Action], stock: usize, side: Side) -> bool {
    actions
        .iter()
        .any(|a| matches!(a, Action::Limit {stock: s, side: d, ..} if *s == stock && *d == side))
}

#[test]
fn headline_follower_ignores_private_news_until_public_release() {
    let s = start(&[("baseline", 0.0), ("amplification", 4.0)]);
    let mut bot = bots::factory("news_reactor", &s).unwrap();
    let mut o = observation();
    let mut private = news(0, 100);
    private.public = false;
    o.news = vec![private];
    assert!(bot.on_tick(&o).is_empty());
    o.tick = 60;
    o.news = vec![news(2, 100)];
    assert!(sides(&bot.on_tick(&o), 0, Side::Buy));
}

#[test]
fn news_direction_and_revised_information_change_actual_orders() {
    let s = start(&[("amplification", 4.0), ("baseline", 0.5), ("cap", 0.3)]);
    let mut o = observation();
    o.account.shares[0] = 10;
    o.account.sellable[0] = 10;
    o.news = vec![news(0, 100)];
    let mut bot = bots::factory("news_reactor", &s).unwrap();
    assert!(sides(&bot.on_tick(&o), 0, Side::Buy));
    o.tick = 1;
    o.news = vec![news(2, -100)];
    assert!(sides(&bot.on_tick(&o), 0, Side::Sell));
}

#[test]
fn repeated_disclosures_do_not_multiply_a_news_bet() {
    let s = start(&[
        ("amplification", 3.0),
        ("baseline", 0.0),
        ("clip", 0.1),
        ("cap", 0.5),
    ]);
    let mut a = bots::factory("news_reactor", &s).unwrap();
    let mut b = bots::factory("news_reactor", &s).unwrap();
    let mut o = observation();
    o.news = vec![news(0, 20)];
    assert_eq!(a.on_tick(&o), b.on_tick(&o));
    for tick in 1..20 {
        o.tick = tick;
        let actual = a.on_tick(&o);
        let mut empty = o.clone();
        empty.news.clear();
        assert_eq!(actual, b.on_tick(&empty));
    }
}

#[test]
fn public_revision_retains_original_attention_age() {
    let s = start(&[
        ("amplification", 3.0),
        ("baseline", 0.0),
        ("memory", 50.0),
        ("clip", 0.1),
    ]);
    let mut a = bots::factory("news_reactor", &s).unwrap();
    let mut b = bots::factory("news_reactor", &s).unwrap();
    let mut o = observation();
    o.news = vec![news(0, 50)];
    a.on_tick(&o);
    b.on_tick(&o);
    o.tick = 60;
    o.news = vec![news(2, 50)];
    let actual = a.on_tick(&o);
    o.news.clear();
    assert_eq!(actual, b.on_tick(&o));
}

fn late_start() -> Start {
    start(&[
        ("delay", 20.0),
        ("patience", 5.0),
        ("memory", 5.0),
        ("threshold", 0.01),
        ("chase", 0.1),
    ])
}

#[test]
fn latecomer_waits_before_both_entry_and_exit() {
    for side in [Side::Buy, Side::Sell] {
        let mut bot = bots::factory("latecomer", &late_start()).unwrap();
        let mut o = observation();
        if side == Side::Sell {
            o.account.shares[0] = 20;
            o.account.sellable[0] = 20;
        }
        assert!(bot.on_tick(&o).is_empty());
        let p = if side == Side::Buy { 10200 } else { 9800 };
        price(&mut o, p, 1);
        assert!(bot.on_tick(&o).is_empty());
        price(&mut o, p, 20);
        assert!(bot.on_tick(&o).is_empty());
        price(&mut o, p, 21);
        assert!(sides(&bot.on_tick(&o), 0, side));
    }
}

#[test]
fn late_intent_keeps_original_price_limit_and_expires() {
    let mut bot = bots::factory("latecomer", &late_start()).unwrap();
    let mut o = observation();
    bot.on_tick(&o);
    price(&mut o, 10200, 1);
    bot.on_tick(&o);
    price(&mut o, 15000, 21);
    let actions = bot.on_tick(&o);
    assert!(
        matches!(actions.as_slice(), [Action::Limit { price, tif: Tif::Gtc, .. }] if *price <= 11220)
    );
    o.open_orders.push(OpenOrder {
        order_id: 1,
        stock: 0,
        side: Side::Buy,
        price: 11220,
        remaining: 1,
        placed_tick: 21,
    });
    price(&mut o, 15100, 27);
    assert_eq!(bot.on_tick(&o), vec![Action::CancelAll { stock: Some(0) }]);
}

#[test]
fn contradictory_price_evidence_cancels_a_pending_buy() {
    let mut bot = bots::factory("latecomer", &late_start()).unwrap();
    let mut o = observation();
    bot.on_tick(&o);
    price(&mut o, 10200, 1);
    bot.on_tick(&o);
    price(&mut o, 9500, 10);
    assert!(bot.on_tick(&o).is_empty());
    price(&mut o, 10000, 21);
    assert!(bot.on_tick(&o).is_empty());
}

#[test]
fn partial_fill_does_not_reissue_the_original_quantity() {
    let mut s = late_start();
    s.params.insert("clip".into(), 0.1);
    s.params.insert("cap".into(), 0.2);
    let mut bot = bots::factory("latecomer", &s).unwrap();
    let mut o = observation();
    bot.on_tick(&o);
    price(&mut o, 10200, 1);
    bot.on_tick(&o);
    price(&mut o, 10200, 21);
    assert!(sides(&bot.on_tick(&o), 0, Side::Buy));
    o.account.shares[0] = 18;
    o.account.sellable[0] = 18;
    price(&mut o, 10200, 22);
    let actions = bot.on_tick(&o);
    let bought: i64 = actions
        .iter()
        .map(|a| match a {
            Action::Limit { qty, .. } => *qty,
            _ => 0,
        })
        .sum();
    assert_eq!(bought, 1);
}

#[test]
fn both_crowds_reserve_cash_across_orders_and_never_short() {
    for name in ["news_reactor", "latecomer"] {
        let mut s = late_start();
        s.params.extend(BTreeMap::from([
            ("amplification".into(), 10.0),
            ("clip".into(), 0.1),
        ]));
        let mut bot = bots::factory(name, &s).unwrap();
        let mut o = observation();
        o.account.cash = 25003;
        o.account.reserved_buy = 5000;
        bot.on_tick(&o);
        for b in &mut o.books {
            b.last = 10200;
            b.asks[0].price = 10201;
            b.bids[0].price = 10199;
        }
        o.tick = 1;
        o.news = (0..10)
            .map(|stock| {
                let mut item = news(2, 100);
                item.id = stock as u32;
                item.changes[0].stock = stock;
                item
            })
            .collect();
        let immediate = bot.on_tick(&o);
        o.tick = 21;
        o.news.clear();
        let delayed = bot.on_tick(&o);
        assert!(!immediate.is_empty() || !delayed.is_empty());
        for actions in [immediate, delayed] {
            let mut reserved = 0;
            for a in actions {
                match a {
                    Action::Limit {
                        stock,
                        side,
                        price,
                        qty,
                        tif: Tif::Ioc,
                        ..
                    } => {
                        assert!(qty > 0 && price > 0);
                        if side == Side::Buy {
                            reserved += qty * s.rules.buy_reserve_per_share(price);
                        } else {
                            assert!(qty <= o.account.sellable[stock]);
                        }
                    }
                    _ => panic!("new crowds must use bounded IOC orders"),
                }
            }
            assert!(reserved <= 20003);
        }
    }
}

#[test]
fn unused_information_does_not_change_orders() {
    for name in ["news_reactor", "latecomer"] {
        let s = late_start();
        let mut a = bots::factory(name, &s).unwrap();
        let mut b = bots::factory(name, &s).unwrap();
        let mut o = observation();
        for tick in 0..40 {
            price(&mut o, if tick == 0 { 10000 } else { 10200 }, tick);
            o.news = if name == "news_reactor" && tick == 1 {
                vec![news(2, 100)]
            } else {
                vec![]
            };
            let mut altered = o.clone();
            altered.social = Some(SocialView {
                peer_stock: vec![1000.0; 10],
                own_net: -1000.0,
                ..Default::default()
            });
            altered.terminal_hints = vec![(100000, 200000)];
            altered.trades[0].push(Trade {
                tick,
                price: 100000,
                qty: 100000,
                aggressor: Side::Buy,
            });
            if name == "latecomer" {
                altered.news = vec![news(2, -1000)];
            } else {
                let mut forecast = news(2, -1000);
                forecast.id = 999;
                forecast.forecast = true;
                altered.news.push(forecast);
            }
            assert_eq!(a.on_tick(&o), b.on_tick(&altered), "{name} tick={tick}");
        }
    }
}

#[test]
fn insolvent_accounts_do_not_submit_orders() {
    for name in ["news_reactor", "latecomer"] {
        let mut bot = bots::factory(name, &late_start()).unwrap();
        let mut o = observation();
        o.account.equity = -1;
        o.news = vec![news(2, 100)];
        assert!(bot.on_tick(&o).is_empty());
    }
}

#[test]
fn missing_quotes_create_bounded_liquidity_then_cancel_on_reversal() {
    let s = start(&[("baseline", 0.0), ("amplification", 4.0), ("cap", 0.3)]);
    let mut bot = bots::factory("news_reactor", &s).unwrap();
    let mut o = observation();
    o.books[0].asks.clear();
    o.books[0].bids.clear();
    o.news = vec![news(0, 100)];
    let actions = bot.on_tick(&o);
    let (price, qty) = match actions.as_slice() {
        [Action::Limit {
            stock: 0,
            side: Side::Buy,
            price,
            qty,
            tif: Tif::Gtc,
            ..
        }] => (*price, *qty),
        _ => panic!("must supply one bounded buy quote"),
    };
    assert!(price <= 16640);
    o.open_orders.push(OpenOrder {
        order_id: 1,
        stock: 0,
        side: Side::Buy,
        price,
        remaining: qty,
        placed_tick: 0,
    });
    o.account.reserved_buy = qty * s.rules.buy_reserve_per_share(price);
    o.tick = 1;
    o.news.clear();
    assert!(
        bot.on_tick(&o).is_empty(),
        "do not duplicate a resting intention"
    );
    o.tick = 2;
    o.news = vec![news(2, -100)];
    assert_eq!(bot.on_tick(&o), vec![Action::CancelAll { stock: Some(0) }]);
}

#[test]
fn stale_resting_quotes_are_cancelled_before_reservations_are_reused() {
    let s = start(&[("baseline", 0.0), ("amplification", 4.0)]);
    let mut bot = bots::factory("news_reactor", &s).unwrap();
    let mut o = observation();
    o.news = vec![news(0, 100)];
    bot.on_tick(&o);
    o.tick = 20;
    o.news.clear();
    o.open_orders.push(OpenOrder {
        order_id: 7,
        stock: 0,
        side: Side::Buy,
        price: 10100,
        remaining: 1,
        placed_tick: 0,
    });
    assert_eq!(bot.on_tick(&o), vec![Action::CancelAll { stock: Some(0) }]);
}

#[test]
fn news_mispricing_affects_the_limit_but_revisions_do_not_reanchor() {
    let s = start(&[
        ("baseline", 0.0),
        ("amplification", 4.0),
        ("clip", 0.1),
        ("memory", 2000.0),
    ]);
    let mut bot = bots::factory("news_reactor", &s).unwrap();
    let mut o = observation();
    o.news = vec![news(0, 20)];
    let actions = bot.on_tick(&o);
    assert!(
        matches!(actions.as_slice(), [Action::Limit {price, side: Side::Buy, ..}] if *price > 14000)
    );
    price(&mut o, 20000, 1);
    o.news = vec![news(2, 20)];
    let actions = bot.on_tick(&o);
    assert!(
        matches!(actions.as_slice(), [Action::Limit {price, tif: Tif::Gtc, ..}] if *price <= 16640)
    );
}

#[test]
fn panic_sell_price_does_not_expand_the_clip_quantity() {
    let s = start(&[("baseline", 0.0), ("amplification", 4.0), ("clip", 0.02)]);
    let mut o = observation();
    o.account.shares[0] = 100;
    o.account.sellable[0] = 100;
    o.news = vec![news(2, -100)];
    let actions = bots::factory("news_reactor", &s).unwrap().on_tick(&o);
    assert!(
        matches!(actions.as_slice(), [Action::Limit { side: Side::Sell, qty, .. }] if *qty <= 2)
    );
}

fn grid_observation() -> Observation {
    let mut o = observation();
    o.account.cash = 500_000;
    o.account.shares = vec![5; 10];
    o.account.sellable = vec![5; 10];
    o.account.gross = 500_000;
    o
}

#[test]
fn grid_v2_quotes_both_sides_without_crossing_or_borrowing() {
    let s = start(&[("clip", 0.02), ("gap", 0.006), ("invested", 0.8)]);
    let o = grid_observation();
    let actions = bots::factory("grid_v2", &s).unwrap().on_tick(&o);
    let mut cash = 0;
    for stock in 0..10 {
        assert!(sides(&actions, stock, Side::Buy));
        assert!(sides(&actions, stock, Side::Sell));
    }
    for action in actions {
        match action {
            Action::Limit {
                stock,
                side,
                price,
                qty,
                tif: Tif::Gtc,
                ..
            } => {
                if side == Side::Buy {
                    assert!(price < o.books[stock].asks[0].price);
                    cash += qty * s.rules.buy_reserve_per_share(price);
                } else {
                    assert!(price > o.books[stock].bids[0].price);
                    assert!(qty <= o.account.sellable[stock]);
                }
            }
            _ => panic!("grid should make bounded resting quotes"),
        }
    }
    assert!(cash <= o.account.cash);
}

#[test]
fn grid_v2_respects_cash_inventory_and_total_stock_capacity() {
    let s = start(&[("clip", 0.05), ("cap", 0.15), ("invested", 0.75)]);
    let mut o = observation();
    o.account.shares[0] = 40;
    o.account.sellable[0] = 40;
    o.account.gross = 400_000;
    o.account.cash = 600_000;
    o.account.reserved_buy = 300_000;
    let actions = bots::factory("grid_v2", &s).unwrap().on_tick(&o);
    assert!(!sides(&actions, 0, Side::Buy));
    assert!(sides(&actions, 0, Side::Sell));
    let reserved: i64 = actions
        .iter()
        .map(|a| match a {
            Action::Limit {
                side: Side::Buy,
                price,
                qty,
                ..
            } => qty * s.rules.buy_reserve_per_share(*price),
            _ => 0,
        })
        .sum();
    assert!(reserved <= 50_000);
    o.account.cash = 0;
    o.account.reserved_buy = 0;
    let actions = bots::factory("grid_v2", &s).unwrap().on_tick(&o);
    assert!(!actions.iter().any(|a| matches!(
        a,
        Action::Limit {
            side: Side::Buy,
            ..
        }
    )));
}

#[test]
fn grid_v2_keeps_young_quotes_and_refreshes_without_duplicate_reservations() {
    let s = start(&[("clip", 0.02), ("lifetime", 5.0)]);
    let mut bot = bots::factory("grid_v2", &s).unwrap();
    let mut o = grid_observation();
    let actions = bot.on_tick(&o);
    o.open_orders = actions
        .iter()
        .enumerate()
        .map(|(id, a)| match a {
            Action::Limit {
                stock,
                side,
                price,
                qty,
                ..
            } => OpenOrder {
                order_id: id as u64,
                stock: *stock,
                side: *side,
                price: *price,
                remaining: *qty,
                placed_tick: 0,
            },
            _ => unreachable!(),
        })
        .collect();
    o.tick = 4;
    assert!(bot.on_tick(&o).is_empty());
    o.tick = 5;
    let actions = bot.on_tick(&o);
    assert_eq!(actions.len(), 10);
    assert!(actions
        .iter()
        .all(|a| matches!(a, Action::CancelAll { stock: Some(_) })));
}

#[test]
fn grid_v2_ignores_news_social_income_and_terminal_information() {
    let s = start(&[("clip", 0.02)]);
    let mut a = bots::factory("grid_v2", &s).unwrap();
    let mut b = bots::factory("grid_v2", &s).unwrap();
    let o = grid_observation();
    let mut changed = o.clone();
    changed.news = vec![news(2, 1000)];
    changed.terminal_hints = vec![(10000, 20000)];
    changed.social = Some(SocialView {
        peer_stock: vec![100.0; 10],
        ..Default::default()
    });
    changed.books.iter_mut().for_each(|b| b.dividend = 99999);
    assert_eq!(a.on_tick(&o), b.on_tick(&changed));
}
