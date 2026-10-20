use bot_api::*;
use market::config::Config;

/// Two stocks from the v13 scenario, both starting at 100 rate units, no combos.
fn start() -> Start {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../scenarios/baseline-v13.toml"
    );
    let mut cfg = Config::load(std::path::Path::new(path)).unwrap();
    cfg.stocks.truncate(2);
    for s in &mut cfg.stocks {
        s.dividend = 100;
    }
    cfg.combos.clear();
    Start {
        account: "test".into(),
        owner: "test".into(),
        account_index: 0,
        n_accounts: 10,
        news_multiplier: 1.0,
        forecast_access: Default::default(),
        seed: 1,
        cash: 2_000_000,
        shares: vec![20, 20],
        rules: cfg.rules(&[10000, 10000]),
        params: Default::default(),
    }
}

/// Both books priced at the flat value of their own dividend, plus `premium`.
fn observation(start: &Start, tick: Tick, dividends: [Money; 2], premium: f64) -> Observation {
    let r = &start.rules;
    let terminal = r.terminal_dividend_payments(
        (r.terminal_payments_min + r.terminal_payments_max) as f64 / 2.0,
    );
    let left = (r.ticks_total - 1 - tick) as f64;
    let books = dividends
        .iter()
        .map(|&d| {
            let p = (r.dividend_cents(d) * (left + terminal) * (1.0 + premium)) as Price;
            BookView {
                bids: vec![Level {
                    price: p - 5,
                    qty: 1000,
                }],
                asks: vec![Level {
                    price: p + 5,
                    qty: 1000,
                }],
                last: p,
                mark: p,
                dividend: d,
            }
        })
        .collect::<Vec<_>>();
    let gross: Money = books.iter().map(|b| b.mark * 20).sum();
    Observation {
        tick,
        ticks_total: r.ticks_total,
        reference_tick: tick,
        season: Season {
            turbulent: false,
            rate_mult: 1.0,
            size_mult: 1.0,
        },
        account: AccountView {
            cash: 2_000_000,
            shares: vec![20, 20],
            sellable: vec![20, 20],
            equity: 2_000_000 + gross,
            gross,
            leverage: gross as f64 / (2_000_000 + gross) as f64,
            buying_power: 10_000_000,
            ..Default::default()
        },
        open_orders: vec![],
        books,
        trades: vec![vec![], vec![]],
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

fn orders(actions: &[Action], stock: usize, side: Side) -> Qty {
    actions
        .iter()
        .map(|a| match a {
            Action::Limit {
                stock: s,
                side: d,
                qty,
                ..
            } if *s == stock && *d == side => *qty,
            _ => 0,
        })
        .sum()
}

#[test]
fn buys_the_stock_whose_dividend_fell_and_sells_the_one_that_rose() {
    let s = start();
    let mut bot = bots::factory("tide", &s).unwrap();
    // Both trade 10% above the flat value of today's dividend. Only the fallen one
    // is worth more once its expected recovery is counted.
    let acts = bot.on_tick(&observation(&s, 2000, [40, 250], 0.1));
    assert!(orders(&acts, 0, Side::Buy) > 0, "{acts:?}");
    assert_eq!(orders(&acts, 0, Side::Sell), 0);
    assert!(orders(&acts, 1, Side::Sell) > 0, "{acts:?}");
    assert_eq!(orders(&acts, 1, Side::Buy), 0);
}

#[test]
fn sells_at_the_end_when_the_recovery_has_no_time_left_to_pay() {
    let s = start();
    let mut bot = bots::factory("tide", &s).unwrap();
    let early = bot.on_tick(&observation(&s, 2000, [40, 40], 0.1));
    assert!(orders(&early, 0, Side::Buy) > 0, "{early:?}");
    let mut bot = bots::factory("tide", &s).unwrap();
    let mut late = observation(&s, s.rules.ticks_total - 5, [40, 40], 0.1);
    // The terminal range is known exactly by now.
    let n = s.rules.terminal_payments_min;
    late.terminal_hints = vec![(n, n)];
    let acts = bot.on_tick(&late);
    assert_eq!(orders(&acts, 0, Side::Buy), 0, "{acts:?}");
    assert!(orders(&acts, 0, Side::Sell) > 0, "{acts:?}");
}
