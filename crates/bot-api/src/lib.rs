//! Types shared by the market engine and bots.
//!
//! Money and prices are integer cents, quantities integer shares. Dividend
//! rates and news changes use `Rules::dividend_scale` units per cent. Stocks are
//! referred to by index into `Rules::stocks`.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub type Money = i64;
pub type Price = i64;
pub type Qty = i64;
pub type OrderId = u64;
pub type Tick = u32;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum Side {
    #[default]
    Buy,
    Sell,
}

impl Side {
    pub fn opposite(self) -> Side {
        match self {
            Side::Buy => Side::Sell,
            Side::Sell => Side::Buy,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Tif {
    /// Rests on the book until filled or cancelled.
    Gtc,
    /// Fills what it can immediately; the rest is cancelled.
    Ioc,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Action {
    Limit {
        stock: usize,
        side: Side,
        price: Price,
        qty: Qty,
        tif: Tif,
        cid: u64,
    },
    /// Unpriced IOC. Buys are capped by buying power.
    Market {
        stock: usize,
        side: Side,
        qty: Qty,
        cid: u64,
    },
    Cancel {
        order_id: OrderId,
    },
    CancelAll {
        stock: Option<usize>,
    },
}

impl Action {
    pub fn limit(stock: usize, side: Side, price: Price, qty: Qty) -> Action {
        Action::Limit {
            stock,
            side,
            price,
            qty,
            tif: Tif::Gtc,
            cid: 0,
        }
    }
    pub fn ioc(stock: usize, side: Side, price: Price, qty: Qty) -> Action {
        Action::Limit {
            stock,
            side,
            price,
            qty,
            tif: Tif::Ioc,
            cid: 0,
        }
    }
    pub fn market(stock: usize, side: Side, qty: Qty) -> Action {
        Action::Market {
            stock,
            side,
            qty,
            cid: 0,
        }
    }
}

// ---- rules -------------------------------------------------------------

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StockInfo {
    pub name: String,
    pub sector: usize,
    pub shares_outstanding: Qty,
    /// Rate units per ordinary payment (one tick in per-tick scenarios).
    pub initial_dividend: Money,
    pub initial_price: Price,
    pub market_loading: f64,
    pub sector_loading: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ComboRule {
    pub a: usize,
    pub a_units: Qty,
    pub b: usize,
    pub b_units: Qty,
    /// Rate units per complete set per combo period; divide by Rules::combo_scale
    /// for cents. Negative values are charged.
    pub bonus: Money,
    pub start_tick: Tick,
    pub end_tick: Tick,
}

impl ComboRule {
    pub fn sets(&self, shares: &[Qty]) -> Qty {
        (shares[self.a] / self.a_units)
            .min(shares[self.b] / self.b_units)
            .max(0)
    }
    pub fn active(&self, tick: Tick) -> bool {
        tick >= self.start_tick && tick <= self.end_tick
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NewsLayerRules {
    pub events_per_1000_ticks: f64,
    pub change_sd: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Rules {
    pub stocks: Vec<StockInfo>,
    pub sectors: Vec<String>,
    pub combos: Vec<ComboRule>,
    pub ticks_total: Tick,
    pub tick_size: Price,
    pub dividend_period: Tick,
    /// Ordinary dividend/news rate units per cent. No per-account remainder is kept.
    pub dividend_scale: i64,
    pub combo_period: Tick,
    /// Combo rate units per cent. Round each account/rule total, without carry.
    #[serde(default = "default_combo_scale")]
    pub combo_scale: i64,
    /// Duration of one terminal-payment unit used by the hidden range and hints.
    pub terminal_payment_ticks: Tick,
    pub min_dividend: Money,
    pub trade_fee_bps: i64,
    pub idle_period: Tick,
    pub idle_rate: f64,
    pub credit_ratio: f64,
    /// "both" | "maker" | "taker"
    pub credit_side: String,
    pub max_leverage: f64,
    pub maintenance_leverage: f64,
    pub loan_rate: f64,
    pub interest_period: Tick,
    pub mark_window: Tick,
    pub max_actions_per_tick: usize,
    pub max_open_orders: usize,
    pub book_depth: usize,
    pub news_market: NewsLayerRules,
    pub news_sector: NewsLayerRules,
    pub news_stock: NewsLayerRules,
    pub news_lead_ticks: Tick,
    /// [offset from first reveal, cumulative fraction informed]
    pub news_stages: Vec<(Tick, f64)>,
    /// Range width at each private stage, in standard deviations of that stock's change
    /// for this layer and season: `max(dividend, initial / 2) * loading * change_sd * size_mult`.
    /// The true change sits at a uniformly random position inside each account's range.
    pub news_range_widths: Vec<f64>,
    pub season_calm_to_turbulent: f64,
    pub season_turbulent_to_calm: f64,
    pub season_turbulent_rate_mult: f64,
    pub season_turbulent_size_mult: f64,
    pub max_tick_skew: Tick,
    pub skip_decision_lag: Tick,
    /// A lagging bot is skipped only when six of its last eight calls exceed this
    /// many microseconds. 0 selects the legacy lag-only policy. Not a hard timeout.
    pub decision_budget_us: u64,
    /// Liquidation period length in ticks after trading ends.
    pub terminal_ticks: Tick,
    /// The number of terminal-payment units (each `terminal_payment_ticks` long) is drawn uniformly from
    /// `terminal_payments_min..=terminal_payments_max` at the start and not announced.
    pub terminal_payments_min: i64,
    pub terminal_payments_max: i64,
    /// (tick, width): at each of these ticks every account privately receives its own range
    /// `[low, low + width]` that contains the true number of terminal payments, with the
    /// truth at a random position. Ranges are delivered in `Observation::terminal_hints`.
    pub terminal_hints: Vec<(Tick, i64)>,
    /// Always "rank_percentile": accounts are ranked by final equity.
    pub objective: String,
}

fn default_combo_scale() -> i64 {
    1
}

/// Round positive payouts and negative charges symmetrically, halves away from zero.
fn rounded_combo(amount: i128, scale: i64) -> Money {
    let magnitude = (amount.abs() + i128::from(scale / 2)) / i128::from(scale);
    (amount.signum() * magnitude) as Money
}

impl Rules {
    /// Cents per share per ordinary payment, before rounding an account's total.
    pub fn dividend_cents(&self, rate: Money) -> f64 {
        rate as f64 / self.dividend_scale as f64
    }

    /// Round the account's whole position once, half up. Ordinary rates are nonnegative.
    pub fn dividend_payment(&self, shares: Qty, rate: Money) -> Money {
        ((shares as i128 * rate as i128 + (self.dividend_scale / 2) as i128)
            / self.dividend_scale as i128) as Money
    }

    pub fn terminal_dividend_payments(&self, units: f64) -> f64 {
        units * (self.terminal_payment_ticks / self.dividend_period) as f64
    }

    pub fn combo_cents(&self, rate: Money) -> f64 {
        rate as f64 / self.combo_scale as f64
    }

    /// Round the entire position in this combo, not each individual set.
    pub fn combo_payment(&self, sets: Qty, rate: Money) -> Money {
        if sets == 0 || rate == 0 {
            return 0;
        }
        rounded_combo(i128::from(sets) * i128::from(rate), self.combo_scale)
    }

    pub fn terminal_combo_payments(&self, units: f64) -> f64 {
        units * (self.terminal_payment_ticks / self.combo_period) as f64
    }

    /// The final executed tick. Trading and world settlement use 0..ticks_total.
    pub fn last_tick(&self) -> Tick {
        self.ticks_total.saturating_sub(1)
    }

    pub fn trade_fee(&self, notional: Money) -> Money {
        ((notional as i128 * self.trade_fee_bps as i128 + 9_999) / 10_000) as Money
    }
    /// Cash reserved per share by a buy order at `price` (price plus worst-case fee).
    pub fn buy_reserve_per_share(&self, price: Price) -> Money {
        price + (price * self.trade_fee_bps + 9_999) / 10_000
    }
    /// Value at the end of the game of holding these shares with the given dividends and
    /// `n` terminal-payment units. Ordinary and combo cash flows use their own periods.
    pub fn terminal_value(&self, shares: &[Qty], dividends: &[Money], n: i64) -> Money {
        let payments = n as i128 * (self.terminal_payment_ticks / self.dividend_period) as i128;
        let mut v: Money = shares
            .iter()
            .zip(dividends)
            .map(|(s, d)| {
                ((i128::from(*s) * i128::from(*d) * payments + (self.dividend_scale / 2) as i128)
                    / self.dividend_scale as i128) as Money
            })
            .sum();
        for c in &self.combos {
            if c.active(self.last_tick()) {
                // Round terminal cash flow once after aggregating future periods.
                v += rounded_combo(
                    i128::from(c.sets(shares))
                        * i128::from(c.bonus)
                        * i128::from(n)
                        * i128::from(self.terminal_payment_ticks / self.combo_period),
                    self.combo_scale,
                );
            }
        }
        v
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Start {
    pub account: String,
    pub owner: String,
    pub account_index: usize,
    pub n_accounts: usize,
    /// Deterministic bot RNG seed, not a security boundary. Reviewed bots must
    /// not use its relation to the world seed to reconstruct hidden information.
    pub seed: u64,
    pub params: BTreeMap<String, f64>,
    pub cash: Money,
    pub shares: Vec<Qty>,
    pub rules: Rules,
}

impl Start {
    pub fn param(&self, key: &str, default: f64) -> f64 {
        self.params.get(key).copied().unwrap_or(default)
    }
}

// ---- observation -------------------------------------------------------

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
pub struct Level {
    pub price: Price,
    pub qty: Qty,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct BookView {
    /// Best first.
    pub bids: Vec<Level>,
    /// Best first.
    pub asks: Vec<Level>,
    /// Last trade price (initial price before any trade).
    pub last: Price,
    /// Mark price used for equity, leverage and the idle fee.
    pub mark: Price,
    /// Current dividend rate per share per ordinary payment, in dividend_scale units/cent.
    pub dividend: Money,
}

impl BookView {
    pub fn best_bid(&self) -> Option<Price> {
        self.bids.first().map(|l| l.price)
    }
    pub fn best_ask(&self) -> Option<Price> {
        self.asks.first().map(|l| l.price)
    }
    pub fn mid(&self) -> Option<Price> {
        Some((self.best_bid()? + self.best_ask()?) / 2)
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct Trade {
    pub tick: Tick,
    pub price: Price,
    pub qty: Qty,
    pub aggressor: Side,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct Fill {
    pub order_id: OrderId,
    pub stock: usize,
    pub side: Side,
    pub price: Price,
    pub qty: Qty,
    pub fee: Money,
    /// True when this order was resting (passive side, delivered via mailbox).
    pub maker: bool,
    pub tick: Tick,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AckStatus {
    Resting,
    PartialResting,
    Filled,
    PartialCancelled,
    NoFill,
    Cancelled,
    CancelledByLiquidation,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct Ack {
    pub cid: u64,
    pub order_id: OrderId,
    pub status: AckStatus,
    pub remaining: Qty,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RejectReason {
    TooManyActions,
    UnknownStock,
    BadQty,
    BadPrice,
    TooManyOpenOrders,
    InsufficientShares,
    InsufficientBuyingPower,
    SelfCross,
    UnknownOrder,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Reject {
    pub action: Action,
    pub reason: RejectReason,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum NewsLayer {
    Market,
    Sector,
    Stock,
}

/// What one account knows about a news item's effect on one stock: the change in dividend
/// rate per share is somewhere in `low..=high`, in dividend_scale units/cent.
/// Exact (`low == high`) once public; the rate changes only at effective_tick.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Change {
    pub stock: usize,
    pub low: Money,
    pub high: Money,
}

impl Change {
    pub fn mid(&self) -> f64 {
        (self.low + self.high) as f64 / 2.0
    }
}

/// A news item as delivered to one account. A privately informed account receives it again
/// at every later disclosure stage with a narrower range, and once more when it is public.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NewsItem {
    pub id: u32,
    pub layer: NewsLayer,
    pub sector: Option<usize>,
    pub changes: Vec<Change>,
    /// Disclosure stage this delivery belongs to (index into `Rules::news_stages`).
    pub stage: usize,
    pub first_reveal_tick: Tick,
    pub public_tick: Tick,
    pub effective_tick: Tick,
    pub public: bool,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct Season {
    pub turbulent: bool,
    pub rate_mult: f64,
    pub size_mult: f64,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub enum Income {
    Dividend {
        stock: usize,
        amount: Money,
    },
    Combo {
        rule: usize,
        amount: Money,
    },
    IdleFee {
        paid: Money,
        gross: Money,
        credit: Money,
    },
    Interest {
        amount: Money,
    },
    Liquidation,
    BadDebtWrittenOff {
        amount: Money,
    },
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct OpenOrder {
    pub order_id: OrderId,
    pub stock: usize,
    pub side: Side,
    pub price: Price,
    /// Remaining quantity as this account knows it (fills from the mailbox may be pending).
    pub remaining: Qty,
    pub placed_tick: Tick,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct IdleEstimate {
    pub period_end_tick: Tick,
    pub avg_equity: Money,
    pub fee: Money,
    pub credit: Money,
    pub due: Money,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AccountView {
    /// Negative cash is a loan.
    pub cash: Money,
    pub shares: Vec<Qty>,
    pub sellable: Vec<Qty>,
    pub equity: Money,
    pub gross: Money,
    pub leverage: f64,
    /// Remaining notional this account may add in buy orders.
    pub buying_power: Money,
    pub reserved_buy: Money,
    pub idle: IdleEstimate,
    pub interest_estimate: Money,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Observation {
    pub tick: Tick,
    pub ticks_total: Tick,
    pub reference_tick: Tick,
    pub season: Season,
    pub account: AccountView,
    pub open_orders: Vec<OpenOrder>,
    pub books: Vec<BookView>,
    /// Public trades per stock since the last decision (capped).
    pub trades: Vec<Vec<Trade>>,
    /// Ticks skipped since the last decision (because this bot fell behind).
    pub skipped: u32,
    pub fills: Vec<Fill>,
    pub acks: Vec<Ack>,
    pub rejects: Vec<Reject>,
    pub news: Vec<NewsItem>,
    pub season_changes: Vec<(Tick, bool)>,
    pub income: Vec<Income>,
    /// Private ranges (low, high) for the number of terminal payments received this tick.
    pub terminal_hints: Vec<(i64, i64)>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FinalResult {
    pub equity: Money,
    pub rank: usize,
    pub percentile: f64,
}

/// A trading strategy. Created by a factory from `Start`.
pub trait Bot: Send {
    fn on_tick(&mut self, obs: &Observation) -> Vec<Action>;
    fn on_end(&mut self, _result: &FinalResult) {}
}
