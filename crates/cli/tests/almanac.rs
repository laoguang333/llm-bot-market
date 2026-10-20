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
        account: "almanac".into(),
        owner: "almanac".into(),
        account_index: 0,
        n_accounts: 100,
        news_multiplier: 1.0,
        forecast_access: ForecastAccess::default(),
        seed: 1,
        params: BTreeMap::new(),
        cash: 2_000_000,
        shares: vec![20; rules.stocks.len()],
        rules,
    }
}

/// Every stock quoted at 30,000/30,020 with deep books and a 2-cent dividend.
fn observation(s: &Start, tick: Tick, shares: Vec<Qty>, cash: Money) -> Observation {
    let n = s.rules.stocks.len();
    let book = |bid: Price, ask: Price| BookView {
        bids: (0..5)
            .map(|k| Level {
                price: bid - k,
                qty: 1_000,
            })
            .collect(),
        asks: (0..5)
            .map(|k| Level {
                price: ask + k,
                qty: 1_000,
            })
            .collect(),
        last: bid,
        mark: (bid + ask) / 2,
        dividend: 100,
    };
    let books: Vec<BookView> = (0..n).map(|_| book(30_000, 30_020)).collect();
    let gross: Money = shares.iter().map(|q| q * 30_010).sum();
    let equity = cash + gross;
    Observation {
        tick,
        ticks_total: s.rules.ticks_total,
        reference_tick: tick,
        season: Season {
            turbulent: false,
            rate_mult: 1.0,
            size_mult: 1.0,
        },
        account: AccountView {
            cash,
            sellable: shares.clone(),
            shares,
            equity,
            gross,
            leverage: gross as f64 / equity as f64,
            buying_power: (4 * equity - gross).max(0),
            reserved_buy: 0,
            idle: IdleEstimate {
                period_end_tick: (tick / 50 + 1) * 50,
                avg_equity: equity,
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

fn orders(acts: &[Action]) -> Vec<(usize, Side, Price, Qty)> {
    acts.iter()
        .filter_map(|a| match a {
            Action::Limit {
                stock,
                side,
                price,
                qty,
                ..
            } => Some((*stock, *side, *price, *qty)),
            _ => None,
        })
        .collect()
}

fn bought(acts: &[Action], stock: usize) -> Qty {
    orders(acts)
        .iter()
        .filter(|o| o.0 == stock && o.1 == Side::Buy)
        .map(|o| o.3)
        .sum()
}

fn sold(acts: &[Action], stock: usize) -> Qty {
    orders(acts)
        .iter()
        .filter(|o| o.0 == stock && o.1 == Side::Sell)
        .map(|o| o.3)
        .sum()
}

#[test]
fn public_news_tilts_toward_the_part_not_yet_priced() {
    let s = start();
    let mut bot = bots::factory("almanac", &s).unwrap();
    // Mid game, every stock already at the 10% base weight.
    let base_shares = 40;
    let mut obs = observation(&s, 5000, vec![base_shares; 10], 0);
    obs.account.cash = obs.account.gross / 9;
    obs.account.equity = obs.account.cash + obs.account.gross;
    obs.news.push(NewsItem {
        id: 7,
        layer: NewsLayer::Stock,
        sector: None,
        changes: vec![Change {
            stock: 3,
            low: 20,
            high: 20,
        }],
        stage: 2,
        forecast: false,
        first_reveal_tick: 4940,
        public_tick: 5000,
        effective_tick: 5060,
        public: true,
    });
    let acts = bot.on_tick(&obs);
    assert!(bought(&acts, 3) > 0, "{acts:?}");
    for k in (0..10).filter(|&k| k != 3) {
        assert_eq!(bought(&acts, k), 0, "stock {k}: {acts:?}");
    }

    // Long after the change took effect the tilt is gone and the stock is sold back.
    let mut obs = observation(&s, 5400, vec![base_shares; 10], 0);
    obs.account.shares[3] = 3 * base_shares;
    obs.account.sellable[3] = 3 * base_shares;
    obs.account.gross += 2 * base_shares * 30_010;
    obs.account.cash = obs.account.gross / 9;
    obs.account.equity = obs.account.cash + obs.account.gross;
    obs.account.leverage = obs.account.gross as f64 / obs.account.equity as f64;
    let acts = bot.on_tick(&obs);
    assert!(sold(&acts, 3) > 0, "{acts:?}");
}

#[test]
fn end_game_sells_stocks_priced_above_their_payout() {
    let s = start();
    let mut bot = bots::factory("almanac", &s).unwrap();
    // A 2-cent dividend with ~500 ticks and N <= 167 left is worth far below 300 yuan.
    let obs = observation(&s, 9500, vec![50; 10], 1_000_000);
    let acts = bot.on_tick(&obs);
    for k in 0..10 {
        assert_eq!(bought(&acts, k), 0);
    }
    let total: Qty = (0..10).map(|k| sold(&acts, k)).sum();
    assert!(total > 0, "{acts:?}");
    for k in 0..10 {
        assert!(sold(&acts, k) <= 50);
    }
}

#[test]
fn idle_shortfall_is_topped_up_with_a_round_trip_before_settlement() {
    let s = start();
    let mut bot = bots::factory("almanac", &s).unwrap();
    // Weights already on target mid game, so only the top-up trades.
    let mut obs = observation(&s, 5045, vec![40; 10], 0);
    obs.account.cash = obs.account.gross / 9;
    obs.account.equity = obs.account.cash + obs.account.gross;
    obs.account.idle.fee = 100_000;
    let acts = bot.on_tick(&obs);
    let o = orders(&acts);
    assert!(!o.is_empty());
    let sells: Qty = o.iter().filter(|x| x.1 == Side::Sell).map(|x| x.3).sum();
    let buys: Qty = o.iter().filter(|x| x.1 == Side::Buy).map(|x| x.3).sum();
    assert_eq!(sells, buys);
    // About 10,000,000 cents of volume is missing: two legs of ~5,000,000 each.
    assert!(sells * 30_000 >= 5_000_000, "{acts:?}");
    for k in 0..10 {
        assert!(sold(&acts, k) <= 40);
    }

    // Outside the top-up window nothing trades.
    let mut obs = observation(&s, 5010, vec![40; 10], 0);
    obs.account.cash = obs.account.gross / 9;
    obs.account.equity = obs.account.cash + obs.account.gross;
    obs.account.idle.fee = 100_000;
    assert!(orders(&bot.on_tick(&obs)).is_empty());
}

#[test]
fn buys_stay_within_buying_power_early() {
    let s = start();
    let mut bot = bots::factory("almanac", &s).unwrap();
    let mut obs = observation(&s, 100, vec![20; 10], 2_000_000);
    obs.account.buying_power = 1_000_000;
    let acts = bot.on_tick(&obs);
    let cost: i64 = orders(&acts)
        .iter()
        .filter(|o| o.1 == Side::Buy)
        .map(|o| o.2 * o.3)
        .sum();
    assert!(cost > 0 && cost <= 1_000_000, "{cost}");
}
