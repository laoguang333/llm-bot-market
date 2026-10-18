//! Engine tests with scripted bots.

use bot_api::*;
use market::config::Config;
use market::engine::Shared;
use market::report::rank_percentiles;
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::Arc;

type Script = Box<dyn FnMut(&Observation) -> Vec<Action> + Send>;
type Log = Arc<Mutex<Vec<Observation>>>;

struct Scripted {
    script: Script,
    log: Log,
}

impl Bot for Scripted {
    fn on_tick(&mut self, obs: &Observation) -> Vec<Action> {
        self.log.lock().push(obs.clone());
        (self.script)(obs)
    }
}

/// Builds bots named in the config from scripts keyed by bot name; returns one log per name.
struct Harness {
    scripts: Mutex<HashMap<String, Vec<Script>>>,
    logs: HashMap<String, Log>,
}

impl Harness {
    fn new(scripts: Vec<(&str, Script)>) -> Self {
        let mut m: HashMap<String, Vec<Script>> = HashMap::new();
        let mut logs = HashMap::new();
        for (name, s) in scripts {
            m.entry(name.into()).or_default().push(s);
            logs.entry(name.to_string())
                .or_insert_with(|| Arc::new(Mutex::new(Vec::new())));
        }
        Self {
            scripts: Mutex::new(m),
            logs,
        }
    }

    fn factory(&self, name: &str, _s: &Start) -> Option<Box<dyn Bot>> {
        let script = self.scripts.lock().get_mut(name)?.remove(0);
        Some(Box::new(Scripted {
            script,
            log: self.logs[name].clone(),
        }))
    }

    fn log(&self, name: &str) -> Vec<Observation> {
        self.logs[name].lock().clone()
    }
}

fn at(tick: Tick, acts: Vec<Action>) -> Script {
    let mut acts = Some(acts);
    Box::new(move |o| {
        if o.tick == tick {
            acts.take().unwrap_or_default()
        } else {
            vec![]
        }
    })
}

fn idle() -> Script {
    Box::new(|_| vec![])
}

#[test]
fn observer_preserves_execution_and_sees_final_maker_delivery() {
    use market::account::Account;
    use market::engine::MatchObserver;
    #[derive(Default)]
    struct Observer {
        actions: Mutex<usize>,
        final_fills: Mutex<Vec<(usize, Fill)>>,
    }
    impl MatchObserver for Observer {
        fn step(
            &self,
            _: &Account,
            _: Tick,
            o: Option<&Observation>,
            actions: &[Action],
            _: Money,
        ) {
            assert!(o.is_some());
            *self.actions.lock() += actions.len();
        }
        fn finish(&self, a: &Account) {
            self.final_fills
                .lock()
                .extend(a.inbox.fills.iter().cloned().map(|f| (a.idx, f)));
        }
    }
    let cfg = config(
        3,
        "[fees]\nidle_rate=0\ntrade_fee_bps=0",
        &(group("seller", 0, 10) + &group("buyer", 100000, 0)),
    );
    let execute = |observer: Option<&dyn MatchObserver>| {
        let h = Harness::new(vec![
            ("seller", at(0, vec![Action::limit(0, Side::Sell, 1000, 3)])),
            ("buyer", at(2, vec![Action::limit(0, Side::Buy, 1000, 3)])),
        ]);
        market::report::run_match_observed(&cfg, 1, &|n, s| h.factory(n, s), None, observer)
            .unwrap()
    };
    let observer = Observer::default();
    let plain = execute(None);
    let traced = execute(Some(&observer));
    assert_eq!(*observer.actions.lock(), 2);
    assert!(observer
        .final_fills
        .lock()
        .iter()
        .any(|(idx, f)| *idx == 0 && f.maker && f.qty == 3));
    for (a, b) in plain.accounts.iter().zip(&traced.accounts) {
        assert_eq!(
            (a.cash, &a.shares, a.terminal_value),
            (b.cash, &b.shares, b.terminal_value)
        );
        assert_eq!(a.stats.trades, b.stats.trades);
    }
}

#[test]
fn idle_fee_reports_cash_and_destroyed_stock_components() {
    let cfg = config(
        11,
        "[fees]\nidle_period=10\nidle_rate=0.01\ntrade_fee_bps=0",
        &group("lazy", 100000, 100),
    );
    let h = Harness::new(vec![("lazy", idle())]);
    let r = market::report::run_match(&cfg, 1, &|n, s| h.factory(n, s), None).unwrap();
    let st = &r.accounts[0].stats;
    assert_eq!(st.idle_cash_fees, 1000);
    assert_eq!(st.idle_stock_fees, 2000);
    assert_eq!(st.idle_fees, st.idle_cash_fees + st.idle_stock_fees);
}

