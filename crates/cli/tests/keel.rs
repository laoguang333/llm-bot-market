//! Behaviour checks for Keel on hand-built observations of the default scenario.

use bot_api::*;
use market::config::Config;
use std::collections::BTreeMap;
use std::path::Path;

fn start() -> Start {
    let cfg =
        Config::load(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scenarios/baseline.toml"))
            .unwrap();
    let rules = cfg.rules(&vec![10_000; cfg.stocks.len()]);
    Start {
        account: "keel".into(),
        owner: "keel".into(),
        account_index: 0,
        n_accounts: 1,
        news_multiplier: 1.0,
        forecast_access: ForecastAccess::default(),
        seed: 1,
        params: BTreeMap::new(),
        cash: 2_000_000,
        shares: vec![20; rules.stocks.len()],
        rules,
    }
}

/// Every stock quoted at `price` +- 0.1% with deep books and its initial dividend.
fn observation(start: &Start, tick: Tick, price: Price, shares: Qty, cash: Money) -> Observation {
    let n = start.rules.stocks.len();
    let books = start
        .rules
        .stocks
        .iter()
        .map(|s| BookView {
            bids: vec![Level {
                price: price - price / 1000,
                qty: 1_000,
            }],
            asks: vec![Level {
                price: price + price / 1000,
                qty: 1_000,
            }],
            last: price,
            mark: price,
            dividend: s.initial_dividend,
        })
        .collect();
    let gross = shares * price * n as i64;
    Observation {
        tick,
        ticks_total: start.rules.ticks_total,
        reference_tick: tick,
        season: Season {
            turbulent: false,
            rate_mult: 1.0,
            size_mult: 1.0,
        },
        account: AccountView {
            cash,
            shares: vec![shares; n],
            sellable: vec![shares; n],
            equity: cash + gross,
            gross,
            leverage: gross as f64 / (cash + gross).max(1) as f64,
            buying_power: ((cash + gross) * 4 - gross).max(0),
            reserved_buy: 0,
            idle: IdleEstimate {
                period_end_tick: (tick / 50 + 1) * 50,
                avg_equity: cash + gross,
                fee: 0,
                credit: 0,
                due: 0,
            },
            interest_estimate: 0,
        },
        open_orders: vec![],
        books,
        trades: vec![vec![]; n],
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

fn limits(acts: &[Action]) -> Vec<(usize, Side, Price, Qty, Tif)> {
    acts.iter()
        .map(|a| match a {
            Action::Limit {
                stock,
                side,
                price,
                qty,
                tif,
                ..
            } => (*stock, *side, *price, *qty, *tif),
            other => panic!("unexpected action {other:?}"),
        })
        .collect()
}

#[test]
fn early_game_levers_up_evenly_with_ioc_only() {
    let s = start();
    let mut bot = bots::factory("keel", &s).unwrap();
    let obs = observation(&s, 1, 30_000, 20, 2_000_000);
    let acts = limits(&bot.on_tick(&obs));
    assert_eq!(acts.len(), 10, "one buy per stock");
    let mut spend = 0;
    for (_, side, price, qty, tif) in &acts {
        assert_eq!(*side, Side::Buy);
        assert_eq!(*tif, Tif::Ioc, "never rests an order");
        spend += price * qty;
    }
    // Each buy is capped at 10% of equity per tick and all of them fit the buying power.
    assert!(spend <= obs.account.buying_power);
    assert!(spend as f64 > 0.5 * obs.account.equity as f64);
}

#[test]
fn rebalances_toward_equal_weights() {
    let s = start();
    let mut bot = bots::factory("keel", &s).unwrap();
    // A periodic rebalance tick, where small gaps are closed too.
    let mut obs = observation(&s, 2_000, 30_000, 60, -5_000_000);
    // Stock 0 doubled: it is now far above its share of the portfolio.
    obs.books[0].bids[0].price = 59_940;
    obs.books[0].asks[0].price = 60_060;
    obs.account.shares[0] = 60;
    let acts = limits(&bot.on_tick(&obs));
    let first = acts.iter().find(|a| a.0 == 0).expect("trades the outlier");
    assert_eq!(first.1, Side::Sell);
    assert!(acts.iter().filter(|a| a.0 != 0).all(|a| a.1 == Side::Buy));
}

#[test]
fn idle_fee_wash_round_trips_even_without_stock() {
    let s = start();
    let mut bot = bots::factory("keel", &s).unwrap();
    // Late game, every stock priced far above what holding it to the end can return:
    // the target is no stock at all, yet the credit still needs turnover.
    let mut obs = observation(&s, 9_995, 30_000, 0, 10_000_000);
    obs.account.idle.fee = 60_000;
    let acts = limits(&bot.on_tick(&obs));
    assert!(!acts.is_empty());
    assert_eq!(acts.len() % 2, 0);
    for pair in acts.chunks(2) {
        assert_eq!(pair[0].0, pair[1].0);
        assert_eq!(
            (pair[0].1, pair[1].1),
            (Side::Buy, Side::Sell),
            "buy first, sell back"
        );
        assert_eq!(pair[0].3, pair[1].3);
    }
}

#[test]
fn sells_stock_overpriced_against_holding_to_the_end() {
    let s = start();
    let mut bot = bots::factory("keel", &s).unwrap();
    // 500 ticks left on a periodic rebalance tick; 2 cents/tick for 500 ticks plus the
    // terminal payments is far below 300 yuan a share.
    let obs = observation(&s, 9_500, 30_000, 50, 1_000_000);
    let acts = limits(&bot.on_tick(&obs));
    assert!(!acts.is_empty());
    assert!(acts.iter().all(|a| a.1 == Side::Sell));
}
