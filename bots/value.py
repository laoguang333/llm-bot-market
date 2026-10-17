"""Value investor: prices each stock off the dividend it knows about.

value = expected dividend per period / required yield, where the expected
dividend includes news this bot has been told but that has not taken effect.
It buys below value and sells above it, holding a position that grows with
the mispricing.
"""


class Bot:
    def __init__(self, start):
        p = start["params"]
        self.req_yield = p.get("required_yield", 0.01)
        self.margin = p.get("margin", 0.02)
        self.max_weight = p.get("max_weight", 0.8)
        self.lot = p.get("lot", 20)
        self.pending = {}  # news id -> (stock, change, effective tick)

    def on_tick(self, obs):
        t = obs["tick"]
        for n in obs["news"]:
            self.pending[n["id"]] = (n["stock"], n["dividend_change"], n["effective_tick"])
        self.pending = {k: v for k, v in self.pending.items() if v[2] > t}
        acts = [{"type": "cancel_all"}]
        acct = obs["account"]
        cash = acct["cash"]
        prices = {s: m["last"] for s, m in obs["market"].items() if m["last"]}
        equity = cash + sum(acct["positions"][s]["shares"] * p for s, p in prices.items())
        for s, m in obs["market"].items():
            div = m["dividend"] + sum(c for st, c, _ in self.pending.values() if st == s)
            value = div / self.req_yield
            px = prices.get(s) or value
            gap = (value - px) / value
            # target weight in [0, max_weight] increasing in undervaluation
            w = min(self.max_weight, max(0.0, 0.4 + 4 * gap))
            target = int(w * equity / len(obs["market"]) / px)
            have = acct["positions"][s]["shares"]
            bid = int(value * (1 - self.margin))
            ask = int(value * (1 + self.margin)) + 1
            if have < target:
                q = min(self.lot, target - have, int(cash // (bid * 1.01)))
                if q > 0:
                    acts.append({"type": "limit", "stock": s, "side": "buy", "price": bid, "qty": q})
            if have > target:
                q = min(self.lot, have - target)
                acts.append({"type": "limit", "stock": s, "side": "sell", "price": ask, "qty": q})
        return acts
