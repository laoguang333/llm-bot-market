//! Opt-in diagnostic recording. Normal matches never allocate trace files.
#[path = "../debug/swift.rs"]
mod swift;

use anyhow::{bail, Context, Result};
use bot_api::*;
use market::{account::Account, config::Config, engine::MatchObserver, report::run_match_observed};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    fs::File,
    io::{BufWriter, Write},
    path::PathBuf,
    sync::{Mutex, OnceLock},
};

struct AccountTrace {
    writer: BufWriter<File>,
    probe: Option<swift::Probe>,
    profit_probe: Option<bots::profit_effect::ProfitEffect>,
    rows: usize,
    mismatches: usize,
    profit_mismatches: usize,
}
struct Recorder {
    dir: PathBuf,
    ticks: Tick,
    record: Vec<String>,
    writers: OnceLock<Vec<Mutex<Option<AccountTrace>>>>,
    starts: Mutex<Vec<Value>>,
    error: Mutex<Option<String>>,
}

impl Recorder {
    fn register(&self, strategy: &str, start: &Start) -> Result<()> {
        let writers = self
            .writers
            .get_or_init(|| (0..start.n_accounts).map(|_| Mutex::new(None)).collect());
        if !self.record.is_empty()
            && !self
                .record
                .iter()
                .any(|x| x == strategy || x == &start.account)
        {
            return Ok(());
        }
        let file = File::create(
            self.dir
                .join(format!("account-{}.jsonl", start.account_index)),
        )?;
        *writers[start.account_index].lock().unwrap() = Some(AccountTrace {
            writer: BufWriter::with_capacity(128 * 1024, file),
            probe: (strategy == "swift").then(|| swift::Probe::new(start)),
            profit_probe: (strategy == "profit_effect")
                .then(|| bots::profit_effect::ProfitEffect::new(start)),
            rows: 0,
            mismatches: 0,
            profit_mismatches: 0,
        });
        self.starts
            .lock()
            .unwrap()
            .push(json!({"strategy":strategy,"start":start}));
        Ok(())
    }
    fn record(
        &self,
        a: &Account,
        tick: Tick,
        o: Option<&Observation>,
        actions: &[Action],
        gross: Money,
        end: bool,
    ) {
        if self.error.lock().unwrap().is_some() {
            return;
        }
        let mut guard = self.writers.get().unwrap()[a.idx].lock().unwrap();
        let Some(trace) = guard.as_mut() else {
            return;
        };
        let (signals, verified) = if let (Some(p), Some(o)) = (&mut trace.probe, o) {
            let (s, expected) = p.evaluate(o);
            let matched = expected == actions;
            if !matched {
                trace.mismatches += 1;
            }
            (Some(s), Some(matched))
        } else {
            (None, None)
        };
        let (profit_signals, profit_verified) =
            if let (Some(p), Some(o)) = (&mut trace.profit_probe, o) {
                let (s, expected) = p.evaluate(o);
                let matched = expected == actions;
                if !matched {
                    trace.profit_mismatches += 1;
                }
                (Some(s), Some(matched))
            } else {
                (None, None)
            };
        let quotes: Option<Vec<[i64; 11]>> = o.map(|o| {
            o.books
                .iter()
                .enumerate()
                .map(|(s, b)| {
                    let mid = b.mid().unwrap_or(b.last).max(1) as f64;
                    let bd = b
                        .bids
                        .iter()
                        .filter(|x| x.price as f64 >= mid * 0.98)
                        .map(|x| x.qty.max(0))
                        .sum();
                    let ad = b
                        .asks
                        .iter()
                        .filter(|x| x.price as f64 <= mid * 1.02)
                        .map(|x| x.qty.max(0))
                        .sum();
                    let buy = o.trades[s]
                        .iter()
                        .filter(|t| t.aggressor == Side::Buy)
                        .map(|t| t.qty)
                        .sum();
                    let sell = o.trades[s]
                        .iter()
                        .filter(|t| t.aggressor == Side::Sell)
                        .map(|t| t.qty)
                        .sum();
                    [
                        b.best_bid().unwrap_or(0),
                        b.best_ask().unwrap_or(0),
                        b.bids.first().map_or(0, |x| x.qty),
                        b.asks.first().map_or(0, |x| x.qty),
                        b.mark,
                        b.last,
                        b.dividend,
                        bd,
                        ad,
                        buy,
                        sell,
                    ]
                })
                .collect()
        });
        let row = json!({"tick":tick,"end":end,"decided":o.is_some(),"disabled":a.disabled,
            "pre":o.map(|o|&o.account),"reference_tick":o.map(|o|o.reference_tick),"quotes":quotes,
            "books":if trace.probe.is_some(){o.map(|o|&o.books)}else{None},
            "observed_trades":if trace.probe.is_some(){o.map(|o|&o.trades)}else{None},
            "open_orders":o.map(|o|&o.open_orders),"actions":actions,
            "fills":o.map(|o|o.fills.as_slice()).unwrap_or(if end{&a.inbox.fills}else{&[]}),
            "acks":o.map(|o|o.acks.as_slice()).unwrap_or(if end{&a.inbox.acks}else{&[]}),
            "rejects":o.map(|o|o.rejects.as_slice()).unwrap_or(if end{&a.inbox.rejects}else{&[]}),
            "news":o.map(|o|o.news.as_slice()).unwrap_or(if end{&a.inbox.news}else{&[]}),
            "income":o.map(|o|o.income.as_slice()).unwrap_or(if end{&a.inbox.income}else{&[]}),
            "terminal_hints":o.map(|o|o.terminal_hints.as_slice()).unwrap_or(if end{&a.inbox.terminal_hints}else{&[]}),
            "post":{"cash":a.cash,"shares":a.shares,"gross":if end{None}else{Some(gross)},"equity":if end{None}else{Some(a.cash+gross)}},
            "post_fills":if o.is_some(){&a.inbox.fills[..]}else{&[]},
            "post_acks":if o.is_some(){&a.inbox.acks[..]}else{&[]},
            "post_rejects":if o.is_some(){&a.inbox.rejects[..]}else{&[]},
            "stats":a.stats,"swift":signals,"swift_verified":verified,
            "social":o.and_then(|o|o.social.as_ref()),"profit_effect":profit_signals,"profit_effect_verified":profit_verified});
        let result = serde_json::to_writer(&mut trace.writer, &row)
            .map_err(std::io::Error::other)
            .and_then(|_| trace.writer.write_all(b"\n"));
        trace.rows += 1;
        if let Err(e) = result {
            *self.error.lock().unwrap() = Some(e.to_string());
        }
    }
    fn close(&self) -> Result<Value> {
        let mut accounts = Vec::new();
        for (idx, w) in self.writers.get().unwrap().iter().enumerate() {
            let mut w = w.lock().unwrap();
            let Some(t) = w.as_mut() else {
                continue;
            };
            t.writer.flush()?;
            accounts.push(json!({"index":idx,"rows":t.rows,"swift_mismatches":t.mismatches,"profit_effect_mismatches":t.profit_mismatches}));
        }
        if let Some(error) = self.error.lock().unwrap().as_ref() {
            bail!("trace recording failed: {error}");
        }
        Ok(json!(accounts))
    }
}
impl MatchObserver for Recorder {
    fn step(
        &self,
        a: &Account,
        t: Tick,
        o: Option<&Observation>,
        actions: &[Action],
        gross: Money,
    ) {
        self.record(a, t, o, actions, gross, false);
    }
    fn finish(&self, a: &Account) {
        self.record(a, self.ticks, None, &[], 0, true);
    }
}

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.is_empty() || !(args.len() - 1).is_multiple_of(2) {
        bail!("debug-run SCENARIO --out DIR [--seed N --threads N --ticks N --challenge BOT]");
    }
    let flags: BTreeMap<_, _> = args[1..]
        .as_chunks::<2>()
        .0
        .iter()
        .map(|p| (p[0].as_str(), p[1].as_str()))
        .collect();
    for k in flags.keys() {
        if ![
            "--out",
            "--seed",
            "--threads",
            "--ticks",
            "--challenge",
            "--record",
        ]
        .contains(k)
        {
            bail!("unknown flag {k}");
        }
    }
    let dest = PathBuf::from(flags.get("--out").context("--out required")?);
    if dest.exists() {
        bail!("output already exists: {}", dest.display());
    }
    let mut cfg = Config::load(std::path::Path::new(&args[0]))?;
    if let Some(t) = flags.get("--threads") {
        cfg.schedule.threads = t.parse()?;
    }
    if let Some(t) = flags.get("--ticks") {
        cfg.ticks = t.parse()?;
    }
    if let Some(bot) = flags.get("--challenge") {
        let mut g = cfg.bots[0].clone();
        g.bot = bot.to_string();
        g.news_multiplier = 1.0;
        g.forecast_access = Default::default();
        g.social_group = None;
        g.owner = Some("challenger".into());
        g.count = 1;
        g.count_max = None;
        g.cash = 2_000_000;
        g.shares_each = 20;
        g.shares.clear();
        g.params.clear();
        cfg.bots.push(g);
    }
    cfg.validate()?;
    let seed = flags
        .get("--seed")
        .map(|s| s.parse())
        .transpose()?
        .unwrap_or(cfg.seed);
    std::fs::create_dir_all(dest.join("trace"))?;
    let recorder = Recorder {
        dir: dest.join("trace"),
        ticks: cfg.ticks,
        record: flags
            .get("--record")
            .map(|v| v.split(',').map(str::to_string).collect())
            .unwrap_or_default(),
        writers: OnceLock::new(),
        starts: Mutex::new(Vec::new()),
        error: Mutex::new(None),
    };
    let factory = |name: &str, s: &Start| {
        recorder.register(name, s).expect("creating account trace");
        bots::factory(name, s)
    };
    let result = run_match_observed(
        &cfg,
        seed,
        &factory,
        Some(&dest.join("market")),
        Some(&recorder),
    )?;
    let verification = recorder.close()?;
    if recorder.starts.lock().unwrap().is_empty() {
        bail!("--record matched no accounts or strategies");
    }
    let manifest = json!({"schema":1,"source_scenario":args[0],"starts":*recorder.starts.lock().unwrap(),
        "verification":verification,"result":result,"quote_fields":["bid","ask","bid_qty","ask_qty","mark","last","dividend_rate","bid_depth_qty","ask_depth_qty","observed_buy_qty","observed_sell_qty"],
        "clock_note":"Personal decision ticks; fills.tick is matching tick. Final mailbox flush is a separate end record. Tracing changes concurrent timing."});
    std::fs::write(
        dest.join("trace-manifest.json"),
        serde_json::to_vec_pretty(&manifest)?,
    )?;
    println!(
        "Recorded {} accounts, {} ticks. Swift mismatches: {}",
        recorder.starts.lock().unwrap().len(),
        result.ticks,
        verification
            .as_array()
            .unwrap()
            .iter()
            .map(|x| x["swift_mismatches"].as_u64().unwrap())
            .sum::<u64>()
    );
    Ok(())
}
