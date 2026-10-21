use super::*;
use std::collections::BTreeMap;

fn start() -> Start {
    let layer = NewsLayerRules {
        events_per_1000_ticks: 1.0,
        change_sd: 0.1,
    };
    Start {
        account: "test".into(),
        owner: "test".into(),
        account_index: 0,
        n_accounts: 2,
        news_multiplier: 1.0,
        forecast_access: Default::default(),
        seed: 1,
        params: BTreeMap::new(),
        cash: 2_000_000,
        shares: vec![20; 2],
        rules: Rules {
            stocks: vec![
                StockInfo {
                    name: "S".into(),
                    sector: 0,
                    shares_outstanding: 1000,
                    initial_dividend: 100,
                    initial_price: 10000,
                    market_loading: 1.0,
                    sector_loading: 1.0
                };
                2
            ],
            sectors: vec!["sector".into()],
            combos: vec![],
            ticks_total: 3000,
            tick_size: 1,
            dividend_period: 50,
            dividend_scale: 1,
            combo_period: 50,
            combo_scale: 1,
            terminal_payment_ticks: 50,
            min_dividend: 0,
            trade_fee_bps: 5,
            idle_period: 50,
            idle_rate: 0.002,
            credit_ratio: 0.01,
            credit_side: "both".into(),
            max_leverage: 4.0,
            maintenance_leverage: 5.0,
            loan_rate: 0.003,
            interest_period: 50,
            mark_window: 20,
            max_actions_per_tick: 20,
            max_open_orders: 40,
            book_depth: 10,
            news_market: layer.clone(),
            news_sector: layer.clone(),
            news_stock: layer,
            news_lead_ticks: 120,
            news_stages: vec![],
            news_range_widths: vec![],
            dragon: None,
            dividend_risk: None,
            random_end: None,
            season_calm_to_turbulent: 0.002,
            season_turbulent_to_calm: 0.01,
            season_turbulent_rate_mult: 3.0,
            season_turbulent_size_mult: 2.0,
            max_tick_skew: 16,
            skip_decision_lag: 2,
            decision_budget_us: 2000,
            terminal_ticks: 1500,
            terminal_payments_min: 10,
            terminal_payments_max: 50,
            terminal_hints: vec![(0, 30)],
            objective: "rank_percentile".into(),
        },
    }
}

