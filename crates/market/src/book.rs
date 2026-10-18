//! Per-stock order book, public tape and mark price. One `StockMarket` per
//! stock, each behind its own lock.

use bot_api::{Level, OrderId, Price, Qty, Side, Tick};
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::ops::Bound::{Excluded, Unbounded};

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

#[derive(Default)]
struct PriceLevel {
    orders: VecDeque<Resting>,
    qty: Qty,
}

pub struct StockMarket {
    bids: BTreeMap<Price, PriceLevel>,
    asks: BTreeMap<Price, PriceLevel>,
    index: HashMap<OrderId, (Side, Price)>,
    depth_limit: usize,
    bid_depth: Vec<Level>,
    ask_depth: Vec<Level>,
    pub tape: Vec<TapeEntry>,
    pub last: Price,
    buckets: Vec<MarkBucket>,
    latest_tick: Tick,
    mark_notional: i64,
    mark_qty: i64,
    pub mark: Price,
}

impl StockMarket {
    pub fn new(initial_price: Price, mark_window: Tick) -> Self {
        Self::with_depth_cache(initial_price, mark_window, 0)
    }

    /// Keep the observation's public prefix current under the book's write lock.
    /// Readers copy contiguous levels without extra locks or snapshot allocations.
    pub fn with_depth_cache(initial_price: Price, mark_window: Tick, depth: usize) -> Self {
        Self {
            bids: BTreeMap::new(),
            asks: BTreeMap::new(),
            index: HashMap::new(),
            depth_limit: depth,
            bid_depth: Vec::new(),
            ask_depth: Vec::new(),
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
            mark_notional: 0,
            mark_qty: 0,
            mark: initial_price,
        }
    }

    fn side_mut(&mut self, side: Side) -> &mut BTreeMap<Price, PriceLevel> {
        match side {
            Side::Buy => &mut self.bids,
            Side::Sell => &mut self.asks,
        }
    }

    pub fn best(&self, side: Side) -> Option<Price> {
        if self.depth_limit > 0 {
            return match side {
                Side::Buy => self.bid_depth.first(),
                Side::Sell => self.ask_depth.first(),
            }
            .map(|level| level.price);
        }
        match side {
            Side::Buy => self.bids.keys().next_back().copied(),
            Side::Sell => self.asks.keys().next().copied(),
        }
    }

    pub fn depth(&self, side: Side, levels: usize) -> Vec<Level> {
        let mut out = Vec::new();
        self.depth_into(side, levels, &mut out);
        out
    }

    /// Reuse the worker's snapshot allocation while still reading the live book.
    pub fn depth_into(&self, side: Side, levels: usize, out: &mut Vec<Level>) {
        out.clear();
        if levels <= self.depth_limit {
            let cached = match side {
                Side::Buy => &self.bid_depth,
                Side::Sell => &self.ask_depth,
            };
            out.extend_from_slice(&cached[..levels.min(cached.len())]);
            return;
        }
        match side {
            Side::Buy => out.extend(self.bids.iter().rev().take(levels).map(|(p, q)| Level {
                price: *p,
                qty: q.qty,
            })),
            Side::Sell => out.extend(self.asks.iter().take(levels).map(|(p, q)| Level {
                price: *p,
                qty: q.qty,
            })),
        }
    }

    fn update_depth(&mut self, side: Side, price: Price, qty: Qty) {
        if self.depth_limit == 0 {
            return;
        }
        let (book, cached) = match side {
            Side::Buy => (&self.bids, &mut self.bid_depth),
            Side::Sell => (&self.asks, &mut self.ask_depth),
        };
        let pos = cached.partition_point(|level| match side {
            Side::Buy => level.price > price,
            Side::Sell => level.price < price,
        });
        if pos < cached.len() && cached[pos].price == price {
            if qty > 0 {
                cached[pos].qty = qty;
                return;
            }
            cached.remove(pos);
            // Refill only the new worst level, without rebuilding the prefix.
            let next = match (side, cached.last()) {
                (Side::Buy, Some(last)) => book.range(..last.price).next_back(),
                (Side::Sell, Some(last)) => book.range((Excluded(last.price), Unbounded)).next(),
                (Side::Buy, None) => book.iter().next_back(),
                (Side::Sell, None) => book.iter().next(),
            };
            if let Some((&price, level)) = next {
                cached.push(Level {
                    price,
                    qty: level.qty,
                });
            }
        } else if qty > 0 && pos < self.depth_limit {
            if cached.len() == self.depth_limit {
                cached.pop();
            }
            cached.insert(pos, Level { price, qty });
        }
    }

