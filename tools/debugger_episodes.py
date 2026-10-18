"""Per-stock position episodes, grouped in account receipt order.

An episode ends only when inventory reaches zero. This is an accounting
grouping, not an inference about a bot's intended trade or news event.
"""


class Episodes:
    def __init__(self, start):
        self.items = []
        self.active = [None] * len(start["shares"])
        for stock, qty in enumerate(start["shares"]):
            if qty:
                e = self._new(stock, 0, 0)
                e.update(initial=True, initial_qty=qty, remaining=qty, peak_qty=qty,
                         entry_value=qty * start["rules"]["stocks"][stock]["initial_price"])

    def _new(self, stock, tick, receipt):
        e = dict(id=len(self.items), stock=stock, entry_tick=tick,
                 entry_receipt_tick=receipt, exit_tick=None, exit_receipt_tick=None,
                 initial=False, initial_qty=0, buy_qty=0, sell_qty=0,
                 entry_value=0, exit_value=0, fees=0, remaining=0, peak_qty=0,
                 fill_ids=[], adjustments=[], closed=False, adjusted=False)
        self.items.append(e)
        self.active[stock] = e
        return e

    def fill(self, f):
        stock = f["stock"]
        e = self.active[stock]
        if e is None:
            e = self._new(stock, f["tick"], f["receipt_tick"])
        f["episode_id"] = e["id"]
        e["fill_ids"].append(f["id"])
        e["fees"] += f["fee"]
        qty = f["qty"]
        if f["side"] == "Buy":
            e["buy_qty"] += qty
            e["entry_value"] += qty * f["price"]
            e["remaining"] += qty
            e["peak_qty"] = max(e["peak_qty"], e["remaining"])
        else:
            e["sell_qty"] += qty
            e["exit_value"] += qty * f["price"]
            if qty > e["remaining"]:
                e["adjusted"] = True
            e["remaining"] = max(0, e["remaining"] - qty)
            if e["remaining"] == 0:
                self._close(e, f["tick"], f["receipt_tick"])

    def adjustment(self, stock, qty, tick):
        e = self.active[stock]
        if e is None:
            e = self._new(stock, tick, tick)
        e["adjusted"] = True
        e["adjustments"].append(dict(tick=tick, qty=qty))
        e["remaining"] = max(0, e["remaining"] + qty)
        e["peak_qty"] = max(e["peak_qty"], e["remaining"])
        if e["remaining"] == 0:
            # No fictitious sale or exit price when fees destroy the last share.
            self._close(e, None, tick)

    def _close(self, e, tick, receipt):
        e.update(closed=True, exit_tick=tick, exit_receipt_tick=receipt)
        self.active[e["stock"]] = None

    def finish(self, fills, last_tick):
        for e in self.items:
            qty = e["initial_qty"] + e["buy_qty"]
            e["entry_price"] = e["entry_value"] / qty if qty and not e["adjusted"] else None
            e["exit_price"] = e["exit_value"] / e["sell_qty"] if e["sell_qty"] else None
            e["price_pnl"] = (e["exit_value"] - e["entry_value"] - e["fees"]
                              if e["closed"] and not e["adjusted"] else None)
            e["holding_ticks"] = (e["exit_receipt_tick"] if e["closed"] else last_tick) - e["entry_receipt_tick"]
            for side, key in [("Buy", "buy_orders"), ("Sell", "sell_orders")]:
                e[key] = len({fills[i].get("order_id", i) for i in e["fill_ids"] if fills[i]["side"] == side})
        return self.items
