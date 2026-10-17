"""Liquidity provider: an ordinary bot that quotes both sides.

There is no privileged market maker in this market; this strategy just tries
to earn the spread. It anchors on a smoothed trade price, skews quotes against
its inventory, and refreshes every `refresh` ticks.
"""


class Bot:
    def __init__(self, start):
        p = start["params"]
        self.spread_bps = p.get("spread_bps", 60)
        self.size = p.get("size", 15)
        self.refresh = p.get("refresh", 5)
        self.skew_bps = p.get("skew_bps", 150)
        self.alpha = p.get("alpha", 0.1)
        self.ref = {s["name"]: s.get("initial_price") for s in start["rules"]["stocks"]}
        self.base = {}

    def on_tick(self, obs):
        acct = obs["account"]
        for s, trades in obs["trades"].items():
            for _, price, _, _ in trades:
                self.ref[s] = price if not self.ref[s] else (1 - self.alpha) * self.ref[s] + self.alpha * price
        if obs["tick"] % self.refresh:
            return []
        acts = [{"type": "cancel_all"}]
        for s, m in obs["market"].items():
            ref = self.ref[s]
            if m["bids"] and m["asks"]:
                ref = 0.5 * ref + 0.25 * (m["bids"][0][0] + m["asks"][0][0]) if ref else (m["bids"][0][0] + m["asks"][0][0]) / 2
            if not ref:
                continue
            shares = acct["positions"][s]["shares"]
            base = self.base.setdefault(s, max(shares, 1))
            inv = max(-1.0, min(1.0, (shares - base) / base))
            mid = ref * (1 - inv * self.skew_bps / 10000)
            half = self.spread_bps / 20000
            bid, ask = int(mid * (1 - half)), int(mid * (1 + half)) + 1
            q_bid = min(self.size, int(acct["cash_available"] / len(obs["market"]) // (bid * 1.01)))
            q_ask = min(self.size, shares)
            if q_bid > 0:
                acts.append({"type": "limit", "stock": s, "side": "buy", "price": bid, "qty": q_bid})
            if q_ask > 0:
                acts.append({"type": "limit", "stock": s, "side": "sell", "price": ask, "qty": q_ask})
        return acts