#[test]
fn per_tick_rounds_each_whole_position_without_carry_and_keeps_tick_zero_unpaid() {
    let mut cfg = config(6, "[fees]\nidle_rate=0", &group("passive", 100000, 10));
    cfg.world.dividend_period = 1;
    cfg.world.dividend_scale = 5; // rate 3 is 0.6 cent per share/tick
    cfg.stocks[0].dividend = 3;
    let mut large = cfg.bots[0].clone();
    large.bot = "large".into();
    large.shares_each = 10;
    let mut small = large.clone();
    small.bot = "small".into();
    small.shares_each = 1;
    cfg.bots = vec![large, small];
    let h = Harness::new(vec![("large", idle()), ("small", idle())]);
    let shared = Shared::new(&cfg, 7, &|n, s| h.factory(n, s)).unwrap();
    shared.run().unwrap();
    for (name, per_tick) in [("large", 6), ("small", 1)] {
        let log = h.log(name);
        assert!(log[0].income.is_empty());
        for o in &log[1..] {
            let paid: Money = o
                .income
                .iter()
                .filter_map(|i| match i {
                    Income::Dividend { stock: 0, amount } => Some(*amount),
                    _ => None,
                })
                .sum();
            assert_eq!(paid, per_tick);
            assert_eq!(
                o.account.cash - log[0].account.cash,
                per_tick * o.tick as i64
            );
        }
    }
    let rules = &shared.rules;
    assert_eq!(rules.dividend_payment(1, 2), 0); // 0.4 cent, no minimum payout
    assert_eq!(rules.dividend_payment(10, 3), 6); // never round each share first
    let mut halves = rules.clone();
    halves.dividend_scale = 10;
    assert_eq!(halves.dividend_payment(1, 5), 1);
    shared.audit().unwrap();
}

#[test]
fn per_tick_ordinary_preserves_combo_frequency_and_terminal_time_scale() {
    let mut old = config(101, "[fees]\nidle_rate=0", &group("passive", 100000, 10));
    old.stocks[0].dividend = 100;
    old.bots[0].shares_each = 10;
    old.scoring.terminal_payments = Some((2, 2));
    old.combos = vec![market::config::ComboCfg {
        a: "A".into(),
        a_units: 1,
        b: "B".into(),
        b_units: 1,
        bonus: 10,
        start_tick: 0,
        end_tick: None,
    }];
    let mut new = old.clone();
    new.world.dividend_period = 1;
    new.world.dividend_scale = 50;
    new.world.combo_period = Some(50);
    new.scoring.terminal_payment_ticks = Some(50);
    let passive = |_: &str, _: &Start| -> Option<Box<dyn Bot>> {
        Some(Box::new(Scripted {
            script: idle(),
            log: Arc::new(Mutex::new(Vec::new())),
        }))
    };
    let a = market::report::run_match(&old, 7, &passive, None).unwrap();
    let b = market::report::run_match(&new, 7, &passive, None).unwrap();
    assert_eq!(a.accounts[0].stats.dividends, 2000);
    assert_eq!(b.accounts[0].stats.dividends, 2000);
    assert_eq!(a.accounts[0].stats.combo_dividends, 200);
    assert_eq!(b.accounts[0].stats.combo_dividends, 200);
    assert_eq!(a.accounts[0].terminal_value, 2200);
    assert_eq!(b.accounts[0].terminal_value, 2200);
    assert_eq!(a.accounts[0].equity, b.accounts[0].equity);
}