fn observation(tick: Tick, buy: bool) -> Observation {
    Observation {
        tick,
        ticks_total: 3000,
        reference_tick: tick,
        season: Season {
            turbulent: false,
            rate_mult: 1.0,
            size_mult: 1.0,
        },
        account: AccountView {
            cash: 2_000_000,
            shares: vec![20; 2],
            sellable: vec![20; 2],
            equity: 2_400_000,
            gross: 400_000,
            buying_power: 9_000_000,
            ..AccountView::default()
        },
        open_orders: vec![],
        books: vec![
            BookView {
                bids: vec![Level {
                    price: 9990,
                    qty: 100
                }],
                asks: vec![Level {
                    price: 10010,
                    qty: 100
                }],
                last: 10000,
                mark: 10000,
                dividend: 100
            };
            2
        ],
        trades: vec![
            vec![Trade {
                tick,
                price: 10000,
                qty: 20,
                aggressor: if buy { Side::Buy } else { Side::Sell }
            }];
            2
        ],
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

#[test]
fn fundamental_channels_cannot_change_orders() {
    let a = start();
    let mut b = a.clone();
    b.seed = 123456789;
    b.account_index = 42;
    b.n_accounts = 100;
    b.rules.stocks.iter_mut().for_each(|s| {
        s.initial_dividend = 99999;
        s.initial_price = 1;
        s.market_loading = 100.0;
        s.sector_loading = 100.0;
        s.shares_outstanding = 1000000;
        s.sector = 99;
    });
    b.rules.combos = vec![ComboRule {
        a: 0,
        a_units: 1,
        b: 1,
        b_units: 1,
        bonus: 123456,
        start_tick: 0,
        end_tick: 3000,
    }];
    b.rules.dividend_period = 1;
    b.rules.min_dividend = 9999;
    b.rules.terminal_ticks = 1;
    b.rules.terminal_payments_min = 500;
    b.rules.terminal_payments_max = 1000;
    b.rules.terminal_hints = vec![(1, 0)];
    b.rules.news_market.change_sd = 100.0;
    b.rules.news_sector.change_sd = 100.0;
    b.rules.news_stock.change_sd = 100.0;
    b.rules.news_stages = vec![(0, 1.0)];
    let (mut left, mut right) = (Swift::new(&a), Swift::new(&b));
    let (mut buys, mut sells) = (0, 0);
    for t in 0..250 {
        let o = observation(t, t % 100 < 50);
        let mut changed = o.clone();
        for book in &mut changed.books {
            book.dividend = (t as i64 + 1) * 9999;
        }
        changed.news = vec![NewsItem {
            id: t,
            layer: NewsLayer::Market,
            sector: None,
            changes: vec![Change {
                stock: 0,
                low: -99999,
                high: 99999,
            }],
            stage: 0,
            forecast: false,
            first_reveal_tick: t,
            public_tick: t + 10,
            effective_tick: t + 20,
            public: false,
        }];
        changed.terminal_hints = vec![(500, 1000)];
        changed.income = vec![
            Income::Dividend {
                stock: 0,
                amount: 999999,
            },
            Income::Combo {
                rule: 0,
                amount: -999999,
            },
        ];
        changed.season = Season {
            turbulent: true,
            rate_mult: 100.0,
            size_mult: 100.0,
        };
        changed.season_changes = vec![(t, true)];
        let orders = left.on_tick(&o);
        assert_eq!(
            orders,
            right.on_tick(&changed),
            "fundamental leakage at tick {t}"
        );
        for order in orders {
            if let Action::Limit { side, .. } = order {
                if side == Side::Buy {
                    buys += 1;
                } else {
                    sells += 1;
                }
            }
        }
    }
    assert!(
        buys > 0 && sells > 0,
        "invariance must exercise actual trading"
    );
}

#[test]
fn single_entry_waits_for_flat_and_finishes_partial_exits() {
    let mut s = start();
    s.params.insert("single_entry".into(), 1.0);
    let mut bot = Swift::new(&s);
    let mut o = observation(100, true);
    o.account.shares = vec![0, 20];
    let entry = bot.on_tick(&o);
    assert!(entry.iter().any(|a| matches!(
        a,
        Action::Limit {
            stock: 0,
            side: Side::Buy,
            ..
        }
    )));
    assert!(!entry.iter().any(|a| matches!(
        a,
        Action::Limit {
            stock: 1,
            side: Side::Buy,
            ..
        }
    )));

    // Zero fills may retry. One share filled locks this position, even if far
    // below the target. A large equity change cannot cause a trim either.
    o.tick += 1;
    assert!(!bot.on_tick(&o).is_empty());
    o.tick += 1;
    o.account.shares = vec![1, 20];
    o.account.sellable = vec![1, 20];
    assert!(bot.on_tick(&o).is_empty());
    o.tick += 1;
    o.account.equity = 1;
    assert!(bot.on_tick(&o).is_empty());

    // A decisive sell signal starts the exit, but only one share is sellable.
    o = observation(104, false);
    o.trades.iter_mut().for_each(|ts| ts[0].qty = 10_000);
    o.account.sellable = vec![1, 0];
    assert_eq!(bot.on_tick(&o), vec![Action::ioc(0, Side::Sell, 9990, 1)]);
    o = observation(105, true);
    o.trades.iter_mut().for_each(|ts| ts[0].qty = 100_000);
    o.account.shares = vec![19, 20];
    o.account.sellable = vec![19, 20];
    let exit = bot.on_tick(&o);
    assert!(exit.contains(&Action::ioc(0, Side::Sell, 9990, 19)));
    assert!(exit.iter().all(|a| matches!(
        a,
        Action::Limit {
            side: Side::Sell,
            ..
        }
    )));
    o.tick += 1;
    o.account.shares = vec![0, 20];
    let reentry = bot.on_tick(&o);
    assert!(reentry.iter().any(|a| matches!(
        a,
        Action::Limit {
            stock: 0,
            side: Side::Buy,
            ..
        }
    )));
    assert!(reentry.contains(&Action::ioc(1, Side::Sell, 9990, 20)));
}

#[test]
fn default_swift_still_tops_up_and_resizes_positions() {
    let mut bot = Swift::new(&start());
    let mut o = observation(100, true);
    assert!(bot.on_tick(&o).iter().all(|a| matches!(
        a,
        Action::Limit {
            side: Side::Buy,
            ..
        }
    )));
    o.tick += 1;
    o.account.equity = 100_000;
    assert!(bot.on_tick(&o).iter().all(|a| matches!(
        a,
        Action::Limit {
            side: Side::Sell,
            ..
        }
    )));
}

#[test]
fn execution_respects_total_budget_and_available_shares() {
    let mut bot = Swift::new(&start());
    let mut o = observation(100, true);
    o.account.buying_power = 100_000;
    let orders = bot.on_tick(&o);
    let mut spend = 0;
    for a in orders {
        if let Action::Limit {
            side: Side::Buy,
            price,
            qty,
            tif,
            ..
        } = a
        {
            assert_eq!(tif, Tif::Ioc);
            assert_eq!(price, 10010);
            spend += (price + (price * 5 + 9999) / 10000) * qty;
        }
    }
    assert!(spend > 0 && spend <= o.account.buying_power);
    o.tick = 2990;
    o.account.sellable = vec![3, 0];
    let orders = bot.on_tick(&o);
    assert_eq!(orders, vec![Action::ioc(0, Side::Sell, 9990, 3)]);
}

#[test]
fn empty_books_and_wide_spreads_do_not_trigger_unpriced_buys() {
    let mut bot = Swift::new(&start());
    let mut o = observation(100, true);
    o.books[0].asks[0].price = 12000;
    o.books[1].asks.clear();
    assert!(bot.on_tick(&o).is_empty());
}

#[test]
fn independent_changes_match_the_sorted_payment_timeline() {
    use crate::common::{hold_value, payments_after};
    // Include coincident changes, exact payment boundaries, old changes,
    // another stock, and changes too late to affect terminal settlement.
    let rules = start().rules;
    let pending = [
        (0, 20, 50),
        (0, -7, 50),
        (1, 100, 60),
        (0, 3, 0),
        (0, 11, 49),
        (0, -2, 51),
        (0, 30, rules.ticks_total - 1),
        (0, 999, rules.ticks_total),
        (0, 999, Tick::MAX),
    ];
    for tick in [0, 1, 49, 50, 51, rules.ticks_total - 1] {
        for terminal in [0.0, 10.0, 17.5, 13.37] {
            let mut changes: Vec<_> = pending
                .iter()
                .filter(|p| p.0 == 0)
                .map(|p| (p.2, p.1))
                .collect();
            changes.sort_unstable();
            let (mut d, mut from, mut expected) = (100.0, tick, 0.0);
            for (eff, delta) in changes {
                if eff >= rules.ticks_total {
                    break;
                }
                expected += d * payments_after(&rules, from, eff.saturating_sub(1)) as f64;
                from = from.max(eff.saturating_sub(1));
                d += delta as f64;
            }
            expected += d * payments_after(&rules, from, rules.ticks_total) as f64 + d * terminal;
            for entries in [pending.to_vec(), pending.iter().rev().copied().collect()] {
                let actual = hold_value(&rules, tick, 0, 100, &entries, terminal);
                assert!(
                    (actual - expected).abs() < 1e-8,
                    "tick={tick}, {actual} != {expected}"
                );
            }
        }
    }
}
