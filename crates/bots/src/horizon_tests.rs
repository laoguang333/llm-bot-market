use super::*;
use crate::news_test_support::{observation, start};

fn news(id: u32, delta: i64, effective: Tick, forecast: bool) -> NewsItem {
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
        forecast,
        first_reveal_tick: effective.saturating_sub(120),
        public_tick: effective.saturating_sub(60),
        effective_tick: effective,
        public: false,
    }
}
fn empty(tick: Tick) -> Observation {
    let mut o = observation(tick, false);
    o.trades.iter_mut().for_each(Vec::clear);
    o.account.shares.fill(0);
    o.account.sellable.fill(0);
    o.account.gross = 0;
    o.account.equity = o.account.cash;
    o
}
fn buys(actions: &[Action]) -> i64 {
    actions
        .iter()
        .filter_map(|a| match a {
            Action::Limit {
                stock: 0,
                side: Side::Buy,
                qty,
                ..
            } => Some(*qty),
            _ => None,
        })
        .sum()
}

#[test]
fn distant_recovery_affects_plan_immediately_and_dip_unlocks_larger_entry() {
    let mut s = start();
    s.params.insert("clip".into(), 0.1);
    let mut bot = Horizon::new(&s);
    let mut o = empty(0);
    o.news = vec![news(1, -40, 100, true), news(2, 80, 700, true)];
    let first = bot.on_tick(&o);
    let p = bot.states[0]
        .plan
        .expect("700-tick recovery must be used now");
    assert_eq!(p.until, 800);
    assert_eq!(Horizon::phase(p, 0, 10000., true), 0.2);
    assert!(buys(&first) > 0);
    o.tick = 101;
    o.news.clear();
    o.books[0].dividend = 60;
    o.books[0].bids[0].price = 5990;
    o.books[0].asks[0].price = 6010;
    o.books[0].last = 6000;
    let after = bot.on_tick(&o);
    assert!(
        buys(&after) > buys(&first),
        "after the dip it must release more of its reserved budget"
    );
}

#[test]
fn cheap_price_unlocks_accumulation_even_before_bad_news_effective() {
    let p = Plan {
        premium: 1.,
        until: 800,
        adverse_until: 100,
        trough_price: 6000.,
    };
    assert_eq!(Horizon::phase(p, 20, 10000., true), 0.2);
    assert_eq!(Horizon::phase(p, 20, 6000., true), 0.65);
    assert_eq!(Horizon::phase(p, 700, 6000., true), 1.);
}

#[test]
fn publication_does_not_cancel_recovery_plan_or_force_exit() {
    let mut bot = Horizon::new(&start());
    let mut o = empty(0);
    o.news = vec![news(1, 80, 700, true)];
    bot.on_tick(&o);
    let anchor = bot.states[0].plan.unwrap().premium;
    o.tick = 640;
    o.news[0].forecast = false;
    o.news[0].public = true;
    let actions = bot.on_tick(&o);
    assert_eq!(bot.states[0].plan.unwrap().premium, anchor);
    assert!(buys(&actions) > 0);
    o.tick = 701;
    o.news.clear();
    o.books[0].dividend = 180;
    bot.on_tick(&o);
    assert!(bot.states[0].plan.is_some());
    o.tick = 801;
    bot.on_tick(&o);
    assert!(bot.states[0].plan.is_none());
}

#[test]
fn research_is_not_erased_by_repeated_market_calibration() {
    let mut bot = Horizon::new(&start());
    let mut o = empty(0);
    o.news = vec![news(1, 80, 900, true)];
    bot.on_tick(&o);
    let anchor = bot.states[0].plan.unwrap().premium;
    o.news.clear();
    for t in 1..400 {
        o.tick = t;
        bot.on_tick(&o);
    }
    assert_eq!(bot.states[0].plan.unwrap().premium, anchor);
    assert!(buys(&bot.on_tick(&o)) > 0);
}

