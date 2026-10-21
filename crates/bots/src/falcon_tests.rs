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
    let (mut left, mut right) = (Falcon::new(&a), Falcon::new(&b));
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
fn execution_respects_total_budget_and_available_shares() {
    let mut bot = Falcon::new(&start());
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
    let mut bot = Falcon::new(&start());
    let mut o = observation(100, true);
    o.books[0].asks[0].price = 12000;
    o.books[1].asks.clear();
    assert!(bot.on_tick(&o).is_empty());
}

#[test]
fn supply_quotes_are_backed_and_do_not_cross_each_other() {
    let mut s = start();
    s.params.insert("mode".into(), 3.0);
    let mut bot = Falcon::new(&s);
    let mut o = observation(100, true);
    o.account.buying_power = 100_000;
    let orders = bot.on_tick(&o);
    let mut spend = 0;
    let mut sells = [0; 2];
    for a in &orders {
        if let Action::Limit {
            stock,
            side,
            price,
            qty,
            ..
        } = *a
        {
            if side == Side::Sell {
                sells[stock] += qty;
            } else {
                spend += (price + (price * 5 + 9999) / 10000) * qty;
            }
        }
    }
    assert!(spend > 0 && spend <= o.account.buying_power);
    assert!(sells.iter().zip(&o.account.sellable).all(|(a, b)| a <= b));
    assert!(orders.len() <= s.rules.max_actions_per_tick);
    o.open_orders.push(OpenOrder {
        order_id: 42,
        stock: 0,
        side: Side::Sell,
        price: 10010,
        remaining: 20,
        placed_tick: 97,
    });
    let orders = bot.on_tick(&o);
    assert!(orders.contains(&Action::CancelAll { stock: Some(0) }));
    assert!(!orders
        .iter()
        .any(|a| matches!(a, Action::Limit { stock: 0, .. })));
}

#[test]
fn all_modes_ignore_identity_and_private_information() {
    for mode in 0..=9 {
        let mut a = start();
        a.params.insert("mode".into(), mode as f64);
        let mut b = a.clone();
        b.seed = 987654321;
        b.account = "unknown".into();
        b.owner = "another".into();
        b.account_index = 99;
        b.n_accounts = 1024;
        b.rules.stocks[0].initial_dividend = 99999;
        b.rules.terminal_payments_min = 900;
        let (mut left, mut right) = (Falcon::new(&a), Falcon::new(&b));
        for tick in 0..180 {
            let o = observation(tick, tick % 90 < 45);
            let mut p = o.clone();
            p.books[0].dividend = 99999;
            p.terminal_hints = vec![(900, 1000)];
            p.news = vec![NewsItem {
                id: tick,
                layer: NewsLayer::Market,
                sector: None,
                changes: vec![Change {
                    stock: 0,
                    low: -9999,
                    high: 9999,
                }],
                stage: 0,
                forecast: false,
                first_reveal_tick: tick,
                public_tick: tick + 20,
                effective_tick: tick + 40,
                public: false,
            }];
            assert_eq!(
                left.on_tick(&o),
                right.on_tick(&p),
                "mode {mode}, tick {tick}"
            );
        }
    }
}

#[test]
fn extra_cash_does_not_expand_a_fixed_trading_allocation() {
    for mode in [0, 4, 5, 6, 7] {
        let mut s = start();
        s.params.insert("mode".into(), mode as f64);
        s.params.insert("capital_limit".into(), 2_000_000.0);
        let (mut small, mut large) = (Falcon::new(&s), Falcon::new(&s));
        for tick in 0..180 {
            let o = observation(tick, tick % 90 < 45);
            let mut rich = o.clone();
            rich.account.cash += 100_000_000;
            rich.account.equity += 100_000_000;
            rich.account.buying_power += 100_000_000;
            assert_eq!(small.on_tick(&o), large.on_tick(&rich), "mode {mode}");
        }
    }
}

#[test]
fn allocation_cap_and_actual_equity_both_constrain_spending() {
    for equity in [2_400_000, 500_000] {
        let mut s = start();
        s.params.insert("mode".into(), 5.0);
        s.params.insert("capital_limit".into(), 1_000_000.0);
        s.params.insert("cap".into(), 1.0);
        let mut bot = Falcon::new(&s);
        let mut o = observation(100, true);
        o.account.equity = equity;
        let orders = bot.on_tick(&o);
        let spend: i64 = orders
            .iter()
            .map(|a| match a {
                Action::Limit {
                    side: Side::Buy,
                    price,
                    qty,
                    ..
                } => (price + (price * 5 + 9999) / 10000) * qty,
                _ => 0,
            })
            .sum();
        assert!(spend > 0);
        assert!(spend <= equity.min(1_000_000) - o.account.gross);
    }
}

#[test]
fn larger_pulse_uses_actual_cash_and_stock_not_an_invented_endowment() {
    let mut s = start();
    s.params.insert("mode".into(), 6.0);
    s.params.insert("pulse_qty".into(), 400.0);
    let mut bot = Falcon::new(&s);
    let mut o = observation(100, false);
    o.account.cash = 1_000_000_000;
    o.account.equity = 1_000_400_000;
    o.account.buying_power = 1_000_000_000;
    let buys = bot.on_tick(&o);
    assert_eq!(buys.len(), 2);
    assert!(buys.iter().all(|a| matches!(
        a,
        Action::Limit {
            side: Side::Buy,
            qty: 400,
            ..
        }
    )));
    // A positive signal switches the relay to selling, but it still only owns
    // twenty shares per stock. Increasing the pulse cannot manufacture stock.
    let mut sell_bot = Falcon::new(&s);
    let sell_obs = observation(100, true);
    let sells = sell_bot.on_tick(&sell_obs);
    assert_eq!(sells.len(), 2);
    assert!(sells.iter().all(|a| matches!(
        a,
        Action::Limit {
            side: Side::Sell,
            qty: 20,
            ..
        }
    )));
    let mut constrained = Falcon::new(&s);
    o.account.buying_power = 100_000;
    let spend: i64 = constrained
        .on_tick(&o)
        .iter()
        .map(|a| match a {
            Action::Limit {
                side: Side::Buy,
                price,
                qty,
                ..
            } => (price + (price * 5 + 9999) / 10000) * qty,
            _ => 0,
        })
        .sum();
    assert!(spend > 0 && spend <= o.account.buying_power);
}

#[test]
fn explicit_default_pulse_preserves_existing_modes() {
    for mode in [6, 7, 8, 9] {
        let mut a = start();
        a.params.insert("mode".into(), mode as f64);
        let mut b = a.clone();
        b.params.insert("pulse_qty".into(), 4.0);
        let (mut left, mut right) = (Falcon::new(&a), Falcon::new(&b));
        for tick in 0..180 {
            let o = observation(tick, tick % 90 < 45);
            assert_eq!(left.on_tick(&o), right.on_tick(&o));
        }
    }
}