#[test]
fn per_tick_news_disclosure_does_not_change_rate_before_effective_tick() {
    let mut cfg = config(90, "[fees]\nidle_rate=0", &group("passive", 100000, 10));
    cfg.world.dividend_period = 1;
    cfg.world.dividend_scale = 50;
    cfg.stocks[0].dividend = 3000;
    cfg.bots[0].shares_each = 10;
    cfg.world.news.stock.events_per_1000_ticks = 200.0;
    cfg.world.news.stock.change_sd = 0.2;
    cfg.world.news.disclosure.stages = vec![(0, 0.5), (3, 1.0)];
    cfg.world.news.disclosure.range_widths = vec![1.5];
    let name = cfg.bots[0].bot.clone();
    let h = Harness::new(vec![(&name, idle())]);
    let shared = Shared::new(&cfg, 7, &|n, s| h.factory(n, s)).unwrap();
    shared.run().unwrap();
    let log = h.log(&name);
    assert!(log.iter().any(|o| o
        .news
        .iter()
        .any(|n| n.public && n.public_tick < n.effective_tick && o.tick < n.effective_tick)));
    assert!(shared.world.events.iter().any(|e| e.effective < cfg.ticks));
    for o in &log {
        for s in 0..cfg.stocks.len() {
            let rate = cfg.stocks[s].dividend
                + shared
                    .world
                    .events
                    .iter()
                    .filter(|e| e.effective <= o.tick)
                    .flat_map(|e| &e.changes)
                    .filter(|(stock, _)| *stock == s)
                    .map(|(_, delta)| delta)
                    .sum::<i64>();
            assert_eq!(o.books[s].dividend, rate);
            let paid: Money = o
                .income
                .iter()
                .filter_map(|i| match i {
                    Income::Dividend { stock, amount } if *stock == s => Some(*amount),
                    _ => None,
                })
                .sum();
            let expected = if o.tick == 0 {
                0
            } else {
                (10 * rate + 25) / 50
            };
            assert_eq!(paid, expected, "stock {s}, tick {}", o.tick);
        }
    }
}

/// Two stocks, no news, no dividends, no idle fee unless overridden.
fn config(ticks: Tick, extra: &str, bots: &str) -> Config {
    let text = format!(
        r#"
ticks = {ticks}
seed = 1
{extra}
[schedule]
threads = 1
check_interval = 10
[world]
sectors = ["s"]
[world.news.market]
events_per_1000_ticks = 0
change_sd = 0
[world.news.sector]
events_per_1000_ticks = 0
change_sd = 0
[world.news.stock]
events_per_1000_ticks = 0
change_sd = 0
[world.news.disclosure]
lead_ticks = 10
stages = [[0, 1.0]]
range_widths = []
[[stocks]]
name = "A"
sector = "s"
dividend = 0
initial_price = 1000
[[stocks]]
name = "B"
sector = "s"
dividend = 0
initial_price = 1000
{bots}
"#
    );
    let mut cfg: Config = toml::from_str(&text).unwrap();
    if !extra.contains("[fees]") {
        cfg.fees.idle_rate = 0.0;
    }
    cfg.world.seasons.calm_to_turbulent = 0.0;
    cfg.validate().unwrap();
    cfg
}

fn group(name: &str, cash: Money, shares: Qty) -> String {
    format!("[[bots]]\nbot = \"{name}\"\ncash = {cash}\nshares_each = {shares}\n")
}

fn run(cfg: &Config, h: &Harness) -> Vec<(Money, Vec<Qty>)> {
    let f = |n: &str, s: &Start| h.factory(n, s);
    let shared = Shared::new(cfg, cfg.seed, &f).unwrap();
    shared.run().unwrap();
    shared
        .accounts
        .iter()
        .map(|a| {
            let a = a.lock();
            (a.cash, a.shares.clone())
        })
        .collect()
}

fn fee(rules_bps: i64, notional: Money) -> Money {
    (notional * rules_bps + 9_999) / 10_000
}

#[test]
fn idle_fee_cancels_unfunded_buys_newest_first_without_borrowing() {
    let mut cfg = config(
        4,
        "[fees]\nidle_period=2\nidle_rate=0.1\ntrade_fee_bps=0\ncredit_ratio=0.0",
        &(group("buyer", 1000, 0) + &group("seller", 0, 10)),
    );
    cfg.margin.max_leverage = 1.0;
    let h = Harness::new(vec![
        (
            "buyer",
            Box::new(|o| match o.tick {
                0 => vec![Action::limit(0, Side::Buy, 400, 1)],
                1 => vec![Action::limit(0, Side::Buy, 600, 1)],
                _ => vec![],
            }),
        ),
        ("seller", at(2, vec![Action::market(0, Side::Sell, 2)])),
    ]);
    let result = run(&cfg, &h);
    let log = h.log("buyer");
    assert_eq!(log[2].account.cash, 900);
    assert_eq!(log[2].open_orders.len(), 1);
    assert_eq!(
        log[2].open_orders[0].price, 400,
        "oldest quote retains priority"
    );
    assert_eq!(result[0].0, 500);
    assert_eq!(result[0].1[0], 1);
    assert!(log.iter().all(|o| o.account.cash >= 0));
}

