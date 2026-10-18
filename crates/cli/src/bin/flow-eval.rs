//! Research-only observation interventions; hidden event labels are used only offline.
use anyhow::{bail, Context, Result};
use bot_api::*;
use market::{config::Config, report::run_match};
use std::{
    collections::{BTreeMap, VecDeque},
    fmt::Write as _,
    path::PathBuf,
    sync::{Arc, Mutex},
};

#[derive(Clone, Copy, Default)]
struct Point {
    tick: Tick,
    stock: usize,
    bid: Price,
    ask: Price,
    mid: f64,
    volume: f64,
    flow: f64,
    return30: f64,
    dividends: Money,
}

#[derive(Default)]
struct Signal {
    prices: VecDeque<(Tick, f64)>,
    volume: f64,
    signed: f64,
    dividends: Money,
    last: Option<Tick>,
    candle_on: bool,
}

impl Signal {
    fn update(&mut self, o: &Observation, s: usize, period: Tick) -> Point {
        let b = &o.books[s];
        let mid = b.mid().unwrap_or(b.last).max(1) as f64;
        let prior = o
            .tick
            .checked_sub(30)
            .and_then(|t| self.prices.iter().rev().find(|(x, _)| *x <= t))
            .map(|(_, p)| *p);
        let ret = prior.map_or(0.0, |p| mid / p - 1.0);
        self.prices.push_back((o.tick, mid));
        while self
            .prices
            .front()
            .is_some_and(|(t, _)| t.saturating_add(40) < o.tick)
        {
            self.prices.pop_front();
        }
        let dt = self.last.map_or(1, |t| o.tick.saturating_sub(t)).max(1) as f64;
        let decay = (-dt / 10.0).exp();
        self.volume *= decay;
        self.signed *= decay;
        for t in &o.trades[s] {
            self.volume += t.qty as f64;
            self.signed += t.qty as f64 * if t.aggressor == Side::Buy { 1.0 } else { -1.0 };
        }
        if self.last != Some(o.tick) && o.tick > 0 && o.tick.is_multiple_of(period) {
            self.dividends += b.dividend;
        }
        self.last = Some(o.tick);
        if ret >= 0.01 {
            self.candle_on = true;
        } else if ret <= 0.0 {
            self.candle_on = false;
        }
        Point {
            tick: o.tick,
            stock: s,
            bid: b.best_bid().unwrap_or(0),
            ask: b.best_ask().unwrap_or(0),
            mid,
            volume: self.volume,
            flow: self.signed / self.volume.max(1.0),
            return30: ret,
            dividends: self.dividends,
        }
    }
}

#[derive(Default)]
struct Trace {
    points: Vec<Point>,
    fills: Vec<(Fill, Point)>,
    unmatched_fills: usize,
    filtered_buys: usize,
    buy_actions: usize,
    calls: usize,
    peak: Money,
    max_drawdown: f64,
}

struct Probe {
    inner: Box<dyn Bot>,
    mode: String,
    signals: Vec<Signal>,
    previous: Vec<Point>,
    random_state: u64,
    period: Tick,
    record: bool,
    trace: Trace,
    output: Arc<Mutex<Trace>>,
}

fn random_side(state: &mut u64) -> Side {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    if *state & 1 == 0 {
        Side::Buy
    } else {
        Side::Sell
    }
}

fn modify(o: &Observation, mode: &str, state: &mut u64, signals: &[Signal]) -> Observation {
    let mut changed = o.clone();
    for (s, ts) in changed.trades.iter_mut().enumerate() {
        for t in ts {
            t.aggressor = match mode {
                "random" => random_side(state),
                "reverse" => t.aggressor.opposite(),
                "green" => {
                    if signals[s].candle_on {
                        Side::Buy
                    } else {
                        Side::Sell
                    }
                }
                _ => t.aggressor,
            };
        }
    }
    changed
}

