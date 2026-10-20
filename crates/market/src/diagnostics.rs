//! Descriptive world-shape checks, separate from liquidity and engine invariants.
use crate::{config::Config, world::World};
use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
pub struct StockPath {
    pub stock: usize,
    pub effective_news: usize,
    pub mean_news_interval: Option<f64>,
    pub max_news_gap: u32,
    pub dividend_peak_initial_multiple: f64,
    /// Lowest strictly positive rate BEFORE a subsequent peak. Zero troughs are
    /// counted separately rather than silently divided by one cent.
    pub dividend_trough_to_later_peak: Option<f64>,
    pub zero_rate_ticks: u32,
    pub max_dividend_drawdown: f64,
    pub negative_updates: usize,
    pub peak_tick: u32,
}
#[derive(Clone, Debug, Serialize)]
pub struct WorldDiagnostics {
    pub stocks: Vec<StockPath>,
    pub mean_stock_news_interval: Option<f64>,
    /// Diagnostic target band, NOT an engine correctness requirement.
    pub cadence_near_100: bool,
    pub selected_dragon: Option<usize>,
    pub selected_peak_at_least_8x_initial: Option<bool>,
    pub selected_has_pullback: Option<bool>,
}
pub fn world(cfg: &Config, w: &World) -> WorldDiagnostics {
    let mut stocks = Vec::new();
    for s in 0..cfg.stocks.len() {
        let mut ticks: Vec<_> = w
            .events
            .iter()
            .filter(|e| e.effective < cfg.ticks && e.changes.iter().any(|c| c.0 == s))
            .map(|e| e.effective)
            .collect();
        ticks.sort_unstable();
        let count = ticks.len();
        let mut gap = 0;
        let mut prev = 0;
        for &t in ticks.iter().chain(std::iter::once(&cfg.ticks)) {
            gap = gap.max(t - prev);
            prev = t;
        }
        let initial = cfg.stocks[s].dividend;
        let mut peak = initial;
        let mut peak_tick = 0;
        let mut low = None::<i64>;
        let mut ratio = None::<f64>;
        let mut zero = 0;
        let mut dd = 0.0_f64;
        let mut negative = 0;
        let mut previous = initial;
        for t in 0..cfg.ticks {
            let rate = w.dividend(s, t);
            if rate > peak {
                peak = rate;
                peak_tick = t;
            }
            if peak > 0 {
                dd = dd.max(1.0 - rate as f64 / peak as f64);
            }
            if rate == 0 {
                zero += 1;
            } else {
                low = Some(low.map_or(rate, |l| l.min(rate)));
                let q = rate as f64 / low.unwrap() as f64;
                ratio = Some(ratio.map_or(q, |r| r.max(q)));
            }
            if rate < previous {
                negative += 1;
            }
            previous = rate;
        }
        stocks.push(StockPath {
            stock: s,
            effective_news: count,
            mean_news_interval: (count > 0).then(|| cfg.ticks as f64 / count as f64),
            max_news_gap: gap,
            dividend_peak_initial_multiple: peak as f64 / initial.max(1) as f64,
            dividend_trough_to_later_peak: ratio,
            zero_rate_ticks: zero,
            max_dividend_drawdown: dd,
            negative_updates: negative,
            peak_tick,
        });
    }
    let total: usize = stocks.iter().map(|s| s.effective_news).sum();
    let interval = (total > 0).then(|| cfg.ticks as f64 * stocks.len() as f64 / total as f64);
    let selected = w.dragon.as_ref().and_then(|d| d.selected_stock);
    WorldDiagnostics {
        mean_stock_news_interval: interval,
        cadence_near_100: interval.is_some_and(|x| (60.0..=150.0).contains(&x)),
        selected_dragon: selected,
        selected_peak_at_least_8x_initial: selected
            .map(|s| stocks[s].dividend_peak_initial_multiple >= 8.),
        selected_has_pullback: selected
            .map(|s| stocks[s].negative_updates > 0 && stocks[s].max_dividend_drawdown >= 0.1),
        stocks,
    }
}
