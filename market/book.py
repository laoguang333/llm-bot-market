"""Price-time priority limit order book for one stock.

Prices are integer ticks (cents), quantities integer shares. The book only
matches; reservations, fees and accounting live in the engine.
"""

from __future__ import annotations

import bisect
from collections import deque
from dataclasses import dataclass, field

BUY = "buy"
SELL = "sell"


@dataclass
class Order:
    order_id: int
    account: int
    stp_group: str
    stock: str
    side: str
    price: int  # limit price; for market orders the engine passes a bound
    qty: int  # remaining
    seq: int  # arrival sequence, for time priority
    placed_tick: int = 0


@dataclass
class Fill:
    maker: Order
    taker_account: int
    price: int
    qty: int


@dataclass
class MatchResult:
    fills: list[Fill] = field(default_factory=list)
    stp_cancelled: list[Order] = field(default_factory=list)
    remaining: int = 0


class OrderBook:
    def __init__(self, stock: str):
        self.stock = stock
        self._levels: dict[str, dict[int, deque[Order]]] = {BUY: {}, SELL: {}}
        # ascending price lists; best bid is the last of bids, best ask the first of asks
        self._prices: dict[str, list[int]] = {BUY: [], SELL: []}
        self.orders: dict[int, Order] = {}

    # ---- queries -------------------------------------------------------
    def best(self, side: str) -> int | None:
        prices = self._prices[side]
        if not prices:
            return None
        return prices[-1] if side == BUY else prices[0]

    def depth(self, side: str, levels: int) -> list[list[int]]:
        prices = self._prices[side]
        seq = reversed(prices) if side == BUY else iter(prices)
        out = []
        for p in seq:
            if len(out) >= levels:
                break
            out.append([p, sum(o.qty for o in self._levels[side][p])])
        return out

    # ---- mutation ------------------------------------------------------
    def add(self, order: Order) -> None:
        side_levels = self._levels[order.side]
        q = side_levels.get(order.price)
        if q is None:
            q = deque()
            side_levels[order.price] = q
            bisect.insort(self._prices[order.side], order.price)
        q.append(order)
        self.orders[order.order_id] = order

    def cancel(self, order_id: int) -> Order | None:
        order = self.orders.pop(order_id, None)
        if order is None:
            return None
        q = self._levels[order.side][order.price]
        q.remove(order)
        if not q:
            self._drop_level(order.side, order.price)
        return order

    def _drop_level(self, side: str, price: int) -> None:
        del self._levels[side][price]
        prices = self._prices[side]
        i = bisect.bisect_left(prices, price)
        del prices[i]

    def match(self, side: str, price: int, qty: int, taker_account: int,
              stp_group: str, max_fill=None, on_fill=None) -> MatchResult:
        """Match an incoming order against the opposite side.

        `max_fill(price, qty) -> qty` lets the engine cap each fill (e.g. cash
        available for a market buy); `on_fill(fill)` is called as each fill
        happens, before the next one is sized. Resting orders of the same STP group are
        cancelled (cancel-resting self-trade prevention) instead of filled.
        """
        res = MatchResult()
        opp = SELL if side == BUY else BUY
        while qty > 0:
            best = self.best(opp)
            if best is None:
                break
            if (side == BUY and best > price) or (side == SELL and best < price):
                break
            q = self._levels[opp][best]
            maker = q[0]
            if maker.stp_group == stp_group:
                q.popleft()
                del self.orders[maker.order_id]
                res.stp_cancelled.append(maker)
                if not q:
                    self._drop_level(opp, best)
                continue
            n = min(qty, maker.qty)
            if max_fill is not None:
                n = max_fill(best, n)
                if n <= 0:
                    break
            maker.qty -= n
            qty -= n
            fill = Fill(maker=maker, taker_account=taker_account, price=best, qty=n)
            res.fills.append(fill)
            if maker.qty == 0:
                q.popleft()
                del self.orders[maker.order_id]
                if not q:
                    self._drop_level(opp, best)
            if on_fill is not None:
                on_fill(fill)
        res.remaining = qty
        return res
