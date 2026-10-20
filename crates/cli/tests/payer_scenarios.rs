use bot_api::{Action, Bot, Observation, Start};
use market::config::Config;
use market::engine::Shared;
use std::path::Path;
use std::sync::Mutex;

struct Idle;
impl Bot for Idle {
    fn on_tick(&mut self, _: &Observation) -> Vec<Action> {
        vec![]
    }
}

fn starts(name: &str, seed: u64) -> Vec<Start> {
    let path = format!("../../scenarios/{name}.toml");
    let cfg = Config::load(Path::new(&path)).unwrap();
    let log = Mutex::new(Vec::new());
    let market = Shared::new(&cfg, seed, &|_, s| {
        log.lock().unwrap().push(s.clone());
        Some(Box::new(Idle))
    })
    .unwrap();
    market.audit().unwrap();
    log.into_inner().unwrap()
}

#[test]
fn five_crowds_preserve_population_world_inputs_and_every_initial_asset() {
    for seed in [1, 7, 85001, 86002, 86010] {
        let before = starts("baseline-v18", seed);
        for arm in ["payer-v19-balanced", "payer-v19-social"] {
            let after = starts(arm, seed);
            assert_eq!(before.len(), after.len());
            assert_eq!(
                before.iter().map(|s| s.cash).sum::<i64>(),
                after.iter().map(|s| s.cash).sum::<i64>()
            );
            for stock in 0..before[0].shares.len() {
                assert_eq!(
                    before.iter().map(|s| s.shares[stock]).sum::<i64>(),
                    after.iter().map(|s| s.shares[stock]).sum::<i64>()
                );
            }
            for (old, new) in before.iter().zip(&after) {
                let name = old
                    .account
                    .replace("momentum#", "latecomer#")
                    .replace("hold#", "news_reactor#")
                    .replace("grid#", "grid_v2#");
                assert_eq!(new.account, name);
                assert_eq!(old.account_index, new.account_index);
                assert_eq!(old.seed, new.seed);
                assert_eq!(old.params, new.params);
                assert_eq!(old.news_multiplier, new.news_multiplier);
                assert_eq!(old.forecast_access, new.forecast_access);
                for (a, b) in old.rules.stocks.iter().zip(&new.rules.stocks) {
                    assert_eq!(a.initial_price, b.initial_price);
                    assert_eq!(a.shares_outstanding, b.shares_outstanding);
                }
                if !old.account.contains('#') || old.account.starts_with("grid#") {
                    assert_eq!(old.cash, new.cash);
                    assert_eq!(old.shares, new.shares);
                }
            }
        }
    }
}
