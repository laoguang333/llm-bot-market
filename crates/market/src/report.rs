//! Scoring and output files.

use crate::config::Config;
use crate::engine::{BotFactory, Shared};
use anyhow::{bail, Context, Result};
use bot_api::{FinalResult, Money, Price, Qty, Side, Tick};
use serde::Serialize;
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;
use std::sync::atomic::Ordering;

#[derive(Clone, Debug, Serialize)]
pub struct AccountResult {
    pub account: String,
    pub owner: String,
    pub strategy: String,
    pub news_multiplier: f64,
    pub forecast_access: bot_api::ForecastAccess,
    pub forecasts_received: usize,
    /// Delivered updates per disclosure stage (including the final public stage).
    pub news_received_by_stage: Vec<usize>,
    pub cash: Money,
    pub deferred_idle_fee: Money,
    pub shares: Vec<Qty>,
    pub terminal_value: Money,
    pub equity: Money,
    /// Initial cash plus initial shares at the opening prices.
    pub initial_equity: Money,
    /// equity / initial_equity; accounts are ranked by this, so different endowments compare.
    pub return_ratio: f64,
    pub initial_cash: Money,
    pub initial_shares: Vec<Qty>,
    pub rank: usize,
    pub percentile: f64,
    pub disabled: String,
    pub stats: crate::account::Stats,
    pub liquidation_events: Vec<crate::account::LiquidationEvent>,
    /// Traded notional by counterparty owner.
    pub notional_by_counterparty: BTreeMap<String, Money>,
}

