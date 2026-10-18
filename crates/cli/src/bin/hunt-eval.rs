//! Two-account observation recorder. Neither bot receives the other's observations.
use anyhow::{bail, Context, Result};
use bot_api::{Action, Bot, FinalResult, Money, Observation, Tick};
use market::{config::Config, report::run_match};
use std::{
    collections::BTreeMap,
    fmt::Write as _,
    path::PathBuf,
    sync::{Arc, Mutex},
};

type Point = (Tick, Money, Money, Money);
type Traces = Arc<Mutex<BTreeMap<String, Vec<Point>>>>;
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
    if args.len() != 7 || args[1] != "--out" || args[3] != "--first-seed" || args[5] != "--seeds" {
        bail!("hunt-eval SCENARIO --out DIR --first-seed N --seeds N");
    }
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
        let result = run_match(&cfg, seed, &factory, Some(&dir))?;
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
