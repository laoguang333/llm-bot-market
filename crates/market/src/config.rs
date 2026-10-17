//! Scenario TOML.

use anyhow::{bail, Context, Result};
use bot_api::{ComboRule, Money, NewsLayerRules, Price, Qty, Rules, StockInfo, Tick};
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
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct MarketCfg {
    pub tick_size: Price,
    pub book_depth: usize,
    pub trade_tape_limit: usize,
}
impl Default for MarketCfg {
    fn default() -> Self {
        Self {
            tick_size: 1,
            book_depth: 10,
            trade_tape_limit: 200,
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
    /// 0 = number of CPUs.
    pub threads: usize,
    pub max_tick_skew: Tick,
    pub skip_decision_lag: Tick,
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
            check_interval: 500,
            mailbox_capacity: 100_000,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorldCfg {
    #[serde(default = "default_dividend_period")]
    pub dividend_period: Tick,
    #[serde(default)]
    pub min_dividend: Money,
    pub sectors: Vec<String>,
    #[serde(default)]
    pub seasons: SeasonsCfg,
    pub news: NewsCfg,
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
    /// `terminal_ratio * ticks / dividend_period`.
    pub terminal_payments: Option<(i64, i64)>,
    /// (tick, width) of the private ranges for the number of terminal payments.
    pub terminal_hints: Vec<(Tick, i64)>,
    pub objective: String,
    pub series_seeds: u32,
}
impl Default for ScoringCfg {
    fn default() -> Self {
        Self {
            terminal_ratio: 0.5,
            terminal_payments: None,
            terminal_hints: Vec::new(),
            objective: "rank_percentile".into(),
            series_seeds: 20,
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
    pub cash: Money,
    /// Shares of every stock.
    #[serde(default)]
    pub shares_each: Qty,
    /// Per-stock overrides.
    #[serde(default)]
    pub shares: BTreeMap<String, Qty>,
    #[serde(default)]
    pub params: BTreeMap<String, f64>,
}
fn one_usize() -> usize {
    1
}

pub const MAX_CASH: Money = 1_000_000_000_000; // 10 billion in cents per account
pub const MAX_PRICE: Price = 100_000_000; // 1 million per share
pub const MAX_SHARES: Qty = 100_000_000;

impl Config {
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

    pub fn validate(&self) -> Result<()> {
        if self.ticks == 0 || self.stocks.is_empty() || self.bots.is_empty() {
            bail!("ticks, stocks and bots must be non-empty");
        }
        if !matches!(self.fees.credit_side.as_str(), "both" | "maker" | "taker") {
            bail!("fees.credit_side must be both, maker or taker");
        }
        if self.margin.max_leverage < 1.0
            || self.margin.maintenance_leverage < self.margin.max_leverage
        {
            bail!("margin: need 1 <= max_leverage <= maintenance_leverage");
        }
        if self.world.dividend_period == 0
            || self.fees.idle_period == 0
            || self.margin.interest_period == 0
        {
            bail!("periods must be positive");
        }
        let st = &self.world.news.disclosure.stages;
        if st.is_empty()
            || st.last().unwrap().1 < 1.0
            || st.last().unwrap().0 > self.world.news.disclosure.lead_ticks
        {
            bail!("news.disclosure.stages must end with fraction 1.0 no later than lead_ticks");
        }
        let (lo, hi) = self.terminal_payments_range();
        if lo < 0 || hi < lo || self.scoring.terminal_hints.iter().any(|h| h.1 < 0) {
            bail!("scoring.terminal_payments must be 0 <= min <= max; hint widths >= 0");
        }
        let d = &self.world.news.disclosure;
        if d.range_widths.len() != d.stages.len() - 1 || d.range_widths.iter().any(|w| *w < 0.0) {
            bail!("news.disclosure.range_widths needs one non-negative width per private stage");
        }
        for s in &self.stocks {
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
            if let Some(o) = &g.owner {
                if g.count != 1 || !owners.insert(o.clone()) {
                    bail!("owner {o}: each owner has exactly one account (count = 1, owner names unique)");
                }
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
        Ok(())
    }

    pub fn terminal_payments_range(&self) -> (i64, i64) {
        self.scoring.terminal_payments.unwrap_or_else(|| {
            let n = (self.terminal_ticks() / self.world.dividend_period) as i64;
            (n, n)
        })
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
            season_calm_to_turbulent: self.world.seasons.calm_to_turbulent,
            season_turbulent_to_calm: self.world.seasons.turbulent_to_calm,
            season_turbulent_rate_mult: self.world.seasons.turbulent_rate_mult,
            season_turbulent_size_mult: self.world.seasons.turbulent_size_mult,
            max_tick_skew: self.schedule.max_tick_skew,
            skip_decision_lag: self.schedule.skip_decision_lag,
            terminal_ticks,
            terminal_payments_min: self.terminal_payments_range().0,
            terminal_payments_max: self.terminal_payments_range().1,
            terminal_hints: self.scoring.terminal_hints.clone(),
            objective: self.scoring.objective.clone(),
        }
    }
}
