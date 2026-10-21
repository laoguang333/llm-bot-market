//! Scenario TOML.

use anyhow::{bail, Context, Result};
use bot_api::{
    ComboRule, DividendRiskRules, DragonModel, DragonRules, Money, NewsLayerRules, Price, Qty,
    RandomEndRules, Rules, StockInfo, Tick, TrendRules,
};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub ticks: Tick,
    pub seed: u64,
    #[serde(default)]
    pub market: MarketCfg,
    #[serde(default)]
    pub fees: FeesCfg,
    #[serde(default)]
    pub margin: MarginCfg,
    #[serde(default)]
    pub rules: RulesCfg,
    #[serde(default)]
    pub schedule: ScheduleCfg,
    pub world: WorldCfg,
    pub stocks: Vec<StockCfg>,
    #[serde(default)]
    pub combos: Vec<ComboCfg>,
    #[serde(default)]
    pub scoring: ScoringCfg,
    pub bots: Vec<BotGroupCfg>,
    /// Optional, conserved redistribution before bots receive Start.
    #[serde(default)]
    pub capital_redistribution: Option<CapitalRedistributionCfg>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapitalRedistributionCfg {
    /// Only anonymous background groups participate, never named challengers.
    pub from_bot: String,
    pub cash_each: Money,
    pub shares_each: Qty,
    /// Relative pool weights by strategy; each strategy's share is split equally
    /// among its anonymous accounts after population sampling.
    pub recipients: BTreeMap<String, u32>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct MarketCfg {
    pub tick_size: Price,
    pub book_depth: usize,
    pub trade_tape_limit: usize,
    /// When set, each stock opens at its expected hold value (initial dividend times in-game
    /// payments plus the midpoint of the terminal payment range) times a random factor in
    /// [1 - opening_spread, 1 + opening_spread], instead of its configured initial_price.
    pub opening_spread: Option<f64>,
}
impl Default for MarketCfg {
    fn default() -> Self {
        Self {
            tick_size: 1,
            book_depth: 10,
            trade_tape_limit: 200,
            opening_spread: None,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct FeesCfg {
    pub trade_fee_bps: i64,
    pub idle_period: Tick,
    pub idle_rate: f64,
    pub credit_ratio: f64,
    pub credit_side: String,
}
impl Default for FeesCfg {
    fn default() -> Self {
        Self {
            trade_fee_bps: 5,
            idle_period: 50,
            idle_rate: 0.002,
            credit_ratio: 0.01,
            credit_side: "both".into(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct MarginCfg {
    pub max_leverage: f64,
    pub maintenance_leverage: f64,
    pub loan_rate: f64,
    pub interest_period: Tick,
    pub mark_window: Tick,
}
impl Default for MarginCfg {
    fn default() -> Self {
        Self {
            max_leverage: 4.0,
            maintenance_leverage: 5.0,
            loan_rate: 0.003,
            interest_period: 50,
            mark_window: 20,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct RulesCfg {
    pub max_actions_per_tick: usize,
    pub max_open_orders: usize,
}
impl Default for RulesCfg {
    fn default() -> Self {
        Self {
            max_actions_per_tick: 20,
            max_open_orders: 40,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct ScheduleCfg {
    /// 0 = available logical CPUs (half on Windows). Explicit values are also
    /// capped to half the available CPUs on Windows; Linux has no such cap.
    pub threads: usize,
    pub max_tick_skew: Tick,
    pub skip_decision_lag: Tick,
    /// Skip lagging bots only if at least six of their last eight decisions exceeded
    /// this wall-time allowance. 0 restores the original lag-only policy.
    pub decision_budget_us: u64,
    /// Pause and audit conservation every this many ticks of the slowest account (0 = only at the end).
    pub check_interval: Tick,
    pub mailbox_capacity: usize,
}
impl Default for ScheduleCfg {
    fn default() -> Self {
        Self {
            threads: 0,
            max_tick_skew: 16,
            skip_decision_lag: 2,
            decision_budget_us: 2000,
            check_interval: 500,
            mailbox_capacity: 100_000,
        }
    }
}

impl ScheduleCfg {
    pub fn worker_threads(&self, accounts: usize) -> usize {
        let cpus = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1);
        worker_threads(self.threads, accounts, cpus, cfg!(windows))
    }
}

fn worker_threads(requested: usize, accounts: usize, cpus: usize, windows: bool) -> usize {
    let budget = if windows {
        (cpus / 2).max(1)
    } else {
        cpus.max(1)
    };
    let requested = if requested == 0 { budget } else { requested };
    let requested = if windows {
        requested.min(budget)
    } else {
        requested
    };
    requested.min(accounts.max(1))
}

#[cfg(test)]
mod thread_tests {
    use super::worker_threads;

    #[test]
    fn platform_cpu_budgets_apply_to_auto_and_explicit_threads() {
        assert_eq!(worker_threads(0, 100, 20, true), 10);
        assert_eq!(worker_threads(40, 100, 20, true), 10);
        assert_eq!(worker_threads(4, 100, 20, true), 4);
        assert_eq!(worker_threads(0, 100, 7, true), 3);
        assert_eq!(worker_threads(0, 100, 1, true), 1);
        assert_eq!(worker_threads(0, 100, 20, false), 20);
        assert_eq!(worker_threads(40, 100, 20, false), 40);
        assert_eq!(worker_threads(0, 2, 20, false), 2);
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorldCfg {
    #[serde(default = "default_dividend_period")]
    pub dividend_period: Tick,
    /// Dividend/news rate units per cent. Integer rates retain sub-cent precision;
    /// payments round the account/stock total, without carrying a remainder.
    #[serde(default = "one_qty")]
    pub dividend_scale: i64,
    /// Defaults to the ordinary period for legacy scenarios.
    pub combo_period: Option<Tick>,
    /// Combination reward rate units per cent; legacy scenarios use integer cents.
    #[serde(default = "one_qty")]
    pub combo_scale: i64,
    #[serde(default)]
    pub min_dividend: Money,
    pub sectors: Vec<String>,
    #[serde(default)]
    pub seasons: SeasonsCfg,
    pub news: NewsCfg,
    #[serde(default)]
    pub dragon: DragonCfg,
    /// Opt-in persistent cuts and suspensions; absent preserves historical worlds.
    pub dividend_risk: Option<DividendRiskRules>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct DragonCfg {
    pub model: DragonModel,
    pub trend: TrendRules,
    pub enabled: bool,
    pub probability: f64,
    pub candidates: usize,
    /// Fraction of match duration at which the selected path may diverge.
    /// Defaults to the archived scenarios' 55% boundary.
    pub start_fraction: f64,
    pub peak_extra: (f64, f64),
}

impl Default for DragonCfg {
    fn default() -> Self {
        Self {
            model: DragonModel::Pulses,
            trend: TrendRules::default(),
            enabled: false,
            probability: 0.5,
            candidates: 3,
            start_fraction: 0.55,
            peak_extra: (2.0, 4.0),
        }
    }
}
fn default_dividend_period() -> Tick {
    50
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct SeasonsCfg {
    pub calm_to_turbulent: f64,
    pub turbulent_to_calm: f64,
    pub turbulent_rate_mult: f64,
    pub turbulent_size_mult: f64,
}
impl Default for SeasonsCfg {
    fn default() -> Self {
        Self {
            calm_to_turbulent: 0.002,
            turbulent_to_calm: 0.01,
            turbulent_rate_mult: 3.0,
            turbulent_size_mult: 2.0,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NewsCfg {
    pub market: LayerCfg,
    pub sector: LayerCfg,
    pub stock: LayerCfg,
    pub disclosure: DisclosureCfg,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LayerCfg {
    /// Market layer: per market. Sector layer: per sector. Stock layer: per stock.
    pub events_per_1000_ticks: f64,
    pub change_sd: f64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DisclosureCfg {
    pub lead_ticks: Tick,
    pub stages: Vec<(Tick, f64)>,
    /// Range width per private stage (every stage but the last), in standard deviations of
    /// the change. 0 means exact.
    #[serde(default = "default_range_widths")]
    pub range_widths: Vec<f64>,
}
fn default_range_widths() -> Vec<f64> {
    vec![3.0, 1.5]
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StockCfg {
    pub name: String,
    pub sector: String,
    pub dividend: Money,
    pub initial_price: Price,
    #[serde(default = "one")]
    pub market_loading: f64,
    #[serde(default = "one")]
    pub sector_loading: f64,
}
fn one() -> f64 {
    1.0
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComboCfg {
    pub a: String,
    #[serde(default = "one_qty")]
    pub a_units: Qty,
    pub b: String,
    #[serde(default = "one_qty")]
    pub b_units: Qty,
    pub bonus: Money,
    #[serde(default)]
    pub start_tick: Tick,
    pub end_tick: Option<Tick>,
}
fn one_qty() -> Qty {
    1
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct ScoringCfg {
    pub terminal_ratio: f64,
    /// Range the number of terminal payments is drawn from. Default: fixed at
    /// `terminal_ratio * ticks / terminal_payment_ticks`.
    pub terminal_payments: Option<(i64, i64)>,
    /// Random terminal duration as fractions of the game length, rounded to units.
    /// Mutually exclusive with a fixed terminal_payments range.
    pub terminal_ratio_range: Option<(f64, f64)>,
    /// Length of each hidden terminal-payment unit. Defaults to the ordinary period.
    pub terminal_payment_ticks: Option<Tick>,
    /// (tick, width) of the private ranges for the number of terminal payments.
    pub terminal_hints: Vec<(Tick, i64)>,
    /// (game-time fraction, width as fraction of game duration). Resolved once.
    /// Mutually exclusive with absolute terminal_hints.
    pub terminal_hint_fractions: Option<Vec<(f64, f64)>>,
    pub objective: String,
    pub series_seeds: u32,
    /// Hidden game length: `ticks` becomes the longest possible game. See `RandomEndRules`.
    pub random_end: Option<RandomEndRules>,
    /// Endowment of a `challenge` account when the CLI gives none. Defaults 20,000 yuan
    /// and 20 shares of each stock.
    pub challenger_cash: Option<Money>,
    pub challenger_shares: Option<Qty>,
}
impl Default for ScoringCfg {
    fn default() -> Self {
        Self {
            terminal_ratio: 0.5,
            terminal_payments: None,
            terminal_ratio_range: None,
            terminal_payment_ticks: None,
            terminal_hints: Vec::new(),
            terminal_hint_fractions: None,
            objective: "rank_percentile".into(),
            series_seeds: 20,
            random_end: None,
            challenger_cash: None,
            challenger_shares: None,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BotGroupCfg {
    /// Built-in bot name.
    pub bot: String,
    /// Who wrote the bot. Each owner has exactly one account; when omitted the account
    /// is named `{bot}#{i}` and is its own owner.
    pub owner: Option<String>,
    #[serde(default = "one_usize")]
    pub count: usize,
    /// When set, each world draws the number of accounts uniformly from count..=count_max.
    pub count_max: Option<usize>,
    pub cash: Money,
    /// Shares of every stock.
    #[serde(default)]
    pub shares_each: Qty,
    /// Per-stock overrides.
    #[serde(default)]
    pub shares: BTreeMap<String, Qty>,
    /// Multiply this account's cumulative private-news disclosure probabilities.
    /// Public disclosure, timing and range precision are unchanged.
    #[serde(default = "one_news_multiplier")]
    pub news_multiplier: f64,
    #[serde(default)]
    pub forecast_access: bot_api::ForecastAccess,
    /// Share recent realized/marked P&L only with accounts in this group.
    #[serde(default)]
    pub social_group: Option<String>,
    #[serde(default)]
    pub params: BTreeMap<String, f64>,
}
fn one_usize() -> usize {
    1
}
fn one_news_multiplier() -> f64 {
    1.0
}

pub const MAX_CASH: Money = 1_000_000_000_000; // 10 billion in cents per account
pub const MAX_PRICE: Price = 100_000_000; // 1 million per share
pub const MAX_SHARES: Qty = 100_000_000;

impl Config {
    pub fn dragon_start_tick(&self) -> Tick {
        if self.world.dragon.start_fraction == 0.55 {
            // Keep historical integer rounding byte-for-byte reproducible.
            (self.ticks as u64 * 55 / 100) as Tick
        } else {
            (self.world_duration() * self.world.dragon.start_fraction).floor() as Tick
        }
    }

    /// Time scale of the world's business phases and growth: the game length, or its
    /// expectation under a random end, so the hidden end is not written into the world.
    pub fn world_duration(&self) -> f64 {
        self.expected_ticks()
    }

    pub fn load(path: &Path) -> Result<Config> {
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let cfg: Config =
            toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
        cfg.validate()?;
        Ok(cfg)
    }

    pub fn stock_index(&self, name: &str) -> Result<usize> {
        self.stocks
            .iter()
            .position(|s| s.name == name)
            .with_context(|| format!("unknown stock {name}"))
    }

    /// Reserve each anonymous group's maximum population so its names stay
    /// stable across worlds even when an earlier group's actual count varies.
    pub(crate) fn auto_owner_offsets(&self) -> Result<Vec<usize>> {
        let mut next = BTreeMap::<&str, usize>::new();
        self.bots
            .iter()
            .map(|g| {
                if g.owner.is_some() {
                    return Ok(0);
                }
                let slot = next.entry(&g.bot).or_default();
                let base = *slot;
                *slot = slot
                    .checked_add(g.count_max.unwrap_or(g.count))
                    .context("bot population overflow")?;
                Ok(base)
            })
            .collect()
    }

    pub fn validate(&self) -> Result<()> {
        if self.ticks == 0
            || self.ticks == Tick::MAX
            || self.stocks.is_empty()
            || self.bots.is_empty()
        {
            bail!("ticks, stocks and bots must be non-empty");
        }
        if !(1..=MAX_PRICE).contains(&self.market.tick_size) {
            bail!("market.tick_size must be in 1..={MAX_PRICE}");
        }
        if self
            .market
            .opening_spread
            .is_some_and(|x| !x.is_finite() || !(0.0..=1.0).contains(&x))
        {
            bail!("market.opening_spread must be finite and in 0..=1");
        }
        if !(0..=10_000).contains(&self.fees.trade_fee_bps) {
            bail!("fees.trade_fee_bps must be in 0..=10000");
        }
        for (name, value) in [
            ("fees.idle_rate", self.fees.idle_rate),
            ("fees.credit_ratio", self.fees.credit_ratio),
            ("margin.loan_rate", self.margin.loan_rate),
            (
                "world.seasons.turbulent_rate_mult",
                self.world.seasons.turbulent_rate_mult,
            ),
            (
                "world.seasons.turbulent_size_mult",
                self.world.seasons.turbulent_size_mult,
            ),
        ] {
            if !value.is_finite() || value < 0.0 {
                bail!("{name} must be finite and non-negative");
            }
        }
        if !matches!(self.fees.credit_side.as_str(), "both" | "maker" | "taker") {
            bail!("fees.credit_side must be both, maker or taker");
        }
        if !self.margin.max_leverage.is_finite()
            || !self.margin.maintenance_leverage.is_finite()
            || self.margin.max_leverage < 1.0
            || self.margin.maintenance_leverage < self.margin.max_leverage
        {
            bail!("margin: need 1 <= max_leverage <= maintenance_leverage");
        }
        if self.world.dividend_period == 0
            || self.combo_period() == 0
            || self.terminal_payment_ticks() == 0
            || self.fees.idle_period == 0
            || self.margin.interest_period == 0
            || self.margin.mark_window == 0
            || self.schedule.max_tick_skew == 0
        {
            bail!("periods must be positive");
        }
        if self.world.dividend_scale <= 0
            || self.world.combo_scale <= 0
            || !self
                .terminal_payment_ticks()
                .is_multiple_of(self.world.dividend_period)
            || !self
                .terminal_payment_ticks()
                .is_multiple_of(self.combo_period())
        {
            bail!("dividend_scale and combo_scale must be positive; terminal_payment_ticks must be a multiple of ordinary and combo periods");
        }
        for p in [
            self.world.seasons.calm_to_turbulent,
            self.world.seasons.turbulent_to_calm,
        ] {
            if !p.is_finite() || !(0.0..=1.0).contains(&p) {
                bail!("season transition probabilities must be finite and in 0..=1");
            }
        }
        for layer in [
            &self.world.news.market,
            &self.world.news.sector,
            &self.world.news.stock,
        ] {
            if [layer.events_per_1000_ticks, layer.change_sd]
                .iter()
                .any(|x| !x.is_finite() || *x < 0.0)
            {
                bail!("news frequency and change_sd must be finite and non-negative");
            }
        }
        let st = &self.world.news.disclosure.stages;
        let dragon = &self.world.dragon;
        let trend = &dragon.trend;
        if !dragon.start_fraction.is_finite()
            || !(0.2..=0.65).contains(&dragon.start_fraction)
            || (dragon.model == DragonModel::Pulses && dragon.start_fraction != 0.55)
        {
            bail!("world.dragon.start_fraction must be finite in 0.2..=0.65; legacy pulses require 0.55");
        }
        if !trend.events_per_1000_ticks.is_finite()
            || !(1.0..=100.0).contains(&trend.events_per_1000_ticks)
            || !trend.growth_log_bias.0.is_finite()
            || !trend.growth_log_bias.1.is_finite()
            || !(0.0..=3.0).contains(&trend.growth_log_bias.0)
            || trend.growth_log_bias.1 < trend.growth_log_bias.0
            || trend.growth_log_bias.1 > 3.0
            || !trend.step_sd.is_finite()
            || !(0.0..=0.1).contains(&trend.step_sd)
            || !trend.max_step.is_finite()
            || !(0.005..=if dragon.model == DragonModel::Growth {
                0.16
            } else {
                0.1
            })
                .contains(&trend.max_step)
        {
            bail!("world.dragon.trend: finite rate in 1..=100, 0 <= growth_log_bias min <= max <= 3, step_sd in 0..=0.1, max_step in 0.005..=0.1 (growth: <=0.16)");
        }
        if !dragon.probability.is_finite()
            || !(0.0..=1.0).contains(&dragon.probability)
            || !dragon.peak_extra.0.is_finite()
            || !dragon.peak_extra.1.is_finite()
            || dragon.peak_extra.0 < 1.0
            || dragon.peak_extra.1 < dragon.peak_extra.0
            || dragon.peak_extra.1 > 10.0
            || (dragon.enabled
                && (dragon.candidates < 2
                    || dragon.candidates > self.stocks.len()
                    || self.ticks as u64 <= self.world.news.disclosure.lead_ticks as u64 * 3 + 30))
        {
            bail!("world.dragon: finite probability in 0..=1, 1 <= peak_extra min <= max <= 10; when enabled need 2..=stocks candidates and ticks > 3*news lead + 30");
        }
        if st.is_empty()
            || st.last().unwrap().1 != 1.0
            || st.last().unwrap().0 > self.world.news.disclosure.lead_ticks
            || st
                .iter()
                .any(|(_, f)| !f.is_finite() || !(0.0..=1.0).contains(f))
            || st.windows(2).any(|w| w[0].0 > w[1].0 || w[0].1 > w[1].1)
            || self
                .ticks
                .checked_add(self.world.news.disclosure.lead_ticks)
                .is_none()
        {
            bail!("news.disclosure.stages must end with fraction 1.0 no later than lead_ticks");
        }
        if !self.scoring.terminal_ratio.is_finite()
            || self.scoring.terminal_ratio < 0.0
            || self.scoring.terminal_ratio * self.ticks as f64 > Tick::MAX as f64
        {
            bail!("scoring.terminal_ratio must produce a finite, non-negative tick count");
        }
        if let Some((lo, hi)) = self.scoring.terminal_ratio_range {
            if self.scoring.terminal_payments.is_some()
                || !lo.is_finite()
                || !hi.is_finite()
                || lo < 0.0
                || hi < lo
                || hi * self.ticks as f64 > Tick::MAX as f64
            {
                bail!("terminal_ratio_range requires finite 0 <= min <= max and cannot coexist with terminal_payments");
            }
        }
        if let Some(hints) = &self.scoring.terminal_hint_fractions {
            if !self.scoring.terminal_hints.is_empty()
                || hints.iter().any(|&(at, width)| {
                    !at.is_finite()
                        || !(0.0..1.0).contains(&at)
                        || !width.is_finite()
                        || width < 0.0
                        || width * self.ticks as f64 > Tick::MAX as f64
                })
                || hints.windows(2).any(|w| w[0].0 > w[1].0)
            {
                bail!("terminal_hint_fractions requires ordered times in [0,1), finite nonnegative widths, and no absolute terminal_hints");
            }
        }
        if let Some(e) = self.scoring.random_end {
            if e.min_tick == 0
                || e.min_tick >= self.ticks
                || !e.hazard_per_tick.is_finite()
                || e.hazard_per_tick <= 0.0
                || e.hazard_per_tick >= 1.0
            {
                bail!("scoring.random_end needs 0 < min_tick < ticks and 0 < hazard_per_tick < 1");
            }
        }
        let (lo, hi) = self.terminal_payments_range();
        if lo < 0
            || hi < lo
            || hi > i64::MAX / 2
            || self
                .scoring
                .terminal_hints
                .iter()
                .any(|h| h.1 < 0 || h.1 > i64::MAX / 2)
            || self
                .scoring
                .terminal_hints
                .windows(2)
                .any(|w| w[0].0 > w[1].0)
        {
            bail!("scoring.terminal_payments must be 0 <= min <= max; hint widths >= 0");
        }
        let d = &self.world.news.disclosure;
        if d.range_widths.len() != d.stages.len() - 1
            || d.range_widths.iter().any(|w| !w.is_finite() || *w < 0.0)
        {
            bail!("news.disclosure.range_widths needs one non-negative width per private stage");
        }
        let mut names = std::collections::HashSet::new();
        if self.world.min_dividend < 0 {
            bail!("world.min_dividend must be non-negative");
        }
        if let Some(r) = &self.world.dividend_risk {
            let range = |(lo, hi): (f64, f64)| {
                lo.is_finite() && hi.is_finite() && lo > 0.0 && lo <= hi && hi <= 1.0
            };
            if self.world.min_dividend != 0
                || !range(r.review_fraction)
                || !range(r.impaired_retention)
                || !range(r.recovered_retention)
                || r.impaired_retention.1 >= r.recovered_retention.0
                || !r.adjustment_fraction.is_finite()
                || r.adjustment_fraction <= 0.0
                || r.adjustment_fraction >= r.review_fraction.0
                || r.adjustment_fraction * (self.ticks as f64) < 4.0
                || self.ticks <= self.world.news.disclosure.lead_ticks
                || [
                    r.impairment_probability,
                    r.suspension_probability,
                    r.recovery_probability,
                ]
                .iter()
                .any(|p| !p.is_finite() || !(0.0..=1.0).contains(p))
                || r.impairment_probability + r.suspension_probability > 1.0
                || r.recovery_probability + r.suspension_probability > 1.0
            {
                bail!("world.dividend_risk requires min_dividend=0, ordered retention ranges in (0,1], valid probabilities, 4 <= adjustment ticks < minimum review ticks, and ticks > news lead");
            }
        }
        for s in &self.stocks {
            if s.name.is_empty() || !names.insert(&s.name) {
                bail!("stock names must be non-empty and unique");
            }
            if !s.market_loading.is_finite() || !s.sector_loading.is_finite() {
                bail!("stock loadings must be finite");
            }
            if !self.world.sectors.contains(&s.sector) {
                bail!("stock {} has unknown sector {}", s.name, s.sector);
            }
            if s.initial_price <= 0 || s.initial_price > MAX_PRICE || s.dividend < 0 {
                bail!(
                    "stock {}: price must be in 1..={MAX_PRICE}, dividend >= 0",
                    s.name
                );
            }
        }
        for c in &self.combos {
            self.stock_index(&c.a)?;
            self.stock_index(&c.b)?;
            if c.a_units <= 0 || c.b_units <= 0 {
                bail!("combo units must be positive");
            }
        }
        let mut owners = std::collections::HashSet::new();
        for g in &self.bots {
            if g.social_group
                .as_ref()
                .is_some_and(|s| s.trim().is_empty() || s.len() > 64)
            {
                bail!("social_group must be a nonempty name of at most 64 bytes");
            }
            let f = g.forecast_access;
            if !f.coverage.is_finite()
                || !(0.0..=1.0).contains(&f.coverage)
                || !f.range_width.is_finite()
                || !(0.0..=100.0).contains(&f.range_width)
                || f.horizon > 100_000
            {
                bail!("forecast_access requires horizon <= 100000, coverage in 0..=1, width in 0..=100");
            }
            if !g.news_multiplier.is_finite() || g.news_multiplier < 0.0 {
                bail!(
                    "bot group {}: news_multiplier must be finite and non-negative",
                    g.bot
                );
            }
            if let Some(o) = &g.owner {
                if g.count != 1 || g.count_max.is_some() || !owners.insert(o.clone()) {
                    bail!("owner {o}: each owner has exactly one account (count = 1, owner names unique)");
                }
            }
            if g.count_max.is_some_and(|m| m < g.count) {
                bail!("bot group {}: count_max below count", g.bot);
            }
            if g.cash < 0 || g.cash > MAX_CASH || g.shares_each < 0 || g.shares_each > MAX_SHARES {
                bail!("bot group {}: cash or shares out of range", g.bot);
            }
            for (name, q) in &g.shares {
                self.stock_index(name)?;
                if *q < 0 || *q > MAX_SHARES {
                    bail!("bot group {}: shares out of range", g.bot);
                }
            }
        }
        let offsets = self.auto_owner_offsets()?;
        let mut max_accounts = 0usize;
        for (g, base) in self.bots.iter().zip(offsets) {
            let count = g.count_max.unwrap_or(g.count);
            max_accounts = max_accounts
                .checked_add(count)
                .context("bot population overflow")?;
            if g.owner.is_none() {
                for owner in &owners {
                    if let Some((bot, suffix)) = owner.rsplit_once('#') {
                        if let Ok(i) = suffix.parse::<usize>() {
                            if bot == g.bot
                                && suffix == i.to_string()
                                && (base..base + count).contains(&i)
                            {
                                bail!("owner {owner} conflicts with an automatically generated account");
                            }
                        }
                    }
                }
            }
        }
        if max_accounts == 0 || max_accounts > u32::MAX as usize {
            bail!("total bot population must be in 1..=u32::MAX");
        }
        crate::capital::validate(self)?;
        Ok(())
    }

    pub fn terminal_payments_range(&self) -> (i64, i64) {
        if let Some((lo, hi)) = self.scoring.terminal_ratio_range {
            let units = self.ticks as f64 / self.terminal_payment_ticks() as f64;
            return ((lo * units).round() as i64, (hi * units).round() as i64);
        }
        self.scoring.terminal_payments.unwrap_or_else(|| {
            let n = (self.terminal_ticks() / self.terminal_payment_ticks()) as i64;
            (n, n)
        })
    }

    pub fn terminal_hints(&self) -> Vec<(Tick, i64)> {
        match &self.scoring.terminal_hint_fractions {
            Some(hints) => hints
                .iter()
                .map(|&(at, width)| {
                    (
                        (at * self.ticks as f64).floor() as Tick,
                        (width * self.ticks as f64 / self.terminal_payment_ticks() as f64).round()
                            as i64,
                    )
                })
                .collect(),
            None => self.scoring.terminal_hints.clone(),
        }
    }

    pub fn combo_period(&self) -> Tick {
        self.world
            .combo_period
            .unwrap_or(self.world.dividend_period)
    }

    pub fn terminal_payment_ticks(&self) -> Tick {
        self.scoring
            .terminal_payment_ticks
            .unwrap_or(self.world.dividend_period)
    }

    /// Number of ticks this world's game actually runs. With `random_end` it is hidden
    /// from bots and drawn from the seed alone, independent of news and accounts.
    pub fn end_tick(&self, seed: u64) -> Tick {
        let Some(e) = self.scoring.random_end else {
            return self.ticks;
        };
        let u = crate::world::unit(seed ^ 0xe7d_0f_6a3e, 0, 0).max(f64::MIN_POSITIVE);
        // ticks after min_tick until the end draw succeeds: geometric with this hazard
        let extra = (u.ln() / (1.0 - e.hazard_per_tick).ln()).floor();
        let end = e.min_tick as f64 + 1.0 + extra;
        if end >= self.ticks as f64 {
            self.ticks
        } else {
            end as Tick
        }
    }

    /// Expected number of ticks the game runs, as bots can compute it from the rules.
    pub fn expected_ticks(&self) -> f64 {
        let Some(e) = self.scoring.random_end else {
            return self.ticks as f64;
        };
        let q = 1.0 - e.hazard_per_tick;
        let span = (self.ticks - e.min_tick) as f64;
        // min_tick + sum over k in 1..=span of P(the game reaches k more ticks)
        e.min_tick as f64 + (1.0 - q.powf(span)) / e.hazard_per_tick
    }

    pub fn terminal_ticks(&self) -> Tick {
        (self.scoring.terminal_ratio * self.ticks as f64).ceil() as Tick
    }

    pub fn rules(&self, shares_outstanding: &[Qty]) -> Rules {
        let sector_of = |name: &str| self.world.sectors.iter().position(|s| s == name).unwrap();
        let terminal_ticks = self.terminal_ticks();
        let layer = |l: &LayerCfg| NewsLayerRules {
            events_per_1000_ticks: l.events_per_1000_ticks,
            change_sd: l.change_sd,
        };
        Rules {
            stocks: self
                .stocks
                .iter()
                .zip(shares_outstanding)
                .map(|(s, &n)| StockInfo {
                    name: s.name.clone(),
                    sector: sector_of(&s.sector),
                    shares_outstanding: n,
                    initial_dividend: s.dividend,
                    initial_price: s.initial_price,
                    market_loading: s.market_loading,
                    sector_loading: s.sector_loading,
                })
                .collect(),
            sectors: self.world.sectors.clone(),
            combos: self
                .combos
                .iter()
                .map(|c| ComboRule {
                    a: self.stock_index(&c.a).unwrap(),
                    a_units: c.a_units,
                    b: self.stock_index(&c.b).unwrap(),
                    b_units: c.b_units,
                    bonus: c.bonus,
                    start_tick: c.start_tick,
                    end_tick: c.end_tick.unwrap_or(Tick::MAX),
                })
                .collect(),
            ticks_total: self.ticks,
            tick_size: self.market.tick_size,
            dividend_period: self.world.dividend_period,
            dividend_scale: self.world.dividend_scale,
            combo_period: self.combo_period(),
            combo_scale: self.world.combo_scale,
            terminal_payment_ticks: self.terminal_payment_ticks(),
            min_dividend: self.world.min_dividend,
            trade_fee_bps: self.fees.trade_fee_bps,
            idle_period: self.fees.idle_period,
            idle_rate: self.fees.idle_rate,
            credit_ratio: self.fees.credit_ratio,
            credit_side: self.fees.credit_side.clone(),
            max_leverage: self.margin.max_leverage,
            maintenance_leverage: self.margin.maintenance_leverage,
            loan_rate: self.margin.loan_rate,
            interest_period: self.margin.interest_period,
            mark_window: self.margin.mark_window,
            max_actions_per_tick: self.rules.max_actions_per_tick,
            max_open_orders: self.rules.max_open_orders,
            book_depth: self.market.book_depth,
            news_market: layer(&self.world.news.market),
            news_sector: layer(&self.world.news.sector),
            news_stock: layer(&self.world.news.stock),
            news_lead_ticks: self.world.news.disclosure.lead_ticks,
            news_stages: self.world.news.disclosure.stages.clone(),
            news_range_widths: self.world.news.disclosure.range_widths.clone(),
            dragon: self.world.dragon.enabled.then_some(DragonRules {
                model: self.world.dragon.model,
                trend: (self.world.dragon.model != DragonModel::Pulses)
                    .then(|| self.world.dragon.trend.clone()),
                probability: self.world.dragon.probability,
                candidates: self.world.dragon.candidates,
                divergence_tick: self.dragon_start_tick(),
                peak_extra: self.world.dragon.peak_extra,
            }),
            dividend_risk: self.world.dividend_risk.clone(),
            random_end: self.scoring.random_end,
            season_calm_to_turbulent: self.world.seasons.calm_to_turbulent,
            season_turbulent_to_calm: self.world.seasons.turbulent_to_calm,
            season_turbulent_rate_mult: self.world.seasons.turbulent_rate_mult,
            season_turbulent_size_mult: self.world.seasons.turbulent_size_mult,
            max_tick_skew: self.schedule.max_tick_skew,
            skip_decision_lag: self.schedule.skip_decision_lag,
            decision_budget_us: self.schedule.decision_budget_us,
            terminal_ticks,
            terminal_payments_min: self.terminal_payments_range().0,
            terminal_payments_max: self.terminal_payments_range().1,
            terminal_hints: self.terminal_hints(),
            objective: self.scoring.objective.clone(),
        }
    }
}