#[derive(Clone, Debug, Serialize)]
pub struct GroupResult {
    pub accounts: usize,
    pub mean_percentile: f64,
    pub mean_equity: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct MatchResult {
    pub seed: u64,
    pub ticks: Tick,
    pub dividend_period: Tick,
    pub dividend_scale: i64,
    pub combo_period: Tick,
    pub combo_scale: i64,
    pub terminal_payment_ticks: Tick,
    pub terminal_payments: i64,
    pub threads: usize,
    pub decision_budget_us: u64,
    pub skip_decision_lag: Tick,
    pub max_tick_skew: Tick,
    pub wall_seconds: f64,
    pub audits: u64,
    pub news_events: usize,
    /// Revealed only after the match. Indexes refer to scenario stocks.
    pub dragon: Option<crate::world::DragonOutcome>,
    pub season_changes: usize,
    pub final_dividends: Vec<Money>,
    pub final_marks: Vec<Price>,
    pub shares_burned: Vec<Qty>,
    pub treasury: Money,
    pub trades: usize,
    pub quality: crate::quality::Quality,
    pub world_diagnostics: crate::diagnostics::WorldDiagnostics,
    pub by_strategy: BTreeMap<String, GroupResult>,
    pub by_owner: BTreeMap<String, GroupResult>,
    pub accounts: Vec<AccountResult>,
}

fn initial_equity(shared: &Shared, a: &crate::account::Account) -> Money {
    a.initial_cash
        + a.initial_shares
            .iter()
            .zip(&shared.world.opening_prices)
            .map(|(q, p)| q * p)
            .sum::<Money>()
}

/// Percentiles with ties averaged: best = 100, worst = 0.
pub fn rank_percentiles(equity: &[Money]) -> Vec<(usize, f64)> {
    let n = equity.len();
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by_key(|&i| std::cmp::Reverse(equity[i]));
    let mut out = vec![(0, 0.0); n];
    let mut i = 0;
    while i < n {
        let mut j = i;
        while j + 1 < n && equity[order[j + 1]] == equity[order[i]] {
            j += 1;
        }
        let avg_rank = (i + j) as f64 / 2.0;
        let pct = if n == 1 {
            100.0
        } else {
            100.0 * (n as f64 - 1.0 - avg_rank) / (n as f64 - 1.0)
        };
        for &k in &order[i..=j] {
            out[k] = (i + 1, pct);
        }
        i = j + 1;
    }
    out
}

fn group(
    results: &[AccountResult],
    key: impl Fn(&AccountResult) -> String,
) -> BTreeMap<String, GroupResult> {
    let mut m: BTreeMap<String, (usize, f64, f64)> = BTreeMap::new();
    for r in results {
        let e = m.entry(key(r)).or_default();
        e.0 += 1;
        e.1 += r.percentile;
        e.2 += r.equity as f64;
    }
    m.into_iter()
        .map(|(k, (n, p, e))| {
            (
                k,
                GroupResult {
                    accounts: n,
                    mean_percentile: p / n as f64,
                    mean_equity: e / n as f64,
                },
            )
        })
        .collect()
}

/// Runs one match. Writes the output directory when `out` is given.
pub fn run_match(
    cfg: &Config,
    seed: u64,
    factory: &BotFactory<'_>,
    out: Option<&Path>,
) -> Result<MatchResult> {
    run_match_observed(cfg, seed, factory, out, None)
}

pub fn run_match_observed(
    cfg: &Config,
    seed: u64,
    factory: &BotFactory<'_>,
    out: Option<&Path>,
    observer: Option<&dyn crate::engine::MatchObserver>,
) -> Result<MatchResult> {
    if let Some(dir) = out {
        if dir.exists() {
            bail!("output directory {} already exists", dir.display());
        }
    }
    let mut shared = Shared::new(cfg, seed, factory)?;
    shared.observer = observer;
    let run = shared.run()?;
    let rules = &shared.rules;
    let final_div = shared.world.dividends(rules.last_tick());
    let mut accounts: Vec<AccountResult> = Vec::new();
    let owners: Vec<String> = shared
        .accounts
        .iter()
        .map(|a| a.lock().owner.clone())
        .collect();
    let mut by_cp: Vec<BTreeMap<String, Money>> = vec![BTreeMap::new(); owners.len()];
    let mut n_trades = 0;
    for m in &shared.markets {
        let m = m.read();
        n_trades += m.tape.len();
        for e in &m.tape {
            let v = e.price * e.qty;
            *by_cp[e.taker as usize]
                .entry(owners[e.maker as usize].clone())
                .or_default() += v;
            *by_cp[e.maker as usize]
                .entry(owners[e.taker as usize].clone())
                .or_default() += v;
        }
    }
    for (i, acc) in shared.accounts.iter().enumerate() {
        let a = acc.lock();
        let tv = rules.terminal_value(&a.shares, &final_div, shared.world.terminal_payments);
        accounts.push(AccountResult {
            account: a.name.clone(),
            owner: a.owner.clone(),
            strategy: a.strategy.clone(),
            news_multiplier: shared.world.news_multipliers[i],
            forecast_access: shared.world.forecast_access[i],
            forecasts_received: shared.world.deliveries[i]
                .iter()
                .take(a.delivery_cursor)
                .filter(|d| d.forecast)
                .count(),
            news_received_by_stage: {
                let mut counts = vec![0; rules.news_stages.len()];
                for delivery in shared.world.deliveries[i].iter().take(a.delivery_cursor) {
                    if !delivery.forecast {
                        counts[delivery.stage] += 1;
                    }
                }
                counts
            },
            cash: a.cash,
            deferred_idle_fee: a.idle_fee_carry,
            shares: a.shares.clone(),
            terminal_value: tv,
            equity: a.cash + tv,
            initial_equity: initial_equity(&shared, &a),
            return_ratio: (a.cash + tv) as f64 / initial_equity(&shared, &a).max(1) as f64,
            initial_cash: a.initial_cash,
            initial_shares: a.initial_shares.clone(),
            rank: 0,
            percentile: 0.0,
            disabled: a.disabled.clone().unwrap_or_default(),
            stats: a.stats.clone(),
            liquidation_events: a.liquidation_events.clone(),
            notional_by_counterparty: std::mem::take(&mut by_cp[i]),
        });
    }
    // rank by return on the opening endowment (same order as final equity when endowments match)
    let ranks = rank_percentiles(
        &accounts
            .iter()
            .map(|a| {
                (a.cash + a.terminal_value) as i128 * 1_000_000 / a.initial_equity.max(1) as i128
            })
            .map(|x| x as Money)
            .collect::<Vec<_>>(),
    );
    for (a, (r, p)) in accounts.iter_mut().zip(ranks) {
        a.rank = r;
        a.percentile = p;
    }
    for (acc, r) in shared.accounts.iter().zip(&accounts) {
        let mut a = acc.lock();
        if let Some(bot) = a.bot.as_mut() {
            let res = FinalResult {
                equity: r.equity,
                rank: r.rank,
                percentile: r.percentile,
            };
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| bot.on_end(&res)));
        }
    }
    let result = MatchResult {
        seed,
        ticks: cfg.ticks,
        terminal_payments: shared.world.terminal_payments,
        dividend_period: rules.dividend_period,
        dividend_scale: rules.dividend_scale,
        combo_period: rules.combo_period,
        combo_scale: rules.combo_scale,
        terminal_payment_ticks: rules.terminal_payment_ticks,
        threads: run.threads,
        decision_budget_us: cfg.schedule.decision_budget_us,
        skip_decision_lag: cfg.schedule.skip_decision_lag,
        max_tick_skew: cfg.schedule.max_tick_skew,
        wall_seconds: run.wall_seconds,
        audits: shared.audits.load(Ordering::Relaxed),
        news_events: shared.world.events.len(),
        dragon: shared.world.dragon.clone(),
        season_changes: shared.world.season_changes.len(),
        final_dividends: final_div,
        final_marks: shared.marks(),
        shares_burned: shared
            .burned
            .iter()
            .map(|b| b.load(Ordering::Relaxed))
            .collect(),
        treasury: shared.treasury.load(Ordering::Relaxed),
        trades: n_trades,
        quality: crate::quality::measure(&shared),
        world_diagnostics: crate::diagnostics::world(cfg, &shared.world),
        by_strategy: group(&accounts, |a| a.strategy.clone()),
        by_owner: group(&accounts, |a| a.owner.clone()),
        accounts,
    };
    if let Some(dir) = out {
        write_outputs(cfg, &shared, &result, dir)?;
    }
    Ok(result)
}

