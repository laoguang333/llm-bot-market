//! Research harness. The strategy and market engine remain unchanged.
//! Records observed equity and can reduce only the challenger's decision cadence.
use anyhow::{bail, Context, Result};
use bot_api::{Action, Bot, FinalResult, Money, Observation, Tick, Trade};
use market::{config::Config, report::run_match};
use std::{
    collections::BTreeMap,
    fmt::Write as _,
    path::PathBuf,
    sync::{Arc, Mutex},
};

#[derive(Default)]
struct Trace {
    points: Vec<(Tick, Money, Money, Money)>,
    calls: usize,
}

struct Probe {
    inner: Box<dyn Bot>,
    every: Tick,
    pending: Vec<Vec<Trade>>,
    tape_limit: usize,
    trace: Trace,
    output: Arc<Mutex<Trace>>,
}

impl Bot for Probe {
    fn on_tick(&mut self, obs: &Observation) -> Vec<Action> {
        self.trace.points.push((
            obs.tick,
            obs.account.equity,
            obs.account.cash,
            obs.account.gross,
        ));
        if self.every == 1 {
            self.trace.calls += 1;
            return self.inner.on_tick(obs);
        }
        if self.pending.is_empty() {
            self.pending = vec![vec![]; obs.trades.len()];
        }
        for (pending, fresh) in self.pending.iter_mut().zip(&obs.trades) {
            pending.extend_from_slice(fresh);
            if pending.len() > self.tape_limit {
                pending.drain(..pending.len() - self.tape_limit);
            }
        }
        if !obs.tick.is_multiple_of(self.every) {
            return vec![];
        }
        let mut combined = obs.clone();
        combined.trades = self.pending.iter_mut().map(std::mem::take).collect();
        self.trace.calls += 1;
        self.inner.on_tick(&combined)
    }
    fn on_end(&mut self, result: &FinalResult) {
        self.inner.on_end(result);
        *self.output.lock().unwrap() = std::mem::take(&mut self.trace);
    }
}

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() < 3 || !(args.len() - 1).is_multiple_of(2) {
        bail!("swift-eval SCENARIO --out DIR [--first-seed N --seeds N --cash C --shares Q --params k=v --every N --cash-reference DIR]");
    }
    let flags: BTreeMap<_, _> = args[1..]
        .as_chunks::<2>()
        .0
        .iter()
        .map(|a| (a[0].as_str(), a[1].as_str()))
        .collect();
    for key in flags.keys() {
        if ![
            "--out",
            "--first-seed",
            "--seeds",
            "--cash",
            "--shares",
            "--params",
            "--every",
            "--cash-reference",
        ]
        .contains(key)
        {
            bail!("unknown option {key}");
        }
    }
    let base = Config::load(&PathBuf::from(&args[0]))?;
    let out = PathBuf::from(flags.get("--out").context("--out required")?);
    if out.exists() {
        bail!("output already exists: {}", out.display());
    }
    let first: u64 = flags.get("--first-seed").unwrap_or(&"7001").parse()?;
    let count: u64 = flags.get("--seeds").unwrap_or(&"20").parse()?;
    let every: Tick = flags.get("--every").unwrap_or(&"1").parse()?;
    if count == 0 || every == 0 {
        bail!("seeds and every must be positive");
    }
    std::fs::create_dir_all(&out)?;
    for seed in first..first + count {
        let mut cfg = base.clone();
        let mut challenger = cfg.bots[0].clone();
        challenger.bot = "swift".into();
        challenger.owner = Some("challenger".into());
        challenger.count = 1;
        challenger.count_max = None;
        challenger.cash = flags.get("--cash").unwrap_or(&"2000000").parse()?;
        challenger.shares_each = flags.get("--shares").unwrap_or(&"20").parse()?;
        challenger.shares.clear();
        challenger.params.clear();
        for pair in flags
            .get("--params")
            .unwrap_or(&"")
            .split(',')
            .filter(|s| !s.is_empty())
        {
            let (key, value) = pair.split_once('=').context("expected k=v")?;
            challenger.params.insert(key.into(), value.parse()?);
        }
        if let Some(reference) = flags.get("--cash-reference") {
            let prior: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(
                PathBuf::from(reference).join(format!("seed-{seed}/summary.json")),
            )?)?;
            let account = prior["accounts"]
                .as_array()
                .context("reference accounts")?
                .iter()
                .find(|a| a["owner"] == "challenger")
                .context("reference challenger")?;
            challenger.cash = account["initial_equity"]
                .as_i64()
                .context("reference initial equity")?;
            challenger.shares_each = 0;
        }
        cfg.bots.push(challenger);
        cfg.validate()?;
        let trace = Arc::new(Mutex::new(Trace::default()));
        let factory = |name: &str, start: &bot_api::Start| -> Option<Box<dyn Bot>> {
            let bot = bots::factory(name, start)?;
            if start.owner != "challenger" {
                return Some(bot);
            }
            Some(Box::new(Probe {
                inner: bot,
                every,
                pending: vec![],
                tape_limit: cfg.market.trade_tape_limit,
                trace: Trace::default(),
                output: Arc::clone(&trace),
            }))
        };
        let dir = out.join(format!("seed-{seed}"));
        let result = run_match(&cfg, seed, &factory, Some(&dir))?;
        let account = result
            .accounts
            .iter()
            .find(|a| a.owner == "challenger")
            .context("challenger missing")?;
        let trace = trace.lock().unwrap();
        let mut csv = String::from("tick,equity,cash,gross\n");
        for (t, eq, cash, gross) in &trace.points {
            writeln!(csv, "{t},{eq},{cash},{gross}")?;
        }
        std::fs::write(dir.join("challenger-equity.csv"), csv)?;
        std::fs::write(
            dir.join("probe.json"),
            serde_json::to_string_pretty(&serde_json::json!({
                "every": every, "strategy_calls": trace.calls, "observations": trace.points.len(),
                "equity_basis": "before each engine decision, at observed mark prices; final settlement is in summary.json"
            }))?,
        )?;
        println!(
            "seed={seed} percentile={:.3} return={:.3}% strategy_calls={}",
            account.percentile,
            (account.return_ratio - 1.0) * 100.0,
            trace.calls
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use bot_api::{AccountView, BookView, Season, Side};

    struct EchoVolume;
    impl Bot for EchoVolume {
        fn on_tick(&mut self, obs: &Observation) -> Vec<Action> {
            vec![Action::market(
                0,
                Side::Buy,
                obs.trades[0].iter().map(|t| t.qty).sum(),
            )]
        }
    }
    fn obs(tick: Tick) -> Observation {
        Observation {
            tick,
            ticks_total: 10,
            reference_tick: tick,
            season: Season {
                turbulent: false,
                rate_mult: 1.0,
                size_mult: 1.0,
            },
            account: AccountView {
                equity: 100 + tick as i64,
                ..AccountView::default()
            },
            open_orders: vec![],
            books: vec![BookView::default()],
            trades: vec![vec![Trade {
                tick,
                price: 10,
                qty: 1,
                aggressor: Side::Buy,
            }]],
            skipped: 0,
            fills: vec![],
            acks: vec![],
            rejects: vec![],
            news: vec![],
            season_changes: vec![],
            income: vec![],
            terminal_hints: vec![],
        }
    }
    fn probe(every: Tick, cap: usize) -> Probe {
        Probe {
            inner: Box::new(EchoVolume),
            every,
            tape_limit: cap,
            pending: vec![],
            trace: Trace::default(),
            output: Arc::new(Mutex::new(Trace::default())),
        }
    }
    #[test]
    fn ordinary_probe_passes_orders_and_observations_unchanged() {
        let mut probe = probe(1, 2);
        let mut plain = EchoVolume;
        for t in 0..10 {
            assert_eq!(probe.on_tick(&obs(t)), plain.on_tick(&obs(t)));
        }
        assert_eq!(probe.trace.calls, 10);
    }
    #[test]
    fn slower_cadence_preserves_intervening_tape_up_to_the_engine_cap() {
        let mut probe = probe(3, 2);
        assert_eq!(
            probe.on_tick(&obs(0)),
            vec![Action::market(0, Side::Buy, 1)]
        );
        assert!(probe.on_tick(&obs(1)).is_empty());
        assert!(probe.on_tick(&obs(2)).is_empty());
        assert_eq!(
            probe.on_tick(&obs(3)),
            vec![Action::market(0, Side::Buy, 2)]
        );
        probe.on_end(&FinalResult {
            equity: 110,
            rank: 1,
            percentile: 100.0,
        });
        let trace = probe.output.lock().unwrap();
        assert_eq!(trace.calls, 2);
        assert_eq!(trace.points.len(), 4);
        assert_eq!(trace.points.last().unwrap().1, 103);
    }
}
