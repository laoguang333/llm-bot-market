"""Noise trader: random small limit orders around the current price.

It has no view; it trades so its volume offsets the cash carry fee and
supplies a little two-sided flow. Old orders are cancelled after `ttl` ticks.
"""

import random


def ref_price(m):
    if m["bids"] and m["asks"]:
        return (m["bids"][0][0] + m["asks"][0][0]) // 2
    return m["last"] or (m["bids"] and m["bids"][0][0]) or (m["asks"] and m["asks"][0][0])


class Bot:
    def __init__(self, start):
        p = start["params"]
        self.rng = random.Random(start["seed"])
        self.prob = p.get("prob", 0.15)
        self.max_qty = p.get("max_qty", 20)
        self.spread_bps = p.get("spread_bps", 80)
        self.ttl = p.get("ttl", 30)

    def on_tick(self, obs):
        acts = []
        t = obs["tick"]
        for o in obs["open_orders"]:
            if t - o["placed_tick"] > self.ttl:
                acts.append({"type": "cancel", "order_id": o["order_id"]})
        for s, m in obs["market"].items():
            if self.rng.random() > self.prob:
                continue
            ref = ref_price(m)
            if not ref:
                continue
            pos = obs["account"]["positions"][s]
            side = self.rng.choice(["buy", "sell"])
            off = self.rng.uniform(-1, 1) * self.spread_bps / 10000
            price = max(1, round(ref * (1 + off)))
            qty = self.rng.randint(1, self.max_qty)
            if side == "sell":
                qty = min(qty, pos["available"])
            elif obs["account"]["cash_available"] < price * qty * 1.01:
                qty = int(obs["account"]["cash_available"] // (price * 1.01))
            if qty > 0:
                acts.append({"type": "limit", "stock": s, "side": side, "price": price, "qty": qty})
        return acts