struct Bar {
    open: Price,
    high: Price,
    low: Price,
    close: Price,
    volume: Qty,
    notional: Money,
}

fn bars(shared: &Shared, stock: usize, ticks: Tick, width: Tick) -> Vec<Option<Bar>> {
    let m = shared.markets[stock].read();
    let n = ticks.div_ceil(width) as usize;
    let mut out: Vec<Option<Bar>> = (0..n).map(|_| None).collect();
    for e in &m.tape {
        let i = ((e.tick / width) as usize).min(n - 1);
        match &mut out[i] {
            None => {
                out[i] = Some(Bar {
                    open: e.price,
                    high: e.price,
                    low: e.price,
                    close: e.price,
                    volume: e.qty,
                    notional: e.price * e.qty,
                })
            }
            Some(b) => {
                b.high = b.high.max(e.price);
                b.low = b.low.min(e.price);
                b.close = e.price;
                b.volume += e.qty;
                b.notional += e.price * e.qty;
            }
        }
    }
    out
}

fn write_outputs(cfg: &Config, shared: &Shared, result: &MatchResult, dir: &Path) -> Result<()> {
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    std::fs::write(
        dir.join("summary.json"),
        serde_json::to_string_pretty(result)?,
    )?;
    let names: Vec<&str> = cfg.stocks.iter().map(|s| s.name.as_str()).collect();

    let mut csv = String::from(
        "tick,stock,open,high,low,close,volume,notional,dividend,dividend_cents_per_payment\n",
    );
    let per_stock: Vec<Vec<Option<Bar>>> = (0..names.len())
        .map(|s| bars(shared, s, cfg.ticks, 1))
        .collect();
    for t in 0..cfg.ticks as usize {
        for (s, b) in per_stock.iter().enumerate() {
            let d = shared.world.dividend(s, t as Tick);
            let cents = shared.rules.dividend_cents(d);
            match &b[t] {
                Some(b) => writeln!(
                    csv,
                    "{t},{},{},{},{},{},{},{},{d},{cents}",
                    names[s], b.open, b.high, b.low, b.close, b.volume, b.notional
                )?,
                None => writeln!(csv, "{t},{},,,,,0,0,{d},{cents}", names[s])?,
            }
        }
    }
    std::fs::write(dir.join("bars.csv"), csv)?;

    let acct_names: Vec<String> = shared
        .accounts
        .iter()
        .map(|a| a.lock().name.clone())
        .collect();
    let mut csv = String::from("tick,stock,price,qty,aggressor,taker,maker\n");
    for (s, m) in shared.markets.iter().enumerate() {
        for e in &m.read().tape {
            writeln!(
                csv,
                "{},{},{},{},{},{},{}",
                e.tick,
                names[s],
                e.price,
                e.qty,
                if e.aggressor == Side::Buy {
                    "buy"
                } else {
                    "sell"
                },
                acct_names[e.taker as usize],
                acct_names[e.maker as usize]
            )?;
        }
    }
    std::fs::write(dir.join("trades.csv"), csv)?;

    let mut csv = String::from(
        "tick,stock,bid,ask,bid_depth_2pct,ask_depth_2pct,bid_shares_2pct,ask_shares_2pct\n",
    );
    for b in shared.book_samples.lock().iter() {
        let opt = |p: Option<Price>| p.map(|x| x.to_string()).unwrap_or_default();
        writeln!(
            csv,
            "{},{},{},{},{},{},{},{}",
            b.tick,
            names[b.stock],
            opt(b.bid),
            opt(b.ask),
            b.bid_depth,
            b.ask_depth,
            b.bid_shares,
            b.ask_shares
        )?;
    }
    std::fs::write(dir.join("books.csv"), csv)?;

    let mut csv = String::from("id,layer,sector,first_reveal,public_tick,effective,changes\n");
    for e in &shared.world.events {
        let ch: Vec<String> = e
            .changes
            .iter()
            .map(|(s, d)| format!("{}:{d}", names[*s]))
            .collect();
        writeln!(
            csv,
            "{},{:?},{},{},{},{},{}",
            e.id,
            e.layer,
            e.sector
                .map(|s| cfg.world.sectors[s].clone())
                .unwrap_or_default(),
            e.first_reveal,
            e.public_tick,
            e.effective,
            ch.join(" ")
        )?;
    }
    std::fs::write(dir.join("news.csv"), csv)?;

    let mut csv = String::from(
        "account,owner,strategy,rank,percentile,equity,cash,terminal_value,trade_fees,idle_fees,interest,dividends,combo_dividends,volume,trades,skipped,decisions,liquidations,bad_debt,max_leverage,mean_decision_us,disabled\n",
    );
    for a in &result.accounts {
        let st = &a.stats;
        let mean_us = if st.decisions > 0 {
            st.decision_nanos_total as f64 / st.decisions as f64 / 1000.0
        } else {
            0.0
        };
        writeln!(
            csv,
            "{},{},{},{},{:.2},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{:.2},{:.1},{}",
            a.account,
            a.owner,
            a.strategy,
            a.rank,
            a.percentile,
            a.equity,
            a.cash,
            a.terminal_value,
            st.trade_fees,
            st.idle_fees,
            st.interest,
            st.dividends,
            st.combo_dividends,
            st.volume_notional,
            st.trades,
            st.skipped,
            st.decisions,
            st.liquidations,
            st.bad_debt,
            st.max_leverage,
            mean_us,
            a.disabled
        )?;
    }
    std::fs::write(dir.join("accounts.csv"), csv)?;
    std::fs::write(dir.join("report.html"), report_html(cfg, shared, result)?)?;
    Ok(())
}

