//! Per-account state. Each account is written only by the worker currently
//! stepping it; fills on its resting orders arrive through its mailbox.

use bot_api::{Ack, Bot, Fill, Income, Money, NewsItem, OrderId, Price, Qty, Reject, Side, Tick};
use std::collections::HashMap;

#[derive(Clone, Debug)]
pub struct OwnOrder {
    pub stock: usize,
    pub side: Side,
    pub price: Price,
    /// Remaining as this account knows it; mailbox fills may still be pending.
    pub remaining: Qty,
    pub reserve_per_share: i64,
    pub placed_tick: Tick,
    /// Still on the book (false once cancelled, while fills may still be in the mailbox).
    pub live: bool,
}

/// A fill on a resting order, delivered to the maker's mailbox.
#[derive(Clone, Copy, Debug)]
pub struct Mail {
    pub order_id: OrderId,
    pub stock: usize,
    pub side: Side,
    pub price: Price,
    pub qty: Qty,
    pub fee: Money,
    pub tick: Tick,
    pub reserve_per_share: i64,
}

impl Mail {
    pub fn cash_effect(&self) -> Money {
        let notional = self.price * self.qty;
        match self.side {
            Side::Buy => -(notional + self.fee),
            Side::Sell => notional - self.fee,
        }
    }
    pub fn share_effect(&self) -> Qty {
        match self.side {
            Side::Buy => self.qty,
            Side::Sell => -self.qty,
        }
    }
}

#[derive(Default)]
pub struct Inbox {
    pub fills: Vec<Fill>,
    pub acks: Vec<Ack>,
    pub rejects: Vec<Reject>,
    pub news: Vec<NewsItem>,
    pub season_changes: Vec<(Tick, bool)>,
    pub income: Vec<Income>,
    pub terminal_hints: Vec<(i64, i64)>,
}

#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct Stats {
    pub trade_fees: Money,
    pub idle_fees: Money,
    pub interest: Money,
    pub dividends: Money,
    pub combo_dividends: Money,
    pub volume_notional: Money,
    pub credited_notional: Money,
    pub trades: u64,
    pub orders: u64,
    pub rejects: u64,
    pub skipped: u64,
    pub decisions: u64,
    pub bot_errors: u64,
    pub liquidations: u64,
    pub bad_debt: Money,
    pub burned_shares: Qty,
    pub max_leverage: f64,
    pub decision_nanos_total: u64,
    pub decision_nanos_max: u64,
}

pub struct Account {
    pub idx: usize,
    pub name: String,
    pub owner: String,
    pub strategy: String,
    pub cash: Money,
    pub shares: Vec<Qty>,
    pub reserved_sell: Vec<Qty>,
    pub reserved_buy: Money,
    pub orders: HashMap<OrderId, OwnOrder>,
    pub equity_sum: i128,
    pub loan_sum: i128,
    pub samples: i64,
    pub loan_samples: i64,
    pub credit_notional: Money,
    /// Fractional shares owed to the idle fee, in millionths of a share.
    pub share_frac: Vec<i64>,
    pub inbox: Inbox,
    pub tape_seen: Vec<usize>,
    pub delivery_cursor: usize,
    pub season_cursor: usize,
    pub hint_cursor: usize,
    pub skipped_since: u32,
    pub consecutive_errors: u32,
    pub stats: Stats,
    pub bot: Option<Box<dyn Bot>>,
    pub disabled: Option<String>,
    pub initial_cash: Money,
    pub initial_shares: Vec<Qty>,
}

impl Account {
    pub fn new(
        idx: usize,
        name: String,
        owner: String,
        strategy: String,
        cash: Money,
        shares: Vec<Qty>,
    ) -> Self {
        let n = shares.len();
        Account {
            idx,
            name,
            owner,
            strategy,
            cash,
            initial_cash: cash,
            initial_shares: shares.clone(),
            shares,
            reserved_sell: vec![0; n],
            reserved_buy: 0,
            orders: HashMap::new(),
            equity_sum: 0,
            loan_sum: 0,
            samples: 0,
            loan_samples: 0,
            credit_notional: 0,
            share_frac: vec![0; n],
            inbox: Inbox::default(),
            tape_seen: vec![0; n],
            delivery_cursor: 0,
            season_cursor: 0,
            hint_cursor: 0,
            skipped_since: 0,
            consecutive_errors: 0,
            stats: Stats::default(),
            bot: None,
            disabled: None,
        }
    }

    pub fn sellable(&self, stock: usize) -> Qty {
        self.shares[stock] - self.reserved_sell[stock]
    }

    pub fn gross(&self, marks: &[Price]) -> Money {
        self.shares.iter().zip(marks).map(|(s, m)| s * m).sum()
    }

    pub fn equity(&self, marks: &[Price]) -> Money {
        self.cash + self.gross(marks)
    }

    pub fn live_orders(&self) -> usize {
        self.orders.values().filter(|o| o.live).count()
    }

    /// Lowest live sell and highest live buy on `stock`.
    pub fn own_touch(&self, stock: usize) -> (Option<Price>, Option<Price>) {
        let mut min_sell = None;
        let mut max_buy = None;
        for o in self.orders.values().filter(|o| o.live && o.stock == stock) {
            match o.side {
                Side::Sell => min_sell = Some(min_sell.map_or(o.price, |p: Price| p.min(o.price))),
                Side::Buy => max_buy = Some(max_buy.map_or(o.price, |p: Price| p.max(o.price))),
            }
        }
        (min_sell, max_buy)
    }
}
