//! Per-account state. Each account is written only by the worker currently
//! stepping it; fills on its resting orders arrive through its mailbox.

use bot_api::{Ack, Bot, Fill, Income, Money, NewsItem, OrderId, Price, Qty, Reject, Side, Tick};
use std::ops::Index;

#[derive(Clone, Debug, PartialEq, Eq)]
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

/// Compact account-local orders. IDs and payloads live in separate contiguous
/// arrays: lookups touch only keys, and hot scans skip keys and hash-table holes.
/// Normal order IDs increase, so insertion appends; general insertion still
/// preserves sorted iteration. Removing an entry moves the small bounded tail.
#[derive(Default)]
pub struct OrderStore {
    ids: Vec<OrderId>,
    orders: Vec<OwnOrder>,
}

impl OrderStore {
    pub fn len(&self) -> usize {
        self.ids.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    pub fn get(&self, id: &OrderId) -> Option<&OwnOrder> {
        self.ids.binary_search(id).ok().map(|i| &self.orders[i])
    }

    pub fn get_mut(&mut self, id: &OrderId) -> Option<&mut OwnOrder> {
        self.ids.binary_search(id).ok().map(|i| &mut self.orders[i])
    }

    pub fn insert(&mut self, id: OrderId, order: OwnOrder) -> Option<OwnOrder> {
        if self.ids.last().is_none_or(|&last| last < id) {
            self.ids.push(id);
            self.orders.push(order);
            return None;
        }
        match self.ids.binary_search(&id) {
            Ok(i) => Some(std::mem::replace(&mut self.orders[i], order)),
            Err(i) => {
                self.ids.insert(i, id);
                self.orders.insert(i, order);
                None
            }
        }
    }

    pub fn remove(&mut self, id: &OrderId) -> Option<OwnOrder> {
        let i = self.ids.binary_search(id).ok()?;
        self.ids.remove(i);
        Some(self.orders.remove(i))
    }

    pub fn values(&self) -> std::slice::Iter<'_, OwnOrder> {
        self.orders.iter()
    }

    pub fn iter(&self) -> impl DoubleEndedIterator<Item = (&OrderId, &OwnOrder)> {
        self.ids.iter().zip(&self.orders)
    }
}

impl Index<&OrderId> for OrderStore {
    type Output = OwnOrder;
    fn index(&self, id: &OrderId) -> &Self::Output {
        self.get(id).expect("account order exists")
    }
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
    /// Idle fee split: cash sent to treasury vs shares destroyed at mark value.
    pub idle_cash_fees: Money,
    pub idle_stock_fees: Money,
    pub interest: Money,
    pub dividends: Money,
    pub combo_dividends: Money,
    pub volume_notional: Money,
    pub credited_notional: Money,
    pub trades: u64,
    pub orders: u64,
    pub rejects: u64,
    pub skipped: u64,
    pub over_budget_decisions: u64,
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
    pub orders: OrderStore,
    pub equity_sum: i128,
    pub loan_sum: i128,
    pub samples: i64,
    pub loan_samples: i64,
    pub credit_notional: Money,
    /// Idle cash charge blocked by an already executed, unsettled buy. It is
    /// reapportioned over available assets at the next idle settlement.
    pub idle_fee_carry: Money,
    /// Fractional shares owed to the idle fee, in millionths of a share.
    pub share_frac: Vec<i64>,
    pub inbox: Inbox,
    pub tape_seen: Vec<usize>,
    pub delivery_cursor: usize,
    pub season_cursor: usize,
    pub hint_cursor: usize,
    pub skipped_since: u32,
    pub consecutive_errors: u32,
    pub(crate) decision_pace: crate::pace::DecisionPace,
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
            orders: OrderStore::default(),
            equity_sum: 0,
            loan_sum: 0,
            samples: 0,
            loan_samples: 0,
            credit_notional: 0,
            idle_fee_carry: 0,
            share_frac: vec![0; n],
            inbox: Inbox::default(),
            tape_seen: vec![0; n],
            delivery_cursor: 0,
            season_cursor: 0,
            hint_cursor: 0,
            skipped_since: 0,
            consecutive_errors: 0,
            decision_pace: Default::default(),
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

#[cfg(test)]
mod tests {
    use super::*;
    use rand::{Rng, SeedableRng};
    use std::collections::BTreeMap;

    #[test]
    fn compact_orders_match_map_through_insertion_fills_cancels_and_replacement() {
        let mut rng = rand_chacha::ChaCha8Rng::seed_from_u64(19);
        let mut actual = OrderStore::default();
        let mut expected = BTreeMap::new();
        for tick in 0..6000 {
            let id = rng.gen_range(0..128);
            match rng.gen_range(0..4) {
                0 | 1 => {
                    let order = OwnOrder {
                        stock: rng.gen_range(0..10),
                        side: if rng.gen_bool(0.5) {
                            Side::Buy
                        } else {
                            Side::Sell
                        },
                        price: rng.gen_range(1..1000),
                        remaining: rng.gen_range(1..100),
                        reserve_per_share: 20,
                        placed_tick: tick,
                        live: true,
                    };
                    assert_eq!(actual.insert(id, order.clone()), expected.insert(id, order));
                }
                2 => assert_eq!(actual.remove(&id), expected.remove(&id)),
                _ => {
                    // A cancelled order may remain while fills are in flight.
                    for order in [actual.get_mut(&id), expected.get_mut(&id)]
                        .into_iter()
                        .flatten()
                    {
                        order.remaining -= 1;
                        order.live = false;
                    }
                }
            }
            assert_eq!(actual.get(&id), expected.get(&id));
            assert_eq!(actual.len(), expected.len());
            assert_eq!(actual.is_empty(), expected.is_empty());
            assert!(actual.iter().eq(expected.iter()));
            assert!(actual.values().eq(expected.values()));
        }
        for id in expected.keys() {
            assert_eq!(&actual[id], &expected[id]);
            actual.remove(id);
        }
        assert!(actual.is_empty());
        assert!(actual.values().next().is_none());
    }
}
