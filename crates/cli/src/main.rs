//! llm-market run SCENARIO --out DIR [--seed N]
//! llm-market series SCENARIO --out DIR [--seeds N] [--first-seed N]
//! llm-market challenge SCENARIO --bot NAME --out DIR [--seeds N] [--first-seed N]

use anyhow::{bail, Context, Result};
use market::config::Config;
use market::report::{run_match, MatchResult};
use std::collections::BTreeMap;
use std::path::PathBuf;

fn usage() -> ! {
    eprintln!(
        "usage:\n  llm-market run <scenario.toml> --out <dir> [--seed N] [--ticks N] [--threads N]\n  \
         llm-market series <scenario.toml> --out <dir> [--seeds N] [--first-seed N] [--threads N]\n  \
         llm-market challenge <scenario.toml> --bot <name> --out <dir> [--seeds N] [--first-seed N] [--cash C] [--shares S] [--params k=v,...]\n  \
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
    let failed: Vec<String> = r
        .quality
        .checks
        .iter()
        .filter(|c| !c.pass)
        .map(|c| format!("{} {:.3}", c.name, c.value))
        .collect();
    if failed.is_empty() {
        println!("market quality: pass");
    } else {
        println!("market quality: FAIL ({})", failed.join(", "));
    }
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
            let (n, first) = seeds(&a, &cfg, 1)?;
            run_series(&cfg, &a.out, n, first)?;
        }
        "challenge" => {
            // one account of the challenger joins the scenario's market, like any other owner
            let a = parse(&args[1..]);
            let mut cfg = load(&a)?;
            let bot = a.flags.get("bot").cloned().unwrap_or_else(|| usage());
            let mut g = cfg.bots[0].clone();
            g.bot = bot.clone();
            g.owner = Some("challenger".into());
            g.count = 1;
            g.count_max = None;
            g.params = Default::default();
            // --params "k=v,k=v" sets the challenger's parameters
            if let Some(ps) = a.flags.get("params") {
                for kv in ps.split(',').filter(|x| !x.is_empty()) {
                    let (k, v) = kv.split_once('=').context("--params expects k=v pairs")?;
                    g.params.insert(
                        k.trim().to_string(),
                        v.trim().parse().context("--params value")?,
                    );
                }
            }
            g.cash = a
                .flags
                .get("cash")
                .map(|s| s.parse())
                .transpose()?
                .unwrap_or(2_000_000);
            g.shares_each = a
                .flags
                .get("shares")
                .map(|s| s.parse())
                .transpose()?
                .unwrap_or(20);
            g.shares = Default::default();
            cfg.bots.push(g);
            cfg.validate()?;
            let (n, first) = seeds(&a, &cfg, 1001)?;
            let (results, _) = run_series(&cfg, &a.out, n, first)?;
            let pct: Vec<f64> = results
                .iter()
                .map(|r| {
                    r.accounts
                        .iter()
                        .find(|x| x.owner == "challenger")
                        .unwrap()
                        .percentile
                })
                .collect();
            let (m, sd) = mean_sd(&pct);
            let top25 = pct.iter().filter(|p| **p >= 75.0).count() as f64 / pct.len() as f64;
            let pass = m >= 90.0 && top25 >= 0.8;
            println!(
                "\nchallenger {bot}: mean percentile {m:.1} ± {sd:.1}, top 25% in {:.0}% of worlds -> {}",
                top25 * 100.0,
                if pass { "BEATS the market" } else { "does not beat the market" }
            );
            println!("(bar: mean >= 90 and top 25% in at least 80% of worlds)");
            std::fs::write(
                a.out.join("challenge.json"),
                serde_json::to_string_pretty(&serde_json::json!({
                    "bot": bot, "seeds": results.iter().map(|r| r.seed).collect::<Vec<_>>(),
                    "percentiles": pct, "mean": m, "sd": sd, "top25_share": top25, "pass": pass,
                }))?,
            )?;
        }
        _ => usage(),
    }
    Ok(())
}

fn seeds(a: &Args, cfg: &Config, default_first: u64) -> Result<(u64, u64)> {
    let n = a
        .flags
        .get("seeds")
        .map(|s| s.parse())
        .transpose()?
        .unwrap_or(cfg.scoring.series_seeds as u64);
    let first = a
        .flags
        .get("first-seed")
        .map(|s| s.parse())
        .transpose()?
        .unwrap_or(default_first);
    Ok((n, first))
}

fn run_series(
    cfg: &Config,
    out: &std::path::Path,
    n: u64,
    first: u64,
) -> Result<(Vec<MatchResult>, serde_json::Value)> {
    if out.exists() {
        bail!("output directory {} already exists", out.display());
    }
    std::fs::create_dir_all(out)?;
    let mut results = Vec::new();
    for seed in first..first + n {
        let r = run_match(
            cfg,
            seed,
            &bots::factory,
            Some(&out.join(format!("seed-{seed}"))),
        )?;
        print_match(&r);
        results.push(r);
    }
    let summary = market_series(&results);
    std::fs::write(
        out.join("series.json"),
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
    println!("\nmarket quality over {n} worlds (median value, share of worlds passing)");
    for q in summary["quality"].as_array().unwrap() {
        println!(
            "{:<22}{:>10.3}  {} {:<8}{:>5.0}%",
            q["name"].as_str().unwrap(),
            q["median"].as_f64().unwrap_or(f64::NAN),
            if q["at_least"].as_bool().unwrap() {
                ">="
            } else {
                "<="
            },
            q["threshold"].as_f64().unwrap(),
            q["pass_share"].as_f64().unwrap() * 100.0
        );
    }
    println!(
        "all checks pass in {:.0}% of worlds",
        summary["quality_pass_share"].as_f64().unwrap() * 100.0
    );
    Ok((results, summary))
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
    let n = results.len() as f64;
    let quality: Vec<serde_json::Value> = results[0]
        .quality
        .checks
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let mut v: Vec<f64> = results.iter().map(|r| r.quality.checks[i].value).filter(|x| x.is_finite()).collect();
            v.sort_by(f64::total_cmp);
            let med = v.get(v.len() / 2).copied().unwrap_or(f64::NAN);
            let pass = results.iter().filter(|r| r.quality.checks[i].pass).count() as f64 / n;
            serde_json::json!({"name": c.name, "median": med, "threshold": c.threshold, "at_least": c.at_least, "pass_share": pass})
        })
        .collect();
    serde_json::json!({
        "worlds": results.iter().map(|r| r.seed).collect::<Vec<_>>(),
        "quality": quality,
        "quality_pass_share": results.iter().filter(|r| r.quality.pass).count() as f64 / n,
        "by_strategy": by_strategy.iter().map(|(k, v)| (k.clone(), stat(v))).collect::<serde_json::Map<_, _>>(),
        "by_owner": by_owner.iter().map(|(k, v)| (k.clone(), stat(v))).collect::<serde_json::Map<_, _>>(),
        "by_account": by_account.iter().map(|(k, (o, s, v))| {
            let mut x = stat(v); x["owner"] = o.clone().into(); x["strategy"] = s.clone().into(); (k.clone(), x)
        }).collect::<serde_json::Map<_, _>>(),
    })
}