/// Per-share value of holding to the end: in-game dividends still to come plus the terminal value.
pub fn reference_value(shared: &Shared, stock: usize, tick: Tick) -> Money {
    let r = &shared.rules;
    let d = shared.world.dividend(stock, tick);
    let remaining = (r.ticks_total.saturating_sub(1) / r.dividend_period)
        .saturating_sub(tick / r.dividend_period) as i64;
    (r.dividend_cents(d)
        * (remaining as f64 + r.terminal_dividend_payments(shared.world.terminal_payments as f64)))
    .round() as Money
}

fn correlation(a: &[f64], b: &[f64]) -> f64 {
    let n = a.len().min(b.len());
    if n < 3 {
        return 0.0;
    }
    let ma = a[..n].iter().sum::<f64>() / n as f64;
    let mb = b[..n].iter().sum::<f64>() / n as f64;
    let (mut c, mut va, mut vb) = (0.0, 0.0, 0.0);
    for i in 0..n {
        c += (a[i] - ma) * (b[i] - mb);
        va += (a[i] - ma).powi(2);
        vb += (b[i] - mb).powi(2);
    }
    if va == 0.0 || vb == 0.0 {
        0.0
    } else {
        c / (va * vb).sqrt()
    }
}

fn report_html(cfg: &Config, shared: &Shared, result: &MatchResult) -> Result<String> {
    // returns for the correlation matrix use 5-tick closes; candles are wide enough for ~150 per chart
    const W: Tick = 5;
    let cw: Tick = (cfg.ticks / 150).div_ceil(W).max(1) * W;
    let n_stocks = cfg.stocks.len();
    let mut stocks = Vec::new();
    let mut closes: Vec<Vec<f64>> = Vec::new();
    for s in 0..n_stocks {
        let mut last = shared.world.opening_prices[s];
        let mut close_series = Vec::new();
        for bar in bars(shared, s, cfg.ticks, W) {
            if let Some(x) = bar {
                last = x.close;
            }
            close_series.push(last as f64);
        }
        let mut last = shared.world.opening_prices[s];
        let mut candles = Vec::new();
        for (i, bar) in bars(shared, s, cfg.ticks, cw).iter().enumerate() {
            let t = i as Tick * cw;
            let reference = reference_value(shared, s, t);
            match bar {
                Some(x) => {
                    candles.push(serde_json::json!([
                        x.open, x.high, x.low, x.close, x.volume, reference
                    ]));
                    last = x.close;
                }
                None => candles.push(serde_json::json!([null, null, null, last, 0, reference])),
            }
        }
        closes.push(close_series);
        let news: Vec<_> = shared
            .world
            .events
            .iter()
            .filter_map(|e| {
                e.changes.iter().find(|(st, _)| *st == s).map(|(_, d)| {
                    serde_json::json!([e.first_reveal, e.public_tick, e.effective, d])
                })
            })
            .collect();
        stocks.push(serde_json::json!({
            "name": cfg.stocks[s].name, "sector": cfg.stocks[s].sector, "candles": candles, "news": news,
        }));
    }
    let returns: Vec<Vec<f64>> = closes
        .iter()
        .map(|c| {
            c.windows(2)
                .map(|w| if w[0] > 0.0 { (w[1] / w[0]).ln() } else { 0.0 })
                .collect()
        })
        .collect();
    let skip = (200 / W) as usize;
    let corr: Vec<Vec<f64>> = (0..n_stocks)
        .map(|i| {
            (0..n_stocks)
                .map(|j| {
                    let a = returns[i].get(skip..).unwrap_or(&[]);
                    let b = returns[j].get(skip..).unwrap_or(&[]);
                    (correlation(a, b) * 100.0).round() / 100.0
                })
                .collect()
        })
        .collect();
    let combos: Vec<_> = shared
        .rules
        .combos
        .iter()
        .map(|c| {
            serde_json::json!([
                cfg.stocks[c.a].name,
                cfg.stocks[c.b].name,
                shared.rules.combo_cents(c.bonus)
            ])
        })
        .collect();
    let seasons: Vec<_> = shared
        .world
        .season_changes
        .iter()
        .map(|(t, s)| serde_json::json!([t, s]))
        .collect();
    let data = serde_json::json!({
        "seed": result.seed, "ticks": cfg.ticks, "width": cw, "stocks": stocks, "corr": corr, "combos": combos,
        "seasons": seasons, "by_strategy": result.by_strategy, "wall": result.wall_seconds, "threads": result.threads,
        "combo_period": result.combo_period,
        "accounts": result.accounts.len(), "trades": result.trades,
    });
    Ok(include_str!("report.html").replace("/*DATA*/null", &serde_json::to_string(&data)?))
}