#[test]
fn idle_fee_cancels_fully_reserved_cash_buy() {
    let mut cfg = config(
        2,
        "[fees]\nidle_period=1\nidle_rate=0.1\ntrade_fee_bps=0\ncredit_ratio=0.0",
        &(group("buyer", 1000, 0) + &group("seller", 0, 10)),
    );
    cfg.margin.max_leverage = 1.0;
    let h = Harness::new(vec![
        ("buyer", at(0, vec![Action::limit(0, Side::Buy, 1000, 1)])),
        ("seller", at(1, vec![Action::market(0, Side::Sell, 1)])),
    ]);
    let result = run(&cfg, &h);
    assert_eq!(result[0], (900, vec![0, 0]));
}

#[test]
fn liquidation_rechecks_slippage_and_stops_when_no_liquidity_remains() {
    for bid in [0, 100, 900] {
        let mut cfg = config(
            3,
            "[fees]\nidle_rate=0.0\ntrade_fee_bps=0",
            &(group("lender", 100000, 0)
                + &group("buyer", 1000, 0)
                + &group("seller", 0, 10)
                + &group("printer", 100000, 0)),
        );
        cfg.margin.mark_window = 1;
        let h = Harness::new(vec![
            (
                "lender",
                at(
                    0,
                    if bid > 0 {
                        vec![Action::limit(0, Side::Buy, bid, 100)]
                    } else {
                        vec![]
                    },
                ),
            ),
            ("buyer", at(0, vec![Action::limit(0, Side::Buy, 1000, 4)])),
            (
                "seller",
                Box::new(|o| match o.tick {
                    0 => vec![Action::market(0, Side::Sell, 4)],
                    1 => vec![Action::limit(0, Side::Sell, 900, 1)],
                    _ => vec![],
                }),
            ),
            ("printer", at(1, vec![Action::market(0, Side::Buy, 1)])),
        ]);
        let result = market::report::run_match(&cfg, 1, &|n, s| h.factory(n, s), None).unwrap();
        let buyer = &result.accounts[1];
        assert_eq!(buyer.stats.liquidations, 1);
        if bid == 100 {
            assert_eq!(buyer.cash, 0);
            assert_eq!(buyer.shares, [0, 0]);
            assert_eq!(buyer.stats.bad_debt, 2600);
        } else if bid == 0 {
            assert_eq!(buyer.cash, -3000);
            assert_eq!(buyer.shares, [4, 0]);
            assert_eq!(buyer.stats.bad_debt, 0);
        } else {
            assert_eq!(buyer.cash, -1200);
            assert_eq!(buyer.shares, [2, 0]);
            assert_eq!(buyer.stats.bad_debt, 0);
            assert_eq!(result.final_marks[0], 900);
            assert!(1800.0 / 600.0 <= cfg.margin.max_leverage);
        }
    }
}

#[test]
fn marks_expire_before_observation_and_settlement_without_trades() {
    let mut cfg = config(
        30,
        "[fees]\nidle_rate=0.0\ntrade_fee_bps=0",
        &(group("seller", 0, 10) + &group("buyer", 100000, 0)),
    );
    cfg.margin.mark_window = 2;
    let h = Harness::new(vec![
        (
            "seller",
            Box::new(|o| match o.tick {
                0 => vec![Action::limit(0, Side::Sell, 1000, 1)],
                1 => vec![Action::limit(0, Side::Sell, 2000, 1)],
                _ => vec![],
            }),
        ),
        (
            "buyer",
            Box::new(|o| {
                if o.tick <= 1 {
                    vec![Action::market(0, Side::Buy, 1)]
                } else {
                    vec![]
                }
            }),
        ),
    ]);
    let result = market::report::run_match(&cfg, 1, &|n, s| h.factory(n, s), None).unwrap();
    let log = h.log("buyer");
    for o in &log[2..] {
        assert_eq!(o.books[0].mark, 2000, "tick {}", o.tick);
        assert_eq!(o.account.gross, 4000);
    }
    assert_eq!(result.final_marks[0], 2000);
}

