use bot_api::*;
use std::collections::BTreeMap;

pub fn start() -> Start {
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

pub fn observation(tick: Tick, buy: bool) -> Observation {
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
