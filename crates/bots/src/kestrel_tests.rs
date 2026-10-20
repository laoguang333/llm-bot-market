use super::*;
use crate::news_test_support::{observation, start};

fn news(id: u32, effective: Tick, delta: Money) -> NewsItem {
    NewsItem {
        id,
        layer: NewsLayer::Stock,
        sector: Some(0),
        changes: vec![Change {
            stock: 0,
            low: delta,
            high: delta,
        }],
        stage: 0,
        forecast: false,
        first_reveal_tick: 1,
        public_tick: 61,
        effective_tick: effective,
        public: false,
    }
}

#[test]
fn decisions_do_not_depend_on_seed_identity_or_privilege_metadata() {
    let s = start();
    let mut other = s.clone();
    other.seed = u64::MAX;
    other.account_index = 99;
    other.account = "different".into();
    other.owner = "different-owner".into();
    other.news_multiplier = 20.0;
    other.forecast_access.horizon = 1000;
    other.rules.dragon = None;
    let (mut a, mut b) = (Kestrel::new(&s), Kestrel::new(&other));
    for tick in 0..150 {
        let mut o = observation(tick, tick < 70);
        if tick == 20 {
            o.news.push(news(1, 100, 30));
        }
        if tick >= 100 {
            o.books[0].dividend += 30;
        }
        assert_eq!(a.on_tick(&o), b.on_tick(&o));
    }
}

#[test]
fn delivered_news_changes_positions_before_effect_and_is_not_double_counted() {
    let mut s = start();
    s.params.insert("flow".into(), 0.0);
    let (mut plain, mut informed) = (Kestrel::new(&s), Kestrel::new(&s));
    plain.on_tick(&observation(0, true));
    informed.on_tick(&observation(0, true));
    let mut o = observation(1, true);
    let ordinary = plain.on_tick(&o);
    o.news.push(news(1, 100, 30));
    let actions = informed.on_tick(&o);
    assert_ne!(ordinary, actions);
    assert!(actions.iter().any(|a| matches!(
        a,
        Action::Limit {
            stock: 0,
            side: Side::Buy,
            ..
        }
    )));
    let v = informed.value(&o, 0, 30.0);
    informed.knowledge.update(&o);
    assert_eq!(v, informed.value(&o, 0, 30.0));
    o.tick = 100;
    o.books[0].dividend = 130;
    informed.knowledge.update(&o);
    assert!(informed.knowledge.heard.is_empty());
    assert_eq!(informed.value(&o, 0, 30.0), 130.0 * (57.0 + 30.0));
}

#[test]
fn payment_timeline_groups_events_and_respects_scales_and_terminal_boundary() {
    let mut s = start();
    s.rules.ticks_total = 200;
    s.rules.dividend_period = 1;
    s.rules.dividend_scale = 50;
    let mut bot = Kestrel::new(&s);
    let mut o = observation(0, true);
    o.news = vec![
        news(1, 50, -200),
        news(2, 100, 50),
        news(3, 150, 100),
        news(4, 200, 99999),
    ];
    bot.knowledge.update(&o);
    // Rate 100 for ticks 1..49, then latent -100/-50, then 50 for 150..199.
    assert_eq!(bot.value(&o, 0, 10.0), 49.0 * 2.0 + 50.0 + 500.0);
    o.news.push(news(5, 50, 100));
    bot.knowledge.update(&o);
    assert_eq!(bot.value(&o, 0, 10.0), 98.0 + 50.0 + 150.0 + 1500.0);
}

#[test]
fn buys_share_one_budget_include_fees_and_respect_action_limit() {
    let mut s = start();
    s.params.insert("base".into(), 0.5);
    s.params.insert("clip".into(), 1.0);
    let mut o = observation(0, true);
    o.account.buying_power = 250_000;
    o.account.shares.fill(0);
    o.account.sellable.fill(0);
    o.account.gross = 0;
    let actions = Kestrel::new(&s).on_tick(&o);
    let cost: Money = actions
        .iter()
        .filter_map(|a| match a {
            Action::Limit {
                side: Side::Buy,
                price,
                qty,
                tif: Tif::Ioc,
                ..
            } => Some(s.rules.buy_reserve_per_share(*price) * qty),
            _ => None,
        })
        .sum();
    assert!(cost > 0 && cost <= o.account.buying_power);
    s.rules.max_actions_per_tick = 1;
    assert!(Kestrel::new(&s).on_tick(&o).len() <= 1);
    s.rules.max_actions_per_tick = 0;
    assert!(Kestrel::new(&s).on_tick(&o).is_empty());
}

#[test]
fn exits_are_limited_to_sellable_and_terminal_debt_prevents_buys() {
    let s = start();
    let mut o = observation(2900, true);
    o.account.cash = -1_000_000;
    o.account.sellable.fill(3);
    o.terminal_hints = vec![(10, 10)];
    let actions = Kestrel::new(&s).on_tick(&o);
    assert!(!actions.is_empty());
    for action in actions {
        match action {
            Action::Limit {
                side: Side::Sell,
                qty,
                ..
            } => assert!(qty <= 3),
            a => panic!("unexpected risk action: {a:?}"),
        }
    }
}

#[test]
fn missing_liquidity_and_existing_orders_cannot_create_unpriced_orders() {
    let s = start();
    let mut o = observation(0, true);
    for book in &mut o.books {
        book.bids.clear();
        book.asks.clear();
    }
    assert!(Kestrel::new(&s).on_tick(&o).is_empty());
    o.open_orders.push(OpenOrder {
        remaining: 5,
        price: 10000,
        ..OpenOrder::default()
    });
    assert_eq!(
        Kestrel::new(&s).on_tick(&o),
        vec![Action::CancelAll { stock: None }]
    );
}

#[test]
fn holding_tolerance_retains_inventory_without_raising_buy_targets_or_overriding_risk() {
    let s = start();
    let mut o = observation(0, false);
    o.trades.iter_mut().for_each(Vec::clear);
    let ordinary = Kestrel::new(&s).on_tick(&o);
    let patient = Kestrel::new(&s).decide_with_holding(&o, &[], &[0.04; 2]);
    assert!(ordinary.iter().any(|a| matches!(
        a,
        Action::Limit {
            side: Side::Sell,
            ..
        }
    )));
    assert!(patient.is_empty());
    o.account.shares.fill(0);
    o.account.sellable.fill(0);
    o.account.gross = 0;
    assert_eq!(
        Kestrel::new(&s).on_tick(&o),
        Kestrel::new(&s).decide_with_holding(&o, &[], &[0.04; 2])
    );
    o.account.shares.fill(200);
    o.account.sellable.fill(200);
    o.account.gross = o.account.equity * 3;
    assert_eq!(
        Kestrel::new(&s).on_tick(&o),
        Kestrel::new(&s).decide_with_holding(&o, &[], &[0.04; 2])
    );
}