#[test]
fn terminal_news_and_combos_use_the_final_executed_tick() {
    let mut cfg = config(10,
        "[[combos]]\na=\"A\"\nb=\"B\"\nbonus=10\nend_tick=9\n[[combos]]\na=\"A\"\nb=\"B\"\nbonus=9999\nstart_tick=10",
        &group("x", 1000, 1));
    cfg.stocks[0].dividend = 100;
    cfg.world.news.market.events_per_1000_ticks = 1000.0;
    cfg.world.news.market.change_sd = 0.2;
    cfg.scoring.terminal_payments = Some((1, 1));
    let h = Harness::new(vec![("x", idle())]);
    let result = market::report::run_match(&cfg, 1, &|n, s| h.factory(n, s), None).unwrap();
    let log = h.log("x");
    assert_eq!(log.last().unwrap().tick, 9);
    assert_eq!(result.final_dividends, [100, 0]);
    assert_eq!(result.accounts[0].terminal_value, 110);
}

#[test]
fn repeated_bot_groups_have_unique_stable_owners() {
    let mut cfg = config(2, "", &(group("x", 1000, 0) + &group("x", 2000, 0)));
    cfg.bots[0].count_max = Some(3);
    for seed in 1..10 {
        let factory = |_: &str, _: &Start| -> Option<Box<dyn Bot>> {
            Some(Box::new(Scripted {
                script: idle(),
                log: Arc::new(Mutex::new(vec![])),
            }))
        };
        let result = market::report::run_match(&cfg, seed, &factory, None).unwrap();
        assert_eq!(result.accounts.last().unwrap().account, "x#3");
        assert_eq!(result.by_owner.len(), result.accounts.len());
    }
    for reverse in [false, true] {
        let mut explicit = cfg.bots[1].clone();
        explicit.owner = Some("x#1".into());
        if reverse {
            cfg.bots.insert(0, explicit);
        } else {
            cfg.bots.push(explicit);
        }
        assert!(
            cfg.validate().is_err(),
            "collisions must be rejected before seed-specific counts are drawn"
        );
        if reverse {
            cfg.bots.remove(0);
        } else {
            cfg.bots.pop();
        }
    }
}

#[test]
fn invalid_numeric_configuration_is_rejected_before_bot_initialization() {
    let base = config(10, "", &group("x", 1000, 0));
    let cases: &[fn(&mut Config)] = &[
        |c| c.market.tick_size = 0,
        |c| c.market.tick_size = -1,
        |c| c.market.opening_spread = Some(f64::INFINITY),
        |c| c.fees.trade_fee_bps = -1,
        |c| c.fees.idle_rate = f64::NAN,
        |c| c.margin.max_leverage = f64::NAN,
        |c| c.margin.mark_window = 0,
        |c| c.world.news.market.change_sd = -1.0,
        |c| c.world.news.disclosure.stages[0].1 = f64::NAN,
        |c| c.scoring.terminal_ratio = f64::INFINITY,
        |c| c.scoring.terminal_hints = vec![(5, 2), (0, 1)],
        |c| c.bots[0].count = 0,
        |c| c.stocks[1].name = c.stocks[0].name.clone(),
    ];
    for mutate in cases {
        let mut cfg = base.clone();
        mutate(&mut cfg);
        assert!(cfg.validate().is_err(), "accepted {cfg:?}");
        assert!(Shared::new(&cfg, 1, &|_, _| panic!(
            "invalid config reached bot factory"
        ))
        .is_err());
    }
    // Zero-count groups are useful for ablation, provided another group runs.
    let mut cfg = base.clone();
    cfg.bots.push(cfg.bots[0].clone());
    cfg.bots[0].count = 0;
    cfg.validate().unwrap();
}

#[test]
fn checked_in_scenarios_pass_validation() {
    let root = std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../scenarios"));
    let mut dirs = vec![root];
    while let Some(dir) = dirs.pop() {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                dirs.push(path);
            } else if path.extension().is_some_and(|ext| ext == "toml") {
                Config::load(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            }
        }
    }
}

#[test]
fn fees_do_not_overflow_at_the_valid_order_and_fee_limits() {
    let mut cfg = config(2, "", &group("x", 1000, 0));
    cfg.fees.trade_fee_bps = 10_000;
    cfg.validate().unwrap();
    let rules = cfg.rules(&[1, 1]);
    let notional = market::config::MAX_PRICE * market::config::MAX_SHARES;
    assert_eq!(rules.trade_fee(notional), notional);
}