    pub fn add(&mut self, order: Resting) {
        self.index.insert(order.id, (order.side, order.price));
        let (side, price) = (order.side, order.price);
        let level = self.side_mut(side).entry(price).or_default();
        level.qty += order.qty;
        level.orders.push_back(order);
        let qty = level.qty;
        self.update_depth(side, price, qty);
    }

    /// Removes a resting order and returns its remaining quantity (0 if it is no longer on the book).
    pub fn cancel(&mut self, id: OrderId) -> Qty {
        let Some((side, price)) = self.index.remove(&id) else {
            return 0;
        };
        let book = self.side_mut(side);
        let q = book.get_mut(&price).expect("indexed level exists");
        let pos = q
            .orders
            .iter()
            .position(|o| o.id == id)
            .expect("indexed order exists");
        let order = q.orders.remove(pos).unwrap();
        q.qty -= order.qty;
        let qty = q.qty;
        if q.orders.is_empty() {
            book.remove(&price);
        }
        self.update_depth(side, price, qty);
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
            let maker = q.orders.front_mut().unwrap();
            debug_assert_ne!(maker.account, taker, "self-cross rule violated");
            let n = cap(best, qty.min(maker.qty));
            if n <= 0 {
                break;
            }
            maker.qty -= n;
            q.qty -= n;
            let level_qty = q.qty;
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
                let done = q.orders.pop_front().unwrap();
                if q.orders.is_empty() {
                    book.remove(&best);
                }
                self.index.remove(&done.id);
            }
            self.update_depth(side.opposite(), best, level_qty);
            self.record(tick, best, n, side, taker, maker_account);
        }
        fills
    }

    pub fn mark_tick(&self) -> Tick {
        self.latest_tick
    }

    /// Advance the shared mark clock even without a trade. Slower accounts and
    /// late fills cannot rewind it. An empty window uses the latest execution.
    pub fn advance_to(&mut self, tick: Tick) {
        if tick <= self.latest_tick {
            return;
        }
        let w = self.buckets.len() as Tick;
        let old_lo = self.latest_tick.saturating_sub(w - 1);
        self.latest_tick = tick;
        let lo = tick.saturating_sub(w - 1);
        if lo - old_lo >= w {
            self.mark_notional = 0;
            self.mark_qty = 0;
        } else {
            for expired in old_lo..lo {
                let b = &self.buckets[expired as usize % w as usize];
                if b.tick == expired {
                    self.mark_notional -= b.notional;
                    self.mark_qty -= b.qty;
                }
            }
        }
        self.update_mark();
    }

    fn update_mark(&mut self) {
        self.mark = if self.mark_qty > 0 {
            self.mark_notional / self.mark_qty
        } else {
            self.last
        };
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
        self.advance_to(tick);
        self.last = price;
        let w = self.buckets.len();
        let lo = self.latest_tick.saturating_sub(w as Tick - 1);
        // Expired late trades still belong on the tape and update `last`, but
        // must not evict a newer valid bucket with the same ring index.
        if tick >= lo {
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
            self.mark_notional += price * qty;
            self.mark_qty += qty;
        }
        self.update_mark();
    }

    pub fn resting_orders(&self) -> impl Iterator<Item = &Resting> {
        self.bids
            .values()
            .chain(self.asks.values())
            .flat_map(|level| &level.orders)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rolling_mark_matches_timestamp_map_with_silence_and_late_trades() {
        use rand::{Rng, SeedableRng};
        let mut rng = rand_chacha::ChaCha8Rng::seed_from_u64(81);
        for window in [1, 2, 20, 127] {
            let mut market = StockMarket::new(100, window);
            let mut by_tick = BTreeMap::<Tick, (i64, i64)>::new();
            let mut now = 0;
            for step in 0..10_000u32 {
                // Clock movement is independent of executions. The map oracle
                // has no ring slots, so old ticks cannot collide with new ones.
                now = now.max(step);
                market.advance_to(now);
                let lo = now.saturating_sub(window - 1);
                by_tick.retain(|t, _| *t >= lo);
                let expected = |map: &BTreeMap<Tick, (i64, i64)>, last| {
                    let (n, q): (i64, i64) =
                        map.values().fold((0, 0), |(n, q), b| (n + b.0, q + b.1));
                    if q == 0 {
                        last
                    } else {
                        n / q
                    }
                };
                assert_eq!(market.mark, expected(&by_tick, market.last));
                if rng.gen_bool(0.4) {
                    continue;
                }
                let tick = now.saturating_sub(rng.gen_range(0..window * 3 + 1));
                let price = rng.gen_range(1..100_000);
                let qty = rng.gen_range(1..10_000);
                if tick >= lo {
                    let b = by_tick.entry(tick).or_default();
                    b.0 += price * qty;
                    b.1 += qty;
                }
                market.record(tick, price, qty, Side::Buy, 1, 2);
                assert_eq!(
                    market.mark,
                    expected(&by_tick, price),
                    "window={window} now={now} trade={tick}"
                );
            }
            market.advance_to(Tick::MAX);
            assert_eq!(market.mark, market.last);
            market.record(Tick::MAX, 777, 1, Side::Buy, 1, 2);
            market.record(0, 999, 1, Side::Buy, 1, 2);
            assert_eq!(
                market.mark, 777,
                "expired trade must not evict a live ring bucket"
            );
        }
    }

    #[test]
    fn depth_tracks_partial_fills_cancels_and_price_time_priority() {
        let mut market = StockMarket::with_depth_cache(100, 20, 10);
        for (id, price, qty) in [(1, 101, 10), (2, 101, 15), (3, 102, 20)] {
            market.add(Resting {
                id,
                account: id as u32,
                side: Side::Sell,
                price,
                qty,
                reserve_per_share: 0,
            });
        }
        let mut depth = Vec::new();
        market.depth_into(Side::Sell, 10, &mut depth);
        assert_eq!(
            depth.iter().map(|l| (l.price, l.qty)).collect::<Vec<_>>(),
            [(101, 25), (102, 20)]
        );
        let fills = market.take(Side::Buy, Some(101), 17, 99, 1, &mut |_, n| n);
        assert_eq!(
            fills.iter().map(|f| (f.id, f.qty)).collect::<Vec<_>>(),
            [(1, 10), (2, 7)]
        );
        market.depth_into(Side::Sell, 10, &mut depth);
        assert_eq!(depth[0].qty, 8);
        assert_eq!(market.cancel(1), 0);
        assert_eq!(market.cancel(2), 8);
        market.depth_into(Side::Sell, 10, &mut depth);
        assert_eq!(depth.len(), 1);
        assert_eq!((depth[0].price, depth[0].qty), (102, 20));
        market.take(Side::Buy, None, 100, 99, 2, &mut |_, n| n);
        market.depth_into(Side::Sell, 10, &mut depth);
        assert!(depth.is_empty(), "reused snapshots must clear old levels");
    }

    #[test]
    fn aggregated_depth_matches_resting_orders_after_mixed_mutations() {
        use rand::{Rng, SeedableRng};
        let mut rng = rand_chacha::ChaCha8Rng::seed_from_u64(7);
        for depth_limit in [0, 1, 3, 10] {
            let mut market = StockMarket::with_depth_cache(100, 20, depth_limit);
            for id in 1..=1500 {
                let side = if rng.gen_bool(0.5) {
                    Side::Buy
                } else {
                    Side::Sell
                };
                match rng.gen_range(0..3) {
                    0 => market.add(Resting {
                        id,
                        account: 1,
                        side,
                        price: rng.gen_range(90..=110),
                        qty: rng.gen_range(1..100),
                        reserve_per_share: 0,
                    }),
                    1 => {
                        market.cancel(rng.gen_range(1..=id));
                    }
                    _ => {
                        market.take(
                            side,
                            None,
                            rng.gen_range(1..50),
                            99,
                            id as Tick,
                            &mut |_, n| n.min(7),
                        );
                    }
                }
                for side in [Side::Buy, Side::Sell] {
                    let mut expected = BTreeMap::<Price, Qty>::new();
                    for order in market.resting_orders().filter(|o| o.side == side) {
                        *expected.entry(order.price).or_default() += order.qty;
                    }
                    let mut expected: Vec<_> = expected.into_iter().collect();
                    if side == Side::Buy {
                        expected.reverse();
                    }
                    assert_eq!(market.best(side), expected.first().map(|&(price, _)| price));
                    for depth in [0, 1, 2, 3, 10, usize::MAX] {
                        let actual: Vec<_> = market
                            .depth(side, depth)
                            .iter()
                            .map(|l| (l.price, l.qty))
                            .collect();
                        assert_eq!(actual, expected[..depth.min(expected.len())]);
                    }
                }
            }
        }
    }
}
