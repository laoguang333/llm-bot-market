//! llm-market run SCENARIO --out DIR [--seed N]
//! llm-market series SCENARIO --out DIR [--seeds N] [--first-seed N]

use anyhow::{bail, Context, Result};
use market::config::Config;
use market::report::{run_match, MatchResult};
use std::collections::BTreeMap;
use std::path::PathBuf;

fn usage() -> ! {
    eprintln!(
        "usage:\n  llm-market run <scenario.toml> --out <dir> [--seed N] [--ticks N] [--threads N]\n  \
         llm-market series <scenario.toml> --out <dir> [--seeds N] [--first-seed N] [--threads N]\n  \
         llm-market bots"
    );
    std::process::exit(2);
}

struct Args {
    scenario: PathBuf,
    out: PathBuf,
    flags: BTreeMap<String, String>,
}

fn parse(args: &[String]) -> Args {
    if args.is_empty() {
        usage();
    }
    let scenario = PathBuf::from(&args[0]);
    let mut out = None;
    let mut flags = BTreeMap::new();
    let mut i = 1;
    while i < args.len() {
        let k = args[i].trim_start_matches("--").to_string();
        let v = args.get(i + 1).cloned().unwrap_or_else(|| usage());
        if k == "out" {
            out = Some(PathBuf::from(v));
        } else {
            flags.insert(k, v);
        }
        i += 2;
    }
    Args {
        scenario,
        out: out.unwrap_or_else(|| usage()),
        flags,
    }
}

fn load(a: &Args) -> Result<Config> {
    let mut cfg = Config::load(&a.scenario)?;
    if let Some(t) = a.flags.get("ticks") {
        cfg.ticks = t.parse().context("--ticks")?;
    }
    if let Some(t) = a.flags.get("threads") {
        cfg.schedule.threads = t.parse().context("--threads")?;
    }
    cfg.validate()?;
    Ok(cfg)
}

fn print_match(r: &MatchResult) {
    println!(
        "seed {} · {} accounts · {} trades · {} threads · {:.2}s",
        r.seed,
        r.accounts.len(),
        r.trades,
        r.threads,
        r.wall_seconds
    );
    let mut rows: Vec<_> = r.by_strategy.iter().collect();
    rows.sort_by(|a, b| b.1.mean_percentile.total_cmp(&a.1.mean_percentile));
    println!(
        "{:<18}{:>5}{:>12}{:>16}",
        "strategy", "n", "percentile", "mean equity"
    );
    for (k, g) in rows {
        println!(
            "{:<18}{:>5}{:>12.1}{:>16.0}",
            k,
            g.accounts,
            g.mean_percentile,
            g.mean_equity / 100.0
        );
    }
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(cmd) = args.first() else { usage() };
    match cmd.as_str() {
        "bots" => {
            for name in bots::NAMES {
                println!("{name}");
            }
        }
        "run" => {
            let a = parse(&args[1..]);
            let cfg = load(&a)?;
            let seed = a
                .flags
                .get("seed")
                .map(|s| s.parse())
                .transpose()?
                .unwrap_or(cfg.seed);
            let r = run_match(&cfg, seed, &bots::factory, Some(&a.out))?;
            print_match(&r);
            println!("report: {}", a.out.join("report.html").display());
        }
        "series" => {
            let a = parse(&args[1..]);
            let cfg = load(&a)?;
            if a.out.exists() {
                bail!("output directory {} already exists", a.out.display());
            }
            let n: u64 = a
                .flags
                .get("seeds")
                .map(|s| s.parse())
                .transpose()?
                .unwrap_or(cfg.scoring.series_seeds as u64);
            let first: u64 = a
                .flags
                .get("first-seed")
                .map(|s| s.parse())
                .transpose()?
                .unwrap_or(1);
            std::fs::create_dir_all(&a.out)?;
            let mut results = Vec::new();
            for seed in first..first + n {
                let dir = a.out.join(format!("seed-{seed}"));
                let r = run_match(&cfg, seed, &bots::factory, Some(&dir))?;
                print_match(&r);
                results.push(r);
            }
            let summary = market_series(&results);
            std::fs::write(
                a.out.join("series.json"),
                serde_json::to_string_pretty(&summary)?,
            )?;
            println!("\nseries over {n} worlds (mean ± sd of rank percentile)");
            let mut rows: Vec<_> = summary["by_strategy"].as_object().unwrap().iter().collect();
            rows.sort_by(|x, y| {
                y.1["mean"]
                    .as_f64()
                    .unwrap()
                    .total_cmp(&x.1["mean"].as_f64().unwrap())
            });
            for (k, v) in rows {
                println!(
                    "{:<18}{:>8.1} ± {:>5.1}",
                    k,
                    v["mean"].as_f64().unwrap(),
                    v["sd"].as_f64().unwrap()
                );
            }
        }
        _ => usage(),
    }
    Ok(())
}

fn mean_sd(xs: &[f64]) -> (f64, f64) {
    let n = xs.len() as f64;
    let m = xs.iter().sum::<f64>() / n;
    let v = if xs.len() > 1 {
        xs.iter().map(|x| (x - m).powi(2)).sum::<f64>() / (n - 1.0)
    } else {
        0.0
    };
    (m, v.sqrt())
}

fn market_series(results: &[MatchResult]) -> serde_json::Value {
    let mut by_account: BTreeMap<String, (String, String, Vec<f64>)> = BTreeMap::new();
    let mut by_strategy: BTreeMap<String, Vec<f64>> = BTreeMap::new();
    let mut by_owner: BTreeMap<String, Vec<f64>> = BTreeMap::new();
    for r in results {
        for a in &r.accounts {
            by_account
                .entry(a.account.clone())
                .or_insert_with(|| (a.owner.clone(), a.strategy.clone(), Vec::new()))
                .2
                .push(a.percentile);
        }
        for (k, g) in &r.by_strategy {
            by_strategy
                .entry(k.clone())
                .or_default()
                .push(g.mean_percentile);
        }
        for (k, g) in &r.by_owner {
            by_owner
                .entry(k.clone())
                .or_default()
                .push(g.mean_percentile);
        }
    }
    let stat = |v: &Vec<f64>| {
        let (m, sd) = mean_sd(v);
        serde_json::json!({"mean": m, "sd": sd, "per_world": v})
    };
    serde_json::json!({
        "worlds": results.iter().map(|r| r.seed).collect::<Vec<_>>(),
        "by_strategy": by_strategy.iter().map(|(k, v)| (k.clone(), stat(v))).collect::<serde_json::Map<_, _>>(),
        "by_owner": by_owner.iter().map(|(k, v)| (k.clone(), stat(v))).collect::<serde_json::Map<_, _>>(),
        "by_account": by_account.iter().map(|(k, (o, s, v))| {
            let mut x = stat(v); x["owner"] = o.clone().into(); x["strategy"] = s.clone().into(); (k.clone(), x)
        }).collect::<serde_json::Map<_, _>>(),
    })
}
