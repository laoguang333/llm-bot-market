//! Arena scoring: the challenger is ranked among the named strategies, not the crowd.
//!
//! Background crowds (Grid, Noise, ...) lose most of their capital in every world, so a
//! rank among all accounts mostly measures beating them. The league is every account
//! whose scenario group names an explicit `owner` (the incumbents) plus the challenger.

use crate::config::Config;
use crate::report::{rank_percentiles, AccountResult, MatchResult};
use serde::Serialize;

/// Explicit owners in scenario order. Each such group has exactly one account.
pub fn league_owners(cfg: &Config) -> Vec<String> {
    cfg.bots.iter().filter_map(|g| g.owner.clone()).collect()
}

/// Percentile (best 100, worst 0) of each league member among the league, by return
/// on its opening endowment. Owners missing from the match are skipped.
pub fn league_percentiles(r: &MatchResult, owners: &[String]) -> Vec<(String, f64)> {
    let members: Vec<&AccountResult> = owners
        .iter()
        .filter_map(|o| r.accounts.iter().find(|a| &a.owner == o))
        .collect();
    // same integer key as the market ranking, so ties agree
    let keys: Vec<_> = members
        .iter()
        .map(|a| ((a.equity as i128 * 1_000_000) / a.initial_equity.max(1) as i128) as i64)
        .collect();
    members
        .iter()
        .zip(rank_percentiles(&keys))
        .map(|(a, (_, p))| (a.owner.clone(), p))
        .collect()
}

/// Where an account's result came from, each as a fraction of its initial equity.
///
/// `net` equals dividends plus combos plus capital, minus trade, idle cash and idle stock
/// fees and interest, exactly. Dividends and combos are paid by the treasury and fees go to
/// it. `capital` is trade cash flow plus terminal value minus the opening shares at
/// opening prices (and any bad debt): price and inventory gains and losses. It includes
/// the fall in value of shares held while their remaining cash flow is paid out, so it
/// is not purely money taken from other accounts. Burned shares are valued at their
/// mark when burned, so the split between `idle_stock_fees` and `capital` is
/// approximate; the sum is not.
#[derive(Clone, Debug, Default, Serialize)]
pub struct Attribution {
    pub net: f64,
    pub dividends: f64,
    pub combos: f64,
    pub trade_fees: f64,
    pub idle_cash_fees: f64,
    pub idle_stock_fees: f64,
    pub interest: f64,
    pub capital: f64,
    /// Turnover that counted toward the idle-fee credit, as a multiple of initial equity.
    pub credited_turnover: f64,
}

pub fn attribution(a: &AccountResult) -> Attribution {
    let e0 = a.initial_equity.max(1) as f64;
    let s = &a.stats;
    let net = (a.equity - a.initial_equity) as f64;
    let explained =
        (s.dividends + s.combo_dividends - s.trade_fees - s.idle_cash_fees - s.interest) as f64;
    let residual = net - explained;
    Attribution {
        net: net / e0,
        dividends: s.dividends as f64 / e0,
        combos: s.combo_dividends as f64 / e0,
        trade_fees: s.trade_fees as f64 / e0,
        idle_cash_fees: s.idle_cash_fees as f64 / e0,
        idle_stock_fees: s.idle_stock_fees as f64 / e0,
        interest: s.interest as f64 / e0,
        capital: (residual + s.idle_stock_fees as f64) / e0,
        credited_turnover: s.credited_notional as f64 / e0,
    }
}

/// Mean of each field over worlds.
pub fn mean_attribution(xs: &[Attribution]) -> Attribution {
    let n = xs.len().max(1) as f64;
    let sum = |f: fn(&Attribution) -> f64| xs.iter().map(f).sum::<f64>() / n;
    Attribution {
        net: sum(|x| x.net),
        dividends: sum(|x| x.dividends),
        combos: sum(|x| x.combos),
        trade_fees: sum(|x| x.trade_fees),
        idle_cash_fees: sum(|x| x.idle_cash_fees),
        idle_stock_fees: sum(|x| x.idle_stock_fees),
        interest: sum(|x| x.interest),
        capital: sum(|x| x.capital),
        credited_turnover: sum(|x| x.credited_turnover),
    }
}