#[test]
fn revised_bad_outlook_can_end_plan_instead_of_averaging_down_forever() {
    let mut bot = Horizon::new(&start());
    let mut o = empty(0);
    o.news = vec![news(1, 80, 700, true)];
    bot.on_tick(&o);
    assert!(bot.states[0].plan.is_some());
    o.tick = 1;
    o.news = vec![news(2, -300, 600, true)];
    bot.on_tick(&o);
    assert!(bot.states[0].plan.is_none());
}

#[test]
fn incomplete_path_does_not_create_fictitious_growth() {
    let mut bot = Horizon::new(&start());
    let mut o = empty(0);
    let plain = bot.path(0, &o, 30.).value;
    o.news = vec![news(1, -200, 100, true), news(2, 200, 700, true)];
    bot.knowledge.update(&o);
    assert!(bot.path(0, &o, 30.).value < plain);
}

#[test]
fn research_disabled_matches_titan_before_terminal_guard() {
    let mut s = start();
    s.params.insert("research".into(), 0.);
    let mut bot = Horizon::new(&s);
    let mut titan = crate::titan::Titan::new(&s);
    for t in 0..100 {
        let mut o = observation(t, true);
        if t == 1 {
            o.news.push(news(1, 30, 121, false));
        }
        assert_eq!(bot.on_tick(&o), titan.on_tick(&o));
    }
}

#[test]
fn research_disabled_ignores_forecasts_in_terminal_risk_too() {
    let mut s = start();
    s.params.insert("research".into(), 0.);
    let mut plain = observation(2800, false);
    plain.account.cash = -100_000;
    plain.account.equity = 2_000_000;
    plain.account.gross = 2_100_000;
    plain.account.shares = vec![110, 100];
    plain.account.sellable = plain.account.shares.clone();
    let mut forecasts = plain.clone();
    forecasts.news = vec![news(1, -10000, 2900, true), news(2, -10000, 2900, true)];
    forecasts.news[1].changes[0].stock = 1;
    assert_eq!(
        Horizon::new(&s).on_tick(&plain),
        Horizon::new(&s).on_tick(&forecasts)
    );
}

#[test]
fn terminal_debt_guard_can_sell_when_marked_leverage_is_healthy() {
    let mut s = start();
    s.rules.terminal_payments_min = 0;
    let mut o = observation(2800, false);
    o.trades.iter_mut().for_each(Vec::clear);
    o.terminal_hints = vec![(0, 0)];
    o.books.iter_mut().for_each(|b| b.dividend = 5000);
    o.account.cash = -100_000;
    o.account.equity = 2_000_000;
    o.account.gross = 2_100_000;
    o.account.shares = vec![110, 100];
    o.account.sellable = o.account.shares.clone();
    let actions = Horizon::new(&s).on_tick(&o);
    assert!(actions.iter().any(|a| matches!(
        a,
        Action::Limit {
            side: Side::Sell,
            tif: Tif::Ioc,
            ..
        }
    )));
    assert!(!actions.iter().any(|a| matches!(
        a,
        Action::Limit {
            side: Side::Buy,
            ..
        }
    )));
    s.params.insert("terminal_guard".into(), 0.);
    assert!(!Horizon::new(&s).on_tick(&o).iter().any(|a| matches!(
        a,
        Action::Limit {
            side: Side::Sell,
            tif: Tif::Ioc,
            ..
        }
    )));
}

#[test]
fn private_metadata_cannot_change_actions_and_insolvency_never_buys() {
    let s = start();
    let mut other = s.clone();
    other.seed = u64::MAX;
    other.account_index = 100;
    other.forecast_access = ForecastAccess {
        horizon: 1000,
        coverage: 1.,
        range_width: 0.,
    };
    other.account = "different".into();
    other.owner = "different".into();
    let (mut a, mut b) = (Horizon::new(&s), Horizon::new(&other));
    for t in 0..100 {
        let mut o = empty(t);
        if t == 0 {
            o.news = vec![news(1, -40, 100, true), news(2, 80, 700, true)];
        }
        assert_eq!(a.on_tick(&o), b.on_tick(&o));
    }
    let mut o = empty(100);
    o.account.equity = -1;
    assert_eq!(buys(&a.on_tick(&o)), 0);
}
