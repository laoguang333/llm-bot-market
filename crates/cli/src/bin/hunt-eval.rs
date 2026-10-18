//! Two-account observation recorder. Neither bot receives the other's observations.
use anyhow::{bail, Context, Result};
use bot_api::{Action, Bot, FinalResult, Money, Observation, Tick};
use market::{config::Config, engine::Shared, report::run_match};
use std::{
    collections::BTreeMap,
    fmt::Write as _,
    path::PathBuf,
    sync::{Arc, Mutex},
};

type Point = (Tick, Money, Money, Money);
type Traces = Arc<Mutex<BTreeMap<String, Vec<Point>>>>;

struct SetupOnly;
impl Bot for SetupOnly {
    fn on_tick(&mut self, _: &Observation) -> Vec<Action> {
        vec![]
    }
}

/// Change only endowments and an absolute risk allocation before trading.
/// The setup probe never runs. Only public opening prices leave it; no world
/// events, future prices or target observations reach either trading strategy.
fn capital_config(
    base: &Config,
    seed: u64,
    multiple: i64,
    reserve: bool,
) -> Result<(Config, serde_json::Value)> {
    if ![10, 30, 100, 300].contains(&multiple) {
        bail!("capital multiple must be 10, 30, 100 or 300");
    }
    let setup = Shared::new(base, seed, &|_, _| Some(Box::new(SetupOnly)))?;
    let prices = setup.world.opening_prices.clone();
    let background_equity: i64 = setup
        .accounts
        .iter()
        .map(|a| {
            let a = a.lock();
            if a.owner == "target" || a.owner == "predator" {
                0
            } else {
                a.initial_cash
                    + a.initial_shares
                        .iter()
                        .zip(&prices)
                        .map(|(q, p)| q * p)
                        .sum::<i64>()
            }
        })
        .sum();
    drop(setup);
    let mut cfg = base.clone();
    let target = cfg
        .bots
        .iter()
        .find(|b| b.owner.as_deref() == Some("target"))
        .context("target missing")?;
    if target.cash != 2_000_000 || target.shares_each != 20 || !target.shares.is_empty() {
        bail!("capital study requires standard Swift endowment");
    }
    let target_equity = target.cash + target.shares_each * prices.iter().sum::<i64>();
    let predators: Vec<_> = cfg
        .bots
        .iter_mut()
        .filter(|b| b.owner.as_deref() == Some("predator"))
        .collect();
    if predators.len() != 1 || predators[0].count != 1 || predators[0].count_max.is_some() {
        bail!("capital study requires exactly one predator");
    }
    let predator = predators.into_iter().next().unwrap();
    if predator.bot != "falcon" || !predator.shares.is_empty() {
        bail!("capital study requires Falcon without share overrides");
    }
    let deployed_multiple = if reserve { 10 } else { multiple };
    predator.shares_each = 20 * deployed_multiple;
    predator.cash = multiple * target_equity - predator.shares_each * prices.iter().sum::<i64>();
    predator.params.insert(
        "capital_limit".into(),
        (deployed_multiple * target_equity) as f64,
    );
    let metadata = serde_json::json!({
        "multiple": multiple, "reserve": reserve, "opening_prices": prices,
        "target_initial_equity": target_equity,
        "predator_initial_equity": multiple * target_equity,
        "predator_initial_cash": predator.cash,
        "predator_shares_each": predator.shares_each,
        "predator_params": predator.params,
        "background_initial_equity": background_equity,
    });
    cfg.validate()?;
    Ok((cfg, metadata))
}

