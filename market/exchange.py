"""Accounts, order handling, fees and dividends. No bot processes here.

All money is integer cents, all quantities integer shares. Cash never goes
negative: buy orders reserve price * qty plus the worst-case trade fee,
sell orders reserve shares, and there is no borrowing or short selling.
"""

from __future__ import annotations

from dataclasses import dataclass, field

from .book import BUY, SELL, Order, OrderBook

MAX_PRICE = 10**9
MAX_QTY = 10**7


def ceil_div(a: int, b: int) -> int:
    return -(-a // b)


@dataclass
class Account:
    idx: int
    name: str
    owner: str
    cash: int
    shares: dict[str, int]
    reserved_cash: int = 0
    reserved_shares: dict[str, int] = field(default_factory=dict)
    open_orders: dict[int, Order] = field(default_factory=dict)
    order_reserve: dict[int, int] = field(default_factory=dict)  # order id -> cash reserved
    # carry-fee period state
    cash_sample_sum: int = 0
    cash_samples: int = 0
    credit_notional: int = 0
    fee_debt: int = 0
    # per-observation inbox, flushed when the bot is sent an observation
    inbox: dict[str, list] = field(default_factory=lambda: {"fills": [], "acks": [], "rejects": [], "news": [], "income": []})
    stats: dict[str, int] = field(default_factory=lambda: {
        "trade_fees": 0, "carry_fees": 0, "dividends": 0, "volume_notional": 0,
        "credited_notional": 0, "trades": 0, "orders": 0, "rejects": 0,
        "stp_cancels": 0, "timeouts": 0, "errors": 0,
    })

    def cash_available(self) -> int:
        return self.cash - self.reserved_cash

    def shares_available(self, stock: str) -> int:
        return self.shares.get(stock, 0) - self.reserved_shares.get(stock, 0)


class Exchange:
    def __init__(self, cfg: dict, stocks: list[str], accounts: list[Account]):
        fees = cfg.get("fees", {})
        rules = cfg.get("rules", {})
        self.trade_fee_bps = int(fees.get("trade_fee_bps", 5))
        self.carry_period = int(fees.get("carry_period", 50))
        self.carry_rate_ppm = round(float(fees.get("carry_rate", 0.002)) * 1_000_000)
        self.credit_ratio_ppm = round(float(fees.get("credit_ratio", 0.01)) * 1_000_000)
        self.credit_side = fees.get("credit_side", "both")  # both | maker | taker
        self.stp_scope = rules.get("stp_scope", "owner")  # owner | account
        self.max_actions = int(rules.get("max_actions_per_tick", 20))
        self.max_open_orders = int(rules.get("max_open_orders", 40))
        if self.credit_side not in ("both", "maker", "taker"):
            raise ValueError("fees.credit_side must be both, maker or taker")
        if self.stp_scope not in ("owner", "account"):
            raise ValueError("rules.stp_scope must be owner or account")

        self.stocks = list(stocks)
        self.books = {s: OrderBook(s) for s in stocks}
        self.accounts = accounts
        self.treasury = 0  # receives fees, pays dividends
        self.initial_cash = sum(a.cash for a in accounts)
        self.initial_shares = {s: sum(a.shares.get(s, 0) for a in accounts) for s in stocks}
        self._next_order_id = 1
        self._seq = 0
        self.tick = 0
        # public tape: per stock list of (tick, price, qty, aggressor side)
        self.tape: dict[str, list[tuple[int, int, int, str]]] = {s: [] for s in stocks}

    # ---- helpers -------------------------------------------------------
    def stp_group(self, acct: Account) -> str:
        return acct.owner if self.stp_scope == "owner" else f"#{acct.idx}"

    def trade_fee(self, notional: int) -> int:
        return ceil_div(notional * self.trade_fee_bps, 10_000)

    def buy_reserve_per_share(self, price: int) -> int:
        # fee(n * q) <= n * ceil(p * bps / 1e4) for any fill price q <= p
        return price + ceil_div(price * self.trade_fee_bps, 10_000)

    def _reject(self, acct: Account, action, reason: str) -> None:
        acct.stats["rejects"] += 1
        acct.inbox["rejects"].append({"action": action, "reason": reason})

    # ---- actions -------------------------------------------------------
    def apply_actions(self, acct: Account, actions) -> None:
        if not isinstance(actions, list):
            self._reject(acct, None, "actions must be a list")
            return
        if len(actions) > self.max_actions:
            self._reject(acct, None, f"only the first {self.max_actions} actions are processed")
            actions = actions[: self.max_actions]
        for a in actions:
            if not isinstance(a, dict):
                self._reject(acct, a, "action must be an object")
                continue
            kind = a.get("type")
            if kind == "cancel":
                self._cancel(acct, a)
            elif kind == "cancel_all":
                stock = a.get("stock")
                for oid in [o for o, od in acct.open_orders.items() if stock in (None, od.stock)]:
                    self.cancel_order(acct, oid)
            elif kind in ("limit", "market"):
                self._order(acct, a)
            else:
                self._reject(acct, a, "unknown action type")

    def _cancel(self, acct: Account, a: dict) -> None:
        oid = a.get("order_id")
        if oid not in acct.open_orders:
            self._reject(acct, a, "no such open order")
            return
        self.cancel_order(acct, oid)

    def cancel_order(self, acct: Account, oid: int) -> None:
        order = acct.open_orders[oid]
        self.books[order.stock].cancel(oid)
        self._release(acct, order)

    def _release(self, acct: Account, order: Order) -> None:
        del acct.open_orders[order.order_id]
        if order.side == BUY:
            acct.reserved_cash -= acct.order_reserve.pop(order.order_id)
        else:
            acct.reserved_shares[order.stock] -= order.qty

    def _order(self, acct: Account, a: dict) -> None:
        stock, side, qty = a.get("stock"), a.get("side"), a.get("qty")
        if stock not in self.books:
            return self._reject(acct, a, "unknown stock")
        if side not in (BUY, SELL):
            return self._reject(acct, a, "side must be buy or sell")
        if not isinstance(qty, int) or isinstance(qty, bool) or not 0 < qty <= MAX_QTY:
            return self._reject(acct, a, "qty must be a positive integer")
        market = a["type"] == "market"
        if market:
            price = MAX_PRICE if side == BUY else 1
            tif = "ioc"
        else:
            price = a.get("price")
            tif = a.get("tif", "gtc")
            if not isinstance(price, int) or isinstance(price, bool) or not 0 < price <= MAX_PRICE:
                return self._reject(acct, a, "price must be a positive integer (cents)")
            if tif not in ("gtc", "ioc"):
                return self._reject(acct, a, "tif must be gtc or ioc")
        if tif == "gtc" and len(acct.open_orders) >= self.max_open_orders:
            return self._reject(acct, a, f"at most {self.max_open_orders} open orders")

        reserve = 0
        if side == SELL:
            if acct.shares_available(stock) < qty:
                return self._reject(acct, a, "not enough shares (no short selling)")
            acct.reserved_shares[stock] = acct.reserved_shares.get(stock, 0) + qty
        elif not market:
            reserve = qty * self.buy_reserve_per_share(price)
            if acct.cash_available() < reserve:
                return self._reject(acct, a, "not enough cash")
            acct.reserved_cash += reserve

        acct.stats["orders"] += 1
        oid = self._next_order_id
        self._next_order_id += 1
        self._seq += 1
        order = Order(oid, acct.idx, self.stp_group(acct), stock, side, price, qty, self._seq, self.tick)
        acct.open_orders[oid] = order
        acct.order_reserve[oid] = reserve if side == BUY else 0

        max_fill = None
        if market and side == BUY:
            def max_fill(q, n):
                per = q + ceil_div(q * self.trade_fee_bps, 10_000)
                return min(n, acct.cash_available() // per)

        res = self.books[stock].match(side, price, qty, acct.idx, order.stp_group, max_fill,
                                      on_fill=lambda f: self._settle(f.maker, order, f.price, f.qty))
        for maker in res.stp_cancelled:
            owner = self.accounts[maker.account]
            owner.stats["stp_cancels"] += 1
            self._release(owner, maker)
            owner.inbox["acks"].append({"order_id": maker.order_id, "status": "cancelled_stp"})
        order.qty = res.remaining
        if order.qty > 0 and tif == "gtc":
            self.books[stock].add(order)
            status = "resting" if order.qty == qty else "partial_resting"
        else:
            self._release(acct, order)
            if order.qty == 0:
                status = "filled"
            else:
                status = "partial_cancelled" if order.qty < qty else "no_fill"
        acct.inbox["acks"].append({"cid": a.get("cid"), "order_id": oid, "status": status,
                                   "remaining": order.qty})

    def _settle(self, maker: Order, taker: Order, price: int, qty: int) -> None:
        notional = price * qty
        fee = self.trade_fee(notional)
        for order, role in ((maker, "maker"), (taker, "taker")):
            acct = self.accounts[order.account]
            if order.side == BUY:
                cost = notional + fee
                release = qty * self.buy_reserve_per_share(order.price) if acct.order_reserve.get(order.order_id) else 0
                acct.cash -= cost
                acct.reserved_cash -= release
                if release:
                    acct.order_reserve[order.order_id] -= release
                acct.shares[order.stock] = acct.shares.get(order.stock, 0) + qty
            else:
                acct.cash += notional - fee
                acct.shares[order.stock] -= qty
                acct.reserved_shares[order.stock] -= qty
            self.treasury += fee
            acct.stats["trade_fees"] += fee
            acct.stats["volume_notional"] += notional
            acct.stats["trades"] += 1
            if self.credit_side in ("both", role):
                acct.credit_notional += notional
                acct.stats["credited_notional"] += notional
            acct.inbox["fills"].append({"order_id": order.order_id, "stock": order.stock, "side": order.side,
                                        "price": price, "qty": qty, "fee": fee, "role": role, "tick": self.tick})
            if order is maker and maker.qty == 0:
                self._release(acct, maker)
        self.tape[maker.stock].append((self.tick, price, qty, taker.side))

    # ---- periodic rules -----------------------------------------------
    def pay_dividends(self, dividend: dict[str, int]) -> None:
        for acct in self.accounts:
            total = 0
            for s in self.stocks:
                amt = acct.shares.get(s, 0) * dividend[s]
                if amt:
                    total += amt
                    acct.inbox["income"].append({"kind": "dividend", "stock": s, "amount": amt, "tick": self.tick})
            acct.cash += total
            acct.stats["dividends"] += total
            self.treasury -= total

    def end_of_tick(self) -> None:
        for acct in self.accounts:
            acct.cash_sample_sum += acct.cash
            acct.cash_samples += 1
        if (self.tick + 1) % self.carry_period == 0:
            self.charge_carry()

    def carry_estimate(self, acct: Account) -> tuple[int, int, int]:
        avg = acct.cash_sample_sum // acct.cash_samples if acct.cash_samples else acct.cash
        fee = avg * self.carry_rate_ppm // 1_000_000
        credit = acct.credit_notional * self.credit_ratio_ppm // 1_000_000
        return fee, credit, max(0, fee - credit) + acct.fee_debt

    def charge_carry(self) -> None:
        for acct in self.accounts:
            fee, credit, due = self.carry_estimate(acct)
            paid = min(due, max(0, acct.cash_available()))
            acct.cash -= paid
            acct.fee_debt = due - paid
            self.treasury += paid
            acct.stats["carry_fees"] += paid
            if due or fee:
                acct.inbox["income"].append({"kind": "carry_fee", "amount": -paid, "gross": fee,
                                             "credit": min(fee, credit), "debt": acct.fee_debt, "tick": self.tick})
            acct.cash_sample_sum = 0
            acct.cash_samples = 0
            acct.credit_notional = 0

    # ---- audit ---------------------------------------------------------
    def check_invariants(self) -> None:
        cash = sum(a.cash for a in self.accounts)
        assert cash + self.treasury == self.initial_cash, "cash not conserved"
        for s in self.stocks:
            assert sum(a.shares.get(s, 0) for a in self.accounts) == self.initial_shares[s], f"{s} shares not conserved"
        for a in self.accounts:
            assert a.cash >= 0 and a.reserved_cash >= 0 and a.cash >= a.reserved_cash, f"{a.name} cash invalid"
            assert a.reserved_cash == sum(a.order_reserve.values()), f"{a.name} reserve mismatch"
            for s in self.stocks:
                held = a.shares.get(s, 0)
                res = a.reserved_shares.get(s, 0)
                assert 0 <= res <= held, f"{a.name} {s} share reserve invalid"
                assert res == sum(o.qty for o in a.open_orders.values() if o.stock == s and o.side == SELL)