#[test]
fn price_time_priority_and_partial_fill() {
    let bots = group("s1", 0, 100) + &group("s2", 0, 100) + &group("buyer", 1_000_000, 0);
    let cfg = config(10, "", &bots);
    let h = Harness::new(vec![
        ("s1", at(1, vec![Action::limit(0, Side::Sell, 1010, 10)])),
        ("s2", at(1, vec![Action::limit(0, Side::Sell, 1010, 10)])),
        ("buyer", at(3, vec![Action::limit(0, Side::Buy, 1010, 15)])),
    ]);
    let r = run(&cfg, &h);
    assert_eq!(r[0].1[0], 90, "earlier order fills first, in full");
    assert_eq!(r[1].1[0], 95, "later order fills the rest");
    assert_eq!(r[2].1[0], 15);
    assert_eq!(r[0].0, 10 * 1010 - fee(5, 10 * 1010));
    // the taker pays a fee per maker fill, each rounded up
    assert_eq!(
        r[2].0,
        1_000_000 - 15 * 1010 - fee(5, 10 * 1010) - fee(5, 5 * 1010)
    );
}

#[test]
fn better_price_fills_first_at_maker_price() {
    let bots = group("s1", 0, 100) + &group("s2", 0, 100) + &group("buyer", 1_000_000, 0);
    let cfg = config(10, "", &bots);
    let h = Harness::new(vec![
        ("s1", at(1, vec![Action::limit(0, Side::Sell, 1020, 10)])),
        ("s2", at(1, vec![Action::limit(0, Side::Sell, 1005, 10)])),
        ("buyer", at(3, vec![Action::ioc(0, Side::Buy, 1030, 12)])),
    ]);
    let r = run(&cfg, &h);
    assert_eq!(r[1].1[0], 90);
    assert_eq!(r[0].1[0], 98);
    let cost = 10 * 1005 + 2 * 1020;
    assert_eq!(
        r[2].0,
        1_000_000 - cost - fee(5, 10 * 1005) - fee(5, 2 * 1020)
    );
}

#[test]
fn self_cross_rules() {
    let bots = group("me", 1_000_000, 100) + &group("other", 1_000_000, 100);
    let cfg = config(10, "", &bots);
    let h = Harness::new(vec![
        (
            "me",
            Box::new(|o: &Observation| match o.tick {
                1 => vec![Action::limit(0, Side::Buy, 1000, 5)],
                2 => vec![
                    Action::limit(0, Side::Sell, 1000, 5), // not above own bid: rejected
                    Action::limit(0, Side::Sell, 1001, 5), // ok
                    Action::limit(1, Side::Sell, 900, 5),  // other stock: ok
                    Action::market(0, Side::Buy, 1),       // live sell on stock 0: rejected
                    Action::limit(0, Side::Buy, 1001, 1),  // not below own ask: rejected
                ],
                _ => vec![],
            }) as Script,
        ),
        ("other", idle()),
    ]);
    run(&cfg, &h);
    let log = h.log("me");
    let rejects: Vec<RejectReason> = log
        .iter()
        .flat_map(|o| o.rejects.iter().map(|r| r.reason))
        .collect();
    assert_eq!(rejects, vec![RejectReason::SelfCross; 3]);
    let last = log.last().unwrap();
    assert_eq!(last.open_orders.len(), 3);
}

#[test]
fn cancel_releases_reservations() {
    let bots = group("me", 1_000_000, 100);
    let cfg = config(10, "", &bots);
    let h = Harness::new(vec![(
        "me",
        Box::new(|o: &Observation| match o.tick {
            1 => vec![
                Action::limit(0, Side::Buy, 900, 100),
                Action::limit(1, Side::Sell, 1100, 40),
            ],
            3 => o
                .open_orders
                .iter()
                .map(|x| Action::Cancel {
                    order_id: x.order_id,
                })
                .collect(),
            _ => vec![],
        }) as Script,
    )]);
    run(&cfg, &h);
    let log = h.log("me");
    let mid = &log[2]; // tick 2
    assert_eq!(mid.account.reserved_buy, 100 * (900 + 1));
    assert_eq!(mid.account.sellable[1], 60);
    let end = log.last().unwrap();
    assert_eq!(end.account.reserved_buy, 0);
    assert_eq!(end.account.sellable[1], 100);
    assert_eq!(end.account.cash, 1_000_000);
    assert!(end.open_orders.is_empty());
}

