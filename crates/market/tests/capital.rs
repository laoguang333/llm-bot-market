use bot_api::{Action, Bot, Observation, Start};
use market::config::{Config, MAX_CASH, MAX_SHARES};
use market::engine::Shared;
use std::path::Path;
use std::sync::Mutex;

struct Idle;
impl Bot for Idle {
    fn on_tick(&mut self, _: &Observation) -> Vec<Action> {
        vec![]
    }
}

fn cfg() -> Config {
    Config::load(Path::new("../../scenarios/capital-v18-balanced.toml")).unwrap()
}

fn starts(cfg: &Config, seed: u64) -> Vec<Start> {
    let log = Mutex::new(Vec::new());
    let shared = Shared::new(cfg, seed, &|_, s| {
        log.lock().unwrap().push(s.clone());
        Some(Box::new(Idle))
    })
    .unwrap();
    shared.audit().unwrap();
    for a in &shared.accounts {
        let a = a.lock();
        assert_eq!(a.cash, a.initial_cash);
        assert_eq!(a.shares, a.initial_shares);
    }
    log.into_inner().unwrap()
}

#[test]
fn varying_populations_preserve_every_asset_identity_and_information_permission() {
    let new = cfg();
    let mut old = new.clone();
    old.capital_redistribution = None;
    for seed in [1, 7, 43, 83001, 83007, 84001] {
        let before = starts(&old, seed);
        let after = starts(&new, seed);
        assert_eq!(before.len(), after.len());
        assert_eq!(
            before.iter().map(|s| s.cash).sum::<i64>(),
            after.iter().map(|s| s.cash).sum::<i64>()
        );
        for stock in 0..new.stocks.len() {
            assert_eq!(
                before.iter().map(|s| s.shares[stock]).sum::<i64>(),
                after.iter().map(|s| s.shares[stock]).sum::<i64>()
            );
        }
        for (a, b) in before.iter().zip(&after) {
            assert_eq!(a.account, b.account);
            assert_eq!(a.account_index, b.account_index);
            assert_eq!(a.seed, b.seed);
            assert_eq!(a.params, b.params);
            assert_eq!(a.news_multiplier, b.news_multiplier);
            assert_eq!(a.forecast_access.horizon, b.forecast_access.horizon);
            assert_eq!(
                a.rules.stocks[0].initial_price,
                b.rules.stocks[0].initial_price
            );
            if b.account.starts_with("grid#") {
                assert_eq!(b.cash, 2_000_000);
                assert!(b.shares.iter().all(|q| *q == 20));
            }
        }
    }
}

#[test]
fn named_challengers_and_incumbents_never_join_the_pool() {
    let mut c = cfg();
    let mut challenger = c.bots.iter().find(|g| g.bot == "coral").unwrap().clone();
    challenger.owner = Some("challenger".into());
    challenger.count = 1;
    challenger.count_max = None;
    c.bots.push(challenger);
    let mut grid_challenger = c.bots[0].clone();
    grid_challenger.owner = Some("grid-challenger".into());
    grid_challenger.count = 1;
    grid_challenger.count_max = None;
    c.bots.push(grid_challenger);
    let log = starts(&c, 7);
    for (owner, cash, shares) in [
        ("challenger", 2_000_000, 20),
        ("grid-challenger", 4_000_000, 120),
        ("almanac-incumbent", 2_000_000, 20),
        ("osprey-incumbent", 2_000_000, 20),
    ] {
        let s = log.iter().find(|s| s.owner == owner).unwrap();
        assert_eq!(s.cash, cash);
        assert!(s.shares.iter().all(|q| *q == shares));
    }
}

#[test]
fn indivisible_pools_and_stock_overrides_are_conserved() {
    let mut c = cfg();
    c.bots[0].count = 1;
    c.bots[0].count_max = None;
    let stock = c.stocks[0].name.clone();
    c.bots[0].shares.insert(stock, 130);
    let plan = c.capital_redistribution.as_mut().unwrap();
    plan.cash_each = 7;
    plan.shares_each = 3;
    plan.recipients = [("coral".into(), 1), ("value".into(), 2)].into();
    let after = starts(&c, 7);
    c.capital_redistribution = None;
    let before = starts(&c, 7);
    assert_eq!(after[0].shares[0], 127);
    assert_eq!(after[0].shares[1], 117);
    assert_eq!(after[0].cash, 3_999_993);
    assert_eq!(
        after.iter().map(|s| s.cash).sum::<i64>(),
        before.iter().map(|s| s.cash).sum::<i64>()
    );
    for stock in 0..c.stocks.len() {
        assert_eq!(
            after.iter().map(|s| s.shares[stock]).sum::<i64>(),
            before.iter().map(|s| s.shares[stock]).sum::<i64>()
        );
    }
    assert_eq!(
        after
            .iter()
            .zip(&before)
            .filter(|(a, _)| a.owner.starts_with("coral#"))
            .map(|(a, b)| a.cash - b.cash)
            .sum::<i64>(),
        2
    );
}

#[test]
fn invalid_plans_fail_instead_of_silently_destroying_or_creating_assets() {
    for case in 0..8 {
        let mut c = cfg();
        let plan = c.capital_redistribution.as_mut().unwrap();
        match case {
            0 => plan.cash_each = 4_000_001,
            1 => plan.shares_each = 121,
            2 => {
                plan.recipients.insert("missing".into(), 1);
            }
            3 => {
                plan.recipients.insert("grid".into(), 1);
            }
            4 => {
                plan.recipients.insert("coral".into(), 0);
            }
            5 => plan.from_bot = "missing".into(),
            6 => {
                c.bots[0].shares.insert(c.stocks[0].name.clone(), 99);
            }
            _ => {
                plan.recipients.insert("coral".into(), 1_000_001);
            }
        }
        assert!(c.validate().is_err(), "case {case}");
    }
}

#[test]
fn excessive_recipient_balances_are_rejected_before_start() {
    for shares in [false, true] {
        let mut c = cfg();
        let recipient = c.bots.iter_mut().find(|g| g.bot == "coral").unwrap();
        if shares {
            recipient.shares_each = MAX_SHARES;
        } else {
            recipient.cash = MAX_CASH;
        }
        assert!(Shared::new(&c, 7, &|_, _| panic!(
            "must reject before constructing bots"
        ))
        .is_err());
    }
}
