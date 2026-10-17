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
    let mut total = 0;
    for log in logs.lock().iter() {
        let mut first_seen: HashMap<u32, Tick> = HashMap::new();
        let mut public_seen: HashMap<u32, u32> = HashMap::new();
        for o in log.lock().iter() {
            for n in &o.news {
                assert!(
                    o.tick < n.effective_tick || n.effective_tick >= cfg.ticks,
                    "news {} seen at {}",
                    n.id,
                    o.tick
                );
                first_seen.entry(n.id).or_insert(o.tick);
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
    assert!(total > 0, "some news was generated");
}