#[test]
fn market_buy_capped_by_buying_power() {
    // equity 100_000 cash; max leverage 4 → at most ~400_000 of stock.
    let bots = group("seller", 0, 10_000) + &group("buyer", 100_000, 0);
    let cfg = config(10, "", &bots);
    let h = Harness::new(vec![
        (
            "seller",
            at(1, vec![Action::limit(0, Side::Sell, 1000, 10_000)]),
        ),
        ("buyer", at(3, vec![Action::market(0, Side::Buy, 10_000)])),
    ]);
    let r = run(&cfg, &h);
    let bought = r[1].1[0];
    assert!(bought > 0 && bought * 1000 <= 400_000, "bought {bought}");
    assert!(
        bought >= 390,
        "should use most of the power, bought {bought}"
    );
    assert!(r[1].0 < 0, "the purchase is financed by a loan");
}

#[test]
fn idle_fee_burns_proportionally_and_trading_earns_credit() {
    let extra =
        "[fees]\nidle_period = 10\nidle_rate = 0.01\ncredit_ratio = 0.01\ntrade_fee_bps = 0\n";
    let bots =
        group("lazy", 100_000, 100) + &group("t1", 100_000, 100) + &group("t2", 100_000, 100);
    let cfg = config(11, extra, &bots);
    let h = Harness::new(vec![
        ("lazy", idle()),
        ("t1", at(2, vec![Action::limit(0, Side::Sell, 1000, 50)])),
        ("t2", at(4, vec![Action::limit(0, Side::Buy, 1000, 50)])),
    ]);
    let r = run(&cfg, &h);
    // lazy: equity 300_000, fee 3_000 = 1% of cash and of each holding
    assert_eq!(r[0].0, 99_000);
    assert_eq!(r[0].1, vec![99, 99]);
    // traders: 50_000 notional * 1% = 500 credit, fee 2_500 left; still pay something
    let t2_value = r[2].0 + (r[2].1[0] + r[2].1[1]) * 1000;
    assert!(
        t2_value > 297_000 && t2_value < 300_000,
        "t2 value {t2_value}"
    );
}

#[test]
fn combo_dividends_positive_and_negative() {
    let extra = "[[combos]]\na = \"A\"\nb = \"B\"\nbonus = 3\n[[combos]]\na = \"A\"\na_units = 2\nb = \"B\"\nbonus = -1\n";
    let bots = group("x", 0, 10) + &group("y", 0, 0);
    let mut cfg = config(51, extra, &bots);
    cfg.bots[1].shares.insert("A".into(), 6);
    cfg.bots[1].shares.insert("B".into(), 2);
    let h = Harness::new(vec![("x", idle()), ("y", idle())]);
    let r = run(&cfg, &h);
    // one payment at tick 50; x: 10 sets*3 - 5 sets*1 = 25; y: 2*3 - 2*1 = 4
    assert_eq!(r[0].0, 25);
    assert_eq!(r[1].0, 4);
}

#[test]
fn rank_ties_are_averaged() {
    let p = rank_percentiles(&[10, 30, 30, 5]);
    assert_eq!(p[1].1, p[2].1);
    assert!((p[1].1 - (100.0 + 100.0 * 2.0 / 3.0) / 2.0).abs() < 1e-9);
    assert_eq!(p[3].1, 0.0);
    assert_eq!(rank_percentiles(&[7])[0].1, 100.0);
}

#[test]
fn baseline_conserves_money_and_shares_multithreaded() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../scenarios/baseline.toml");
    let mut cfg = Config::load(std::path::Path::new(path)).unwrap();
    cfg.ticks = 600;
    cfg.schedule.threads = 4;
    cfg.schedule.check_interval = 50;
    // noisy random traders stand in for the real bots
    let factory = |_: &str, s: &Start| -> Option<Box<dyn Bot>> {
        let mut x = s.seed | 1;
        Some(Box::new(Scripted {
            log: Arc::new(Mutex::new(Vec::new())),
            script: Box::new(move |o: &Observation| {
                let mut acts = vec![];
                for (st, b) in o.books.iter().enumerate() {
                    x ^= x << 13;
                    x ^= x >> 7;
                    x ^= x << 17;
                    if !x.is_multiple_of(3) {
                        continue;
                    }
                    let px = (b.last + (x % 41) as i64 - 20).max(1);
                    let side = if x.is_multiple_of(2) {
                        Side::Buy
                    } else {
                        Side::Sell
                    };
                    acts.push(Action::limit(st, side, px, 1 + (x % 30) as i64));
                }
                if o.tick.is_multiple_of(37) {
                    acts.push(Action::CancelAll { stock: None });
                }
                acts
            }),
        }))
    };
    let shared = Shared::new(&cfg, 3, &factory).unwrap();
    shared.run().unwrap();
    assert!(shared.audits.load(std::sync::atomic::Ordering::Relaxed) >= 10);
}

