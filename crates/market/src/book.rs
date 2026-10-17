//! Per-stock order book, public tape and mark price. One `StockMarket` per
//! stock, each behind its own lock.

use bot_api::{Level, OrderId, Price, Qty, Side, Tick};
use std::collections::{BTreeMap, HashMap, VecDeque};

#[derive(Clone, Debug)]
pub struct Resting {
    pub id: OrderId,
    pub account: u32,
    pub side: Side,
    pub price: Price,
    pub qty: Qty,
    /// Cash reserved per share (buy orders only).
    pub reserve_per_share: i64,
}

#[derive(Clone, Copy, Debug)]
pub struct MakerFill {
    pub id: OrderId,
    pub account: u32,
    pub side: Side,
    pub price: Price,
    pub qty: Qty,
    pub reserve_per_share: i64,
}

#[derive(Clone, Copy, Debug)]
pub struct TapeEntry {
    pub tick: Tick,
    pub price: Price,
    pub qty: Qty,
    pub aggressor: Side,
    pub taker: u32,
    pub maker: u32,
}

#[derive(Clone, Copy, Debug, Default)]
struct MarkBucket {
    tick: Tick,
    notional: i64,
    qty: i64,
}

pub struct StockMarket {
    bids: BTreeMap<Price, VecDeque<Resting>>,
    asks: BTreeMap<Price, VecDeque<Resting>>,
    index: HashMap<OrderId, (Side, Price)>,
    pub tape: Vec<TapeEntry>,
    pub last: Price,
    buckets: Vec<MarkBucket>,
    latest_tick: Tick,
    pub mark: Price,
}

impl StockMarket {
    pub fn new(initial_price: Price, mark_window: Tick) -> Self {
        Self {
            bids: BTreeMap::new(),
            asks: BTreeMap::new(),
            index: HashMap::new(),
            tape: Vec::new(),
            last: initial_price,
            buckets: vec![
                MarkBucket {
                    tick: Tick::MAX,
                    ..Default::default()
                };
                mark_window.max(1) as usize
            ],
            latest_tick: 0,
            mark: initial_price,
        }
    }

    fn side_mut(&mut self, side: Side) -> &mut BTreeMap<Price, VecDeque<Resting>> {
        match side {
            Side::Buy => &mut self.bids,
            Side::Sell => &mut self.asks,
        }
    }

    pub fn best(&self, side: Side) -> Option<Price> {
        match side {
            Side::Buy => self.bids.keys().next_back().copied(),
            Side::Sell => self.asks.keys().next().copied(),
        }
    }

    pub fn depth(&self, side: Side, levels: usize) -> Vec<Level> {
        let sum = |q: &VecDeque<Resting>| q.iter().map(|o| o.qty).sum();
        match side {
            Side::Buy => self
                .bids
                .iter()
                .rev()
                .take(levels)
                .map(|(p, q)| Level {
                    price: *p,
                    qty: sum(q),
                })
                .collect(),
            Side::Sell => self
                .asks
                .iter()
                .take(levels)
                .map(|(p, q)| Level {
                    price: *p,
                    qty: sum(q),
                })
                .collect(),
        }
    }

    pub fn add(&mut self, order: Resting) {
        self.index.insert(order.id, (order.side, order.price));
        let (side, price) = (order.side, order.price);
        self.side_mut(side)
            .entry(price)
            .or_default()
            .push_back(order);
    }

    /// Removes a resting order and returns its remaining quantity (0 if it is no longer on the book).
    pub fn cancel(&mut self, id: OrderId) -> Qty {
        let Some((side, price)) = self.index.remove(&id) else {
            return 0;
        };
        let book = self.side_mut(side);
        let q = book.get_mut(&price).expect("indexed level exists");
        let pos = q
            .iter()
            .position(|o| o.id == id)
            .expect("indexed order exists");
        let order = q.remove(pos).unwrap();
        if q.is_empty() {
            book.remove(&price);
        }
        order.qty
    }

    /// Matches an incoming order. `limit` None means market order. `cap(price, n)`
    /// may reduce each fill (buying power). Never matches the taker's own orders:
    /// the self-cross rule guarantees they cannot be reached, and this is asserted.
    pub fn take(
        &mut self,
        side: Side,
        limit: Option<Price>,
        mut qty: Qty,
        taker: u32,
        tick: Tick,
        cap: &mut dyn FnMut(Price, Qty) -> Qty,
    ) -> Vec<MakerFill> {
        let mut fills = Vec::new();
        while qty > 0 {
            let Some(best) = self.best(side.opposite()) else {
                break;
            };
            if let Some(l) = limit {
                if (side == Side::Buy && best > l) || (side == Side::Sell && best < l) {
                    break;
                }
            }
            let book = self.side_mut(side.opposite());
            let q = book.get_mut(&best).unwrap();
            let maker = q.front_mut().unwrap();
            debug_assert_ne!(maker.account, taker, "self-cross rule violated");
            let n = cap(best, qty.min(maker.qty));
            if n <= 0 {
                break;
            }
            maker.qty -= n;
            qty -= n;
            fills.push(MakerFill {
                id: maker.id,
                account: maker.account,
                side: maker.side,
                price: best,
                qty: n,
                reserve_per_share: maker.reserve_per_share,
            });
            let maker_account = maker.account;
            if maker.qty == 0 {
                let done = q.pop_front().unwrap();
                if q.is_empty() {
                    book.remove(&best);
                }
                self.index.remove(&done.id);
            }
            self.record(tick, best, n, side, taker, maker_account);
        }
        fills
    }

    fn record(
        &mut self,
        tick: Tick,
        price: Price,
        qty: Qty,
        aggressor: Side,
        taker: u32,
        maker: u32,
    ) {
        self.tape.push(TapeEntry {
            tick,
            price,
            qty,
            aggressor,
            taker,
            maker,
        });
        self.last = price;
        let w = self.buckets.len();
        let b = &mut self.buckets[tick as usize % w];
        if b.tick != tick {
            *b = MarkBucket {
                tick,
                notional: 0,
                qty: 0,
            };
        }
        b.notional += price * qty;
        b.qty += qty;
        self.latest_tick = self.latest_tick.max(tick);
        let lo = self.latest_tick.saturating_sub(w as Tick - 1);
        let (mut n, mut v) = (0i64, 0i64);
        for b in &self.buckets {
            if b.tick != Tick::MAX && b.tick >= lo && b.tick <= self.latest_tick {
                n += b.notional;
                v += b.qty;
            }
        }
        if v > 0 {
            self.mark = n / v;
        }
    }

    pub fn resting_orders(&self) -> impl Iterator<Item = &Resting> {
        self.bids.values().chain(self.asks.values()).flatten()
    }
}