impl Bot for Probe {
    fn on_tick(&mut self, o: &Observation) -> Vec<Action> {
        self.trace.calls += 1;
        self.trace.peak = self.trace.peak.max(o.account.equity);
        if self.trace.peak > 0 {
            self.trace.max_drawdown = self
                .trace
                .max_drawdown
                .max(1.0 - o.account.equity as f64 / self.trace.peak as f64);
        }
        if self.record {
            for fill in &o.fills {
                if let Some(p) = self
                    .previous
                    .get(fill.stock)
                    .filter(|p| p.tick == fill.tick)
                {
                    self.trace.fills.push((*fill, *p));
                } else {
                    self.trace.unmatched_fills += 1;
                }
            }
        }
        let points: Vec<_> = self
            .signals
            .iter_mut()
            .enumerate()
            .map(|(s, state)| state.update(o, s, self.period))
            .collect();
        if self.record && o.tick.is_multiple_of(5) {
            self.trace.points.extend_from_slice(&points);
        }
        let mut actions = if matches!(self.mode.as_str(), "random" | "reverse" | "green") {
            self.inner.on_tick(&modify(
                o,
                &self.mode,
                &mut self.random_state,
                &self.signals,
            ))
        } else {
            self.inner.on_tick(o)
        };
        actions.retain(|a| {
            if let Action::Limit {
                stock,
                side: Side::Buy,
                ..
            } = a
            {
                self.trace.buy_actions += 1;
                let green = points[*stock].return30 >= 0.01;
                let keep = match self.mode.as_str() {
                    "exclude_green" => !green,
                    "only_green" => green,
                    _ => true,
                };
                if !keep {
                    self.trace.filtered_buys += 1;
                }
                keep
            } else {
                true
            }
        });
        self.previous = points;
        actions
    }
    fn on_end(&mut self, r: &FinalResult) {
        if self.trace.peak > 0 {
            self.trace.max_drawdown = self
                .trace
                .max_drawdown
                .max(1.0 - r.equity as f64 / self.trace.peak as f64);
        }
        self.inner.on_end(r);
        *self.output.lock().unwrap() = std::mem::take(&mut self.trace);
    }
}

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() < 3 || !(args.len() - 1).is_multiple_of(2) {
        bail!("flow-eval SCENARIO --out DIR [--bot NAME --mode plain|flow|random|reverse|exclude_green|only_green|green --first-seed N --seeds N --trace 0|1]");
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
            "--bot",
            "--mode",
            "--first-seed",
            "--seeds",
            "--trace",
        ]
        .contains(key)
        {
            bail!("unknown flag {key}");
        }
    }
    let bot = *flags.get("--bot").unwrap_or(&"swift");
    let mode = *flags.get("--mode").unwrap_or(&"plain");
    if ![
        "plain",
        "flow",
        "random",
        "reverse",
        "exclude_green",
        "only_green",
        "green",
    ]
    .contains(&mode)
    {
        bail!("unknown mode");
    }
    if bot != "swift" && mode != "plain" {
        bail!("interventions only apply to Swift");
    }
    let out = PathBuf::from(flags.get("--out").context("--out required")?);
    if out.exists() {
        bail!("output exists: {}", out.display());
    }
    let first: u64 = flags.get("--first-seed").unwrap_or(&"21001").parse()?;
    let seeds: u64 = flags.get("--seeds").unwrap_or(&"40").parse()?;
    let record = *flags.get("--trace").unwrap_or(&"0") == "1";
    if seeds == 0 {
        bail!("zero seeds");
    }
    let base = Config::load(&PathBuf::from(&args[0]))?;
    std::fs::create_dir_all(&out)?;
    for seed in first..first + seeds {
        let mut cfg = base.clone();
        let mut g = cfg.bots[0].clone();
        g.bot = bot.into();
        g.owner = Some("challenger".into());
        g.count = 1;
        g.count_max = None;
        g.cash = 2_000_000;
        g.shares_each = 20;
        g.shares.clear();
        g.params.clear();
        if matches!(mode, "flow" | "random" | "reverse" | "green") {
            g.params.insert("depth".into(), 0.0);
            g.params.insert("momentum".into(), 0.0);
        }
        cfg.bots.push(g);
        cfg.validate()?;
        let trace = Arc::new(Mutex::new(Trace::default()));
        let factory = |name: &str, s: &Start| -> Option<Box<dyn Bot>> {
            let inner = bots::factory(name, s)?;
            if s.owner != "challenger" {
                return Some(inner);
            }
            Some(Box::new(Probe {
                inner,
                mode: mode.into(),
                signals: (0..s.shares.len()).map(|_| Signal::default()).collect(),
                previous: vec![],
                random_state: 0x56_78_ab_cd_12_34_ef_01,
                period: s.rules.dividend_period,
                record,
                trace: Trace::default(),
                output: Arc::clone(&trace),
            }))
        };
        let dir = out.join(format!("seed-{seed}"));
        let result = run_match(&cfg, seed, &factory, if record { Some(&dir) } else { None })?;
        std::fs::create_dir_all(&dir)?;
        std::fs::write(
            dir.join("summary.json"),
            serde_json::to_string_pretty(&result)?,
        )?;
        let t = trace.lock().unwrap();
        std::fs::write(
            dir.join("probe.json"),
            serde_json::to_string_pretty(&serde_json::json!({
                "bot":bot,"mode":mode,"calls":t.calls,"unmatched_fills":t.unmatched_fills,
                "buy_actions":t.buy_actions,"filtered_buys":t.filtered_buys,"max_drawdown":t.max_drawdown,
                "observed_fills":t.fills.len(),"signal_points":t.points.len(),"random_seed":"fixed independently of world"
            }))?,
        )?;
        if record {
            let mut csv = String::from(
                "tick,stock,bid,ask,mid,volume10,flow10,return30,cumulative_dividend\n",
            );
            for p in &t.points {
                writeln!(
                    csv,
                    "{},{},{},{},{},{:.6},{:.6},{:.8},{}",
                    p.tick,
                    p.stock,
                    p.bid,
                    p.ask,
                    p.mid,
                    p.volume,
                    p.flow,
                    p.return30,
                    p.dividends as f64 / cfg.world.dividend_scale as f64
                )?;
            }
            std::fs::write(dir.join("signals.csv"), csv)?;
            let mut csv = String::from(
                "tick,stock,side,price,qty,fee,volume10,flow10,return30,cumulative_dividend\n",
            );
            for (f, p) in &t.fills {
                writeln!(
                    csv,
                    "{},{},{:?},{},{},{},{:.6},{:.6},{:.8},{}",
                    f.tick,
                    f.stock,
                    f.side,
                    f.price,
                    f.qty,
                    f.fee,
                    p.volume,
                    p.flow,
                    p.return30,
                    p.dividends as f64 / cfg.world.dividend_scale as f64
                )?;
            }
            std::fs::write(dir.join("challenger-fills.csv"), csv)?;
        }
        let a = result
            .accounts
            .iter()
            .find(|a| a.owner == "challenger")
            .context("missing challenger")?;
        println!(
            "{seed} {bot}/{mode} rank={:.2} return={:.2}%",
            a.percentile,
            (a.return_ratio - 1.0) * 100.0
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn observation(tick: Tick, price: Price) -> Observation {
        Observation {
            tick,
            ticks_total: 3000,
            reference_tick: tick,
            season: Season {
                turbulent: false,
                rate_mult: 1.0,
                size_mult: 1.0,
            },
            account: AccountView::default(),
            open_orders: vec![],
            books: vec![BookView {
                last: price,
                ..BookView::default()
            }],
            trades: vec![vec![Trade {
                tick,
                price,
                qty: 17,
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
    #[test]
    fn randomization_is_repeatable_without_a_world_seed() {
        let mut x = 91;
        let mut y = 91;
        let a: Vec<_> = (0..1000).map(|_| random_side(&mut x)).collect();
        let b: Vec<_> = (0..1000).map(|_| random_side(&mut y)).collect();
        assert_eq!(a, b);
        assert!((400..600).contains(&a.iter().filter(|s| **s == Side::Buy).count()));
    }
    #[test]
    fn interventions_change_only_trade_directions() {
        let o = observation(51, 1000);
        let mut state = 91;
        for mode in ["random", "reverse", "green"] {
            let v = modify(&o, mode, &mut state, &[Signal::default()]);
            assert_eq!(v.trades[0][0].price, 1000);
            assert_eq!(v.trades[0][0].qty, 17);
            assert_eq!(v.trades[0][0].tick, 51);
            assert_eq!(o.trades[0][0].aggressor, Side::Buy);
        }
    }
    #[test]
    fn candle_requires_a_past_thirty_tick_price_and_uses_hysteresis() {
        let mut s = Signal::default();
        for t in 0..30 {
            assert_eq!(s.update(&observation(t, 100), 0, 50).return30, 0.0);
        }
        let p = s.update(&observation(30, 102), 0, 50);
        assert!((p.return30 - 0.02).abs() < 1e-10 && s.candle_on);
        s.update(&observation(31, 101), 0, 50);
        assert!(s.candle_on);
        s.update(&observation(32, 100), 0, 50);
        assert!(!s.candle_on);
    }
}
