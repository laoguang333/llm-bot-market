//! Market-quality checks: does the market trade all game, with a tight two-sided book,
//! prices that wander around value without running away, and volatility that clusters?

use crate::engine::Shared;
use crate::report::reference_value;
use bot_api::Tick;
use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
pub struct Check {
    pub name: &'static str,
    /// Median across stocks.
    pub value: f64,
    pub threshold: f64,
    /// True when `value >= threshold`; false when it must stay at or below.
    pub at_least: bool,
    pub pass: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct Quality {
    pub pass: bool,
    pub checks: Vec<Check>,
}

const BUCKET: Tick = 20;

fn median(mut v: Vec<f64>) -> f64 {
    v.retain(|x| x.is_finite());
    if v.is_empty() {
        return f64::NAN;
    }
    v.sort_by(f64::total_cmp);
    let n = v.len();
    if n % 2 == 1 {
        v[n / 2]
    } else {
        (v[n / 2 - 1] + v[n / 2]) / 2.0
    }
}

fn autocorr1(x: &[f64]) -> f64 {
    if x.len() < 3 {
        return f64::NAN;
    }
    let m = x.iter().sum::<f64>() / x.len() as f64;
    let var: f64 = x.iter().map(|v| (v - m).powi(2)).sum();
    if var == 0.0 {
        return f64::NAN;
    }
    let cov: f64 = x.windows(2).map(|w| (w[0] - m) * (w[1] - m)).sum();
    cov / var
}

pub fn measure(shared: &Shared) -> Quality {
    let ticks = shared.cfg.ticks;
    let n_stocks = shared.markets.len();
    let samples = shared.book_samples.lock().clone();
    let n_buckets = ticks.div_ceil(BUCKET) as usize;
    let (mut coverage, mut gaps, mut two_sided, mut spreads, mut depth) =
        (vec![], vec![], vec![], vec![], vec![]);
    let (mut dev, mut extreme, mut crossings, mut clustering) = (vec![], vec![], vec![], vec![]);
    for s in 0..n_stocks {
        let tape: Vec<(Tick, i64)> = shared.markets[s]
            .read()
            .tape
            .iter()
            .map(|e| (e.tick, e.price))
            .collect();
        let trade_size = median(
            shared.markets[s]
                .read()
                .tape
                .iter()
                .map(|e| (e.price * e.qty) as f64)
                .collect(),
        );
        // continuity
        let mut close: Vec<Option<i64>> = vec![None; n_buckets];
        for &(t, p) in &tape {
            close[((t / BUCKET) as usize).min(n_buckets - 1)] = Some(p);
        }
        coverage.push(close.iter().filter(|c| c.is_some()).count() as f64 / n_buckets as f64);
        let mut last = 0;
        let mut gap = 0;
        for &(t, _) in &tape {
            gap = gap.max(t.saturating_sub(last));
            last = t;
        }
        gaps.push(gap.max(ticks.saturating_sub(last)) as f64);
        // book
        let mine: Vec<_> = samples.iter().filter(|b| b.stock == s).collect();
        let both: Vec<_> = mine
            .iter()
            .filter_map(|b| Some((b.bid?, b.ask?, b.bid_depth.min(b.ask_depth))))
            .collect();
        two_sided.push(both.len() as f64 / mine.len().max(1) as f64);
        spreads.push(median(
            both.iter()
                .map(|&(b, a, _)| (a - b) as f64 / ((a + b) as f64 / 2.0))
                .collect(),
        ));
        // depth on the thinner side, in multiples of the median trade
        depth.push(median(
            both.iter()
                .map(|&(_, _, d)| d as f64 / trade_size)
                .collect(),
        ));
        // price against value, on bucket closes carried forward
        let mut p = shared.world.opening_prices[s];
        let mut rel = Vec::with_capacity(n_buckets);
        let mut pub_rel = Vec::with_capacity(n_buckets);
        for (i, c) in close.iter().enumerate() {
            if let Some(c) = c {
                p = *c;
            }
            let t = i as Tick * BUCKET;
            let r = reference_value(shared, s, t).max(1) as f64;
            rel.push((p as f64 / r).ln());
            pub_rel.push((p as f64 / public_value(shared, s, t).max(1.0)).ln());
        }
        dev.push(median(rel.iter().map(|x| x.abs()).collect()));
        extreme.push(rel.iter().filter(|x| x.abs() > 2f64.ln()).count() as f64 / rel.len() as f64);
        let mut cross = 0;
        let mut sign = 0i8;
        // crossings are counted against the value the crowd can know (see public_value), so a world
        // whose hidden count is far from the middle is not marked down for a mispricing nobody
        // can see yet
        for x in &pub_rel {
            // a crossing needs a 5% excursion to the other side, so jitter at value does not count
            let s2 = if *x > 0.05 {
                1
            } else if *x < -0.05 {
                -1
            } else {
                0
            };
            if s2 != 0 && sign != 0 && s2 != sign {
                cross += 1;
            }
            if s2 != 0 {
                sign = s2;
            }
        }
        crossings.push(cross as f64);
        // volatility clustering: autocorrelation of absolute bucket returns
        let closes: Vec<f64> = close.iter().flatten().map(|&c| c as f64).collect();
        let abs_ret: Vec<f64> = closes
            .windows(2)
            .map(|w| (w[1] / w[0]).ln().abs())
            .collect();
        clustering.push(autocorr1(&abs_ret));
    }
    let check = |name, values: Vec<f64>, threshold, at_least: bool| {
        let value = median(values);
        let pass = if at_least {
            value >= threshold
        } else {
            value <= threshold
        };
        Check {
            name,
            value,
            threshold,
            at_least,
            pass,
        }
    };
    let checks = vec![
        check("trade_coverage", coverage, 0.9, true),
        check("max_gap_ticks", gaps, 200.0, false),
        check("two_sided_share", two_sided, 0.95, true),
        check("median_spread", spreads, 0.02, false),
        check("depth_2pct_in_trades", depth, 3.0, true),
        check("median_abs_log_dev", dev, 0.25, false),
        check("share_beyond_2x", extreme, 0.05, false),
        check("value_crossings", crossings, 2.0, true),
        check("abs_return_autocorr", clustering, 0.05, true),
    ];
    Quality {
        pass: checks.iter().all(|c| c.pass),
        checks,
    }
}

/// Hold value of one share as far as the crowd can know it: the hidden number of terminal payments
/// starts at the middle of its public range and moves toward the truth as the private hints narrow.
fn public_value(shared: &Shared, stock: usize, tick: Tick) -> f64 {
    let r = &shared.rules;
    let d = shared.world.dividend(stock, tick) as f64;
    let remaining = (r.ticks_total.saturating_sub(1) / r.dividend_period)
        .saturating_sub(tick / r.dividend_period) as f64;
    let span = (r.terminal_payments_max - r.terminal_payments_min).max(1) as f64;
    let width = r
        .terminal_hints
        .iter()
        .filter(|h| h.0 <= tick)
        .map(|h| h.1 as f64)
        .fold(span, f64::min);
    let mid = (r.terminal_payments_min + r.terminal_payments_max) as f64 / 2.0;
    let known = mid + (1.0 - width / span) * (shared.world.terminal_payments as f64 - mid);
    d * (remaining + known)
}
