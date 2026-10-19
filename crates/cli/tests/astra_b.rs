use bot_api::*;
use market::config::Config;

fn fixture(params: &[(&str, f64)], shares: Vec<Qty>) -> (Box<dyn Bot>, Observation) {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../scenarios/baseline-v6-five-swift.toml"
    );
    let mut cfg = Config::load(std::path::Path::new(path)).unwrap();
    cfg.stocks.truncate(2);
    cfg.combos.truncate(1);
    let start = Start {
        account: "test".into(),
        owner: "test".into(),
        account_index: 0,
        n_accounts: 10,
        news_multiplier: 1.0,
        seed: 1,
        cash: 2_000_000,
        shares: shares.clone(),
        rules: cfg.rules(&[10000, 10000]),
        params: params.iter().map(|(k, v)| (k.to_string(), *v)).collect(),
    };
    let obs = Observation {
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
            shares: shares.clone(),
            sellable: shares,
            equity: 4_000_000,
            gross: 400_000,
            buying_power: 8_000_000,
            ..Default::default()
        },
        books: vec![
            BookView {
                bids: vec![Level {
                    price: 15000,
                    qty: 100
                }],
                asks: vec![Level {
                    price: 15002,
                    qty: 100
                }],
                last: 15001,
                mark: 15001,
                dividend: 100
            };
            2
        ],
        open_orders: vec![],
        trades: vec![vec![]; 2],
        skipped: 0,
        fills: vec![],
        acks: vec![],
        rejects: vec![],
        news: vec![],
        season_changes: vec![],
        income: vec![],
        terminal_hints: vec![],
    };
    (bots::factory("astra_b", &start).unwrap(), obs)
}

fn bid(actions: &[Action], stock: usize) -> Price {
    actions
        .iter()
        .find_map(|a| match a {
            Action::Limit {
                stock: s,
                side: Side::Buy,
                price,
                tif: Tif::Gtc,
                ..
            } if *s == stock => Some(*price),
            _ => None,
        })
        .expect("resting buy quote")
}

#[test]
fn balanced_pair_can_be_started_without_existing_inventory() {
    let quote = |balance| {
        let (mut bot, obs) = fixture(
            &[("balance", balance), ("combo", 0.5), ("edge", 100.0)],
            vec![0, 0],
        );
        bot.on_tick(&obs)
    };
    let plain = quote(0.0);
    let prospective = quote(1.0);
    for stock in 0..2 {
        assert!(bid(&prospective, stock) > bid(&plain, stock));
    }
}

#[test]
fn historical_premium_cannot_rebuy_above_the_immediate_sell_trigger() {
    let quote = |coherence| {
        let (mut bot, mut obs) = fixture(
            &[("coherence", coherence), ("combo", 0.0), ("converge", 0.0)],
            vec![0, 0],
        );
        for tick in (0..800).step_by(5) {
            obs.tick = tick;
            bot.on_tick(&obs);
        }
        obs.tick = 800;
        bot.on_tick(&obs)
    };
    let unsafe_bid = bid(&quote(0.0), 0);
    let safe_bid = bid(&quote(1.0), 0);
    // 43 in-game payments plus the public terminal upper bound of 50.
    let immediate_sell_trigger = 100.0 * (43.0 + 50.0) * 1.02;
    assert!(unsafe_bid as f64 > immediate_sell_trigger);
    assert!((safe_bid as f64) < immediate_sell_trigger);
}
