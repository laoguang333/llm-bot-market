//! Read-only aggregate recorder for one ordinary research account.
//! Every stock is recorded; hidden world labels are used only by offline analysis.
use anyhow::{bail, Result};
use bot_api::*;
use market::{account::Account, config::Config, engine::MatchObserver, report::run_match_observed};
use serde::Serialize;
use std::{path::Path, sync::Mutex};

#[derive(Clone, Default, Serialize)]
struct Ledger {
    bought: i64,
    sold: i64,
    fees: i64,
    dividends: i64,
}

#[derive(Serialize)]
struct Sample {
    tick: Tick,
    equity: i64,
    shares: Vec<i64>,
    marks: Vec<i64>,
    mids: Vec<i64>,
    dividends: Vec<i64>,
}

#[derive(Default, Serialize)]
struct Record {
    opening_prices: Vec<i64>,
    ledger: Vec<Ledger>,
    samples: Vec<Sample>,
}

impl Record {
    fn receipts(&mut self, fills: &[Fill], income: &[Income]) {
        for f in fills {
            let l = &mut self.ledger[f.stock];
            match f.side {
                Side::Buy => l.bought += f.price * f.qty,
                Side::Sell => l.sold += f.price * f.qty,
            }
            l.fees += f.fee;
        }
        for income in income {
            if let Income::Dividend { stock, amount } = *income {
                self.ledger[stock].dividends += amount;
            }
        }
    }
}

struct Observer(Mutex<Record>);
impl MatchObserver for Observer {
    fn step(&self, a: &Account, tick: Tick, obs: Option<&Observation>, _: &[Action], _: Money) {
        if a.owner != "osprey-research" {
            return;
        }
        if let Some(o) = obs {
            let mut r = self.0.lock().unwrap();
            r.receipts(&o.fills, &o.income);
            if tick.is_multiple_of(20) {
                r.samples.push(Sample {
                    tick,
                    equity: o.account.equity,
                    shares: o.account.shares.clone(),
                    marks: o.books.iter().map(|b| b.mark).collect(),
                    mids: o.books.iter().map(|b| b.mid().unwrap_or(b.last)).collect(),
                    dividends: o.books.iter().map(|b| b.dividend).collect(),
                });
            }
        }
    }
    fn finish(&self, a: &Account) {
        if a.owner == "osprey-research" {
            self.0
                .lock()
                .unwrap()
                .receipts(&a.inbox.fills, &a.inbox.income);
        }
    }
}

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 3 {
        bail!("osprey-eval SCENARIO SEED OUTPUT");
    }
    let cfg = Config::load(Path::new(&args[0]))?;
    let seed = args[1].parse()?;
    let out = Path::new(&args[2]);
    let observer = Observer(Mutex::new(Record {
        opening_prices: Vec::new(),
        ledger: vec![Ledger::default(); cfg.stocks.len()],
        samples: Vec::new(),
    }));
    let factory = |name: &str, s: &Start| {
        if s.owner == "osprey-research" {
            observer.0.lock().unwrap().opening_prices =
                s.rules.stocks.iter().map(|s| s.initial_price).collect();
        }
        bots::factory(name, s)
    };
    let result = run_match_observed(&cfg, seed, &factory, Some(out), Some(&observer))?;
    let r = observer.0.into_inner().unwrap();
    let account = result
        .accounts
        .iter()
        .find(|a| a.owner == "osprey-research")
        .ok_or_else(|| anyhow::anyhow!("missing research account"))?;
    if r.ledger.iter().map(|l| l.dividends).sum::<i64>() != account.stats.dividends {
        bail!("recorded dividend receipts disagree with final account stats");
    }
    if r.ledger.iter().map(|l| l.fees).sum::<i64>() != account.stats.trade_fees {
        bail!("recorded trade fees disagree with final account stats");
    }
    std::fs::write(out.join("stock-record.json"), serde_json::to_vec(&r)?)?;
    println!(
        "seed={seed} percentile={:.2} return={:.2}%",
        account.percentile,
        (account.return_ratio - 1.0) * 100.0
    );
    Ok(())
}