#[test]
fn news_arrives_before_it_takes_effect() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../scenarios/baseline.toml");
    let mut cfg = Config::load(std::path::Path::new(path)).unwrap();
    cfg.ticks = 1500;
    cfg.schedule.threads = 1; // no skipped decisions, so each delivery is observed on its tick
    cfg.bots.truncate(1);
    cfg.bots[0].count = 6;
    let logs: Mutex<Vec<Log>> = Mutex::new(vec![]);
    let factory = |_: &str, _s: &Start| -> Option<Box<dyn Bot>> {
        let log: Log = Arc::new(Mutex::new(Vec::new()));
        logs.lock().push(log.clone());
        Some(Box::new(Scripted {
            log,
            script: Box::new(|_| vec![]),
        }))
    };
    let shared = Shared::new(&cfg, 11, &factory).unwrap();
    shared.run().unwrap();
    let truth: HashMap<(u32, usize), Money> = shared
        .world
        .events
        .iter()
        .flat_map(|e| e.changes.iter().map(move |&(s, d)| ((e.id, s), d)))
        .collect();
    let mut total = 0;
    let mut ranged = 0;
    for log in logs.lock().iter() {
        let mut first_seen: HashMap<u32, Tick> = HashMap::new();
        let mut public_seen: HashMap<u32, u32> = HashMap::new();
        let mut last_width: HashMap<(u32, usize), Money> = HashMap::new();
        for o in log.lock().iter() {
            for n in &o.news {
                assert!(
                    o.tick < n.effective_tick || n.effective_tick >= cfg.ticks,
                    "news {} seen at {}",
                    n.id,
                    o.tick
                );
                first_seen.entry(n.id).or_insert(o.tick);
                for c in &n.changes {
                    let d = truth[&(n.id, c.stock)];
                    assert!(
                        c.low <= d && d <= c.high,
                        "range must contain the true change"
                    );
                    let w = c.high - c.low;
                    if n.public {
                        assert_eq!(w, 0, "public news is exact");
                    } else if w > 0 {
                        ranged += 1;
                    }
                    let prev = last_width.insert((n.id, c.stock), w);
                    assert!(prev.is_none_or(|p| w <= p), "ranges narrow at later stages");
                }
                if n.public {
                    assert!(o.tick >= n.public_tick);
                    *public_seen.entry(n.id).or_default() += 1;
                } else {
                    assert!(o.tick < n.public_tick);
                }
            }
        }
        assert!(
            public_seen.values().all(|&c| c == 1),
            "public delivery exactly once"
        );
        total += first_seen.len();
    }
    assert!(ranged > 0, "private deliveries carry ranges");
    assert!(total > 0, "some news was generated");
}

#[test]
fn terminal_hints_contain_the_hidden_payment_count() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../scenarios/baseline.toml");
    let mut cfg = Config::load(std::path::Path::new(path)).unwrap();
    cfg.ticks = 2100;
    cfg.schedule.threads = 1;
    cfg.bots.truncate(1);
    cfg.bots[0].count = 5;
    let logs: Mutex<Vec<Log>> = Mutex::new(vec![]);
    let factory = |_: &str, _s: &Start| -> Option<Box<dyn Bot>> {
        let log: Log = Arc::new(Mutex::new(Vec::new()));
        logs.lock().push(log.clone());
        Some(Box::new(Scripted {
            log,
            script: Box::new(|_| vec![]),
        }))
    };
    let shared = Shared::new(&cfg, 5, &factory).unwrap();
    shared.run().unwrap();
    let n = shared.world.terminal_payments;
    let (lo, hi) = cfg.scoring.terminal_payments.unwrap();
    assert!(lo <= n && n <= hi);
    for log in logs.lock().iter() {
        let hints: Vec<(Tick, (i64, i64))> = log
            .lock()
            .iter()
            .flat_map(|o| o.terminal_hints.iter().map(move |h| (o.tick, *h)))
            .collect();
        let ticks: Vec<Tick> = hints.iter().map(|h| h.0).collect();
        assert_eq!(ticks, vec![0, 1000, 2000]);
        for (k, (_, (a, b))) in hints.iter().enumerate() {
            assert!(*a <= n && n <= *b, "hint must contain the truth");
            assert_eq!(b - a, cfg.scoring.terminal_hints[k].1);
        }
    }
}
