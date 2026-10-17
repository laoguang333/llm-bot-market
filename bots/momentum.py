"""Trend follower: fast/slow moving averages of trade prices.

When the fast average is above the slow one by more than `threshold` it buys
toward a long target; below, it sells down. Orders cross the spread (IOC
limits a little through the touch).
"""


class Bot:
    def __init__(self, start):
        p = start["params"]
        self.fast_a = 2 / (p.get("fast", 20) + 1)
        self.slow_a = 2 / (p.get("slow", 120) + 1)
        self.threshold = p.get("threshold", 0.004)
        self.lot = p.get("lot", 15)
        self.every = p.get("every", 5)
        self.fast, self.slow = {}, {}

    def on_tick(self, obs):
        for s, trades in obs["trades"].items():
            for _, price, _, _ in trades:
                f = self.fast.get(s, price)
                sl = self.slow.get(s, price)
                self.fast[s] = f + self.fast_a * (price - f)
                self.slow[s] = sl + self.slow_a * (price - sl)
        if obs["tick"] % self.every or obs["tick"] < 100:
            return []
        acts = []
        acct = obs["account"]
        for s, m in obs["market"].items():
            if s not in self.slow:
                continue
            sig = self.fast[s] / self.slow[s] - 1
            pos = acct["positions"][s]
            if sig > self.threshold and m["asks"]:
                px = int(m["asks"][0][0] * 1.002) + 1
                q = min(self.lot, int(acct["cash_available"] // (px * 1.01)))
                if q > 0:
                    acts.append({"type": "limit", "stock": s, "side": "buy", "price": px, "qty": q, "tif": "ioc"})
            elif sig < -self.threshold and m["bids"] and pos["available"] > 0:
                px = max(1, int(m["bids"][0][0] * 0.998))
                acts.append({"type": "limit", "stock": s, "side": "sell", "price": px,
                             "qty": min(self.lot, pos["available"]), "tif": "ioc"})
        return acts