struct Recorder {
    inner: Box<dyn Bot>,
    owner: String,
    points: Vec<Point>,
    output: Traces,
}
impl Bot for Recorder {
    fn on_tick(&mut self, obs: &Observation) -> Vec<Action> {
        self.points.push((
            obs.tick,
            obs.account.equity,
            obs.account.cash,
            obs.account.gross,
        ));
        self.inner.on_tick(obs)
    }
    fn on_end(&mut self, result: &FinalResult) {
        self.inner.on_end(result);
        self.output
            .lock()
            .unwrap()
            .insert(self.owner.clone(), std::mem::take(&mut self.points));
    }
}
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if ![7, 11].contains(&args.len())
        || args[1] != "--out"
        || args[3] != "--first-seed"
        || args[5] != "--seeds"
    {
        bail!("hunt-eval SCENARIO --out DIR --first-seed N --seeds N [--capital-multiple N --allocation reserve|scaled]");
    }
    let capital = if args.len() == 11 {
        if args[7] != "--capital-multiple"
            || args[9] != "--allocation"
            || !["reserve", "scaled"].contains(&args[10].as_str())
        {
            bail!("expected --capital-multiple N --allocation reserve|scaled");
        }
        Some((args[8].parse::<i64>()?, args[10] == "reserve"))
    } else {
        None
    };
    let cfg = Config::load(&PathBuf::from(&args[0]))?;
    let out = PathBuf::from(&args[2]);
    if out.exists() {
        bail!("output exists: {}", out.display());
    }
    let first: u64 = args[4].parse()?;
    let count: u64 = args[6].parse()?;
    if count == 0 {
        bail!("seeds must be positive");
    }
    // A second Swift, or a parameter-modified target, would invalidate this study.
    let targets: Vec<_> = cfg.bots.iter().filter(|b| b.bot == "swift").collect();
    if targets.len() != 1
        || targets[0].count != 1
        || targets[0].count_max.is_some()
        || !targets[0].params.is_empty()
        || targets[0].owner.as_deref() != Some("target")
    {
        bail!("requires exactly one unchanged Swift named target");
    }
    std::fs::create_dir_all(&out)?;
    for seed in first..first + count {
        let (run_cfg, capital_metadata) = if let Some((multiple, reserve)) = capital {
            let (cfg, metadata) = capital_config(&cfg, seed, multiple, reserve)?;
            (cfg, Some(metadata))
        } else {
            (cfg.clone(), None)
        };
        let traces: Traces = Arc::new(Mutex::new(BTreeMap::new()));
        let factory = |name: &str, start: &bot_api::Start| -> Option<Box<dyn Bot>> {
            let inner = bots::factory(name, start)?;
            if start.owner != "target" && start.owner != "predator" {
                return Some(inner);
            }
            Some(Box::new(Recorder {
                inner,
                owner: start.owner.clone(),
                points: vec![],
                output: Arc::clone(&traces),
            }))
        };
        let dir = out.join(format!("seed-{seed}"));
        let result = run_match(&run_cfg, seed, &factory, Some(&dir))?;
        if let Some(metadata) = capital_metadata {
            std::fs::write(
                dir.join("capital-setup.json"),
                serde_json::to_string_pretty(&metadata)?,
            )?;
        }
        let traces = traces.lock().unwrap();
        for (owner, points) in traces.iter() {
            if points.len() != cfg.ticks as usize {
                bail!("incomplete trace: {owner}");
            }
            let mut csv = String::from("tick,equity,cash,gross\n");
            for (t, e, c, g) in points {
                writeln!(csv, "{t},{e},{c},{g}")?;
            }
            std::fs::write(dir.join(format!("{owner}-equity.csv")), csv)?;
        }
        let target = result
            .accounts
            .iter()
            .find(|a| a.owner == "target")
            .context("target missing")?;
        let predator = result.accounts.iter().find(|a| a.owner == "predator");
        println!(
            "seed={seed} target={:.2}% predator={:?}",
            (target.return_ratio - 1.0) * 100.0,
            predator.map(|a| (a.return_ratio - 1.0) * 100.0)
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use market::config::BotGroupCfg;

    #[test]
    fn reserve_and_scaled_have_equal_wealth_but_different_allocations() {
        let mut cfg = Config::load(
            &PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../scenarios/baseline.toml"),
        )
        .unwrap();
        for (name, owner, cash, shares) in [
            ("swift", "target", 2_000_000, 20),
            ("falcon", "predator", 20_000_000, 200),
        ] {
            cfg.bots.push(BotGroupCfg {
                bot: name.into(),
                owner: Some(owner.into()),
                count: 1,
                count_max: None,
                cash,
                shares_each: shares,
                shares: BTreeMap::new(),
                params: BTreeMap::new(),
            });
        }
        for multiple in [10, 30, 100, 300] {
            let (a, x) = capital_config(&cfg, 7, multiple, true).unwrap();
            let (b, y) = capital_config(&cfg, 7, multiple, false).unwrap();
            assert_eq!(x["predator_initial_equity"], y["predator_initial_equity"]);
            assert_eq!(
                x["predator_initial_equity"].as_i64().unwrap(),
                multiple * x["target_initial_equity"].as_i64().unwrap()
            );
            assert_eq!(a.bots.last().unwrap().shares_each, 200);
            assert_eq!(b.bots.last().unwrap().shares_each, 20 * multiple);
            let target = a.bots.iter().find(|g| g.bot == "swift").unwrap();
            assert!(target.params.is_empty());
            assert_eq!(target.cash, 2_000_000);
            assert_eq!(target.shares_each, 20);
            assert_eq!(
                x["predator_params"]["capital_limit"].as_f64().unwrap(),
                10.0 * x["target_initial_equity"].as_i64().unwrap() as f64
            );
            if multiple == 10 {
                assert_eq!(x["predator_initial_cash"], y["predator_initial_cash"]);
            }
        }
    }
}
