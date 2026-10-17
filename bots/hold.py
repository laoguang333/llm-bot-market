"""Buy and hold: puts its cash into the market, then just collects dividends.

Holding no cash means no carry fee; it is the passive benchmark. Every
`every` ticks it re-bids for whatever cash is still idle (including dividends).
"""


class Bot:
    def __init__(self, start):
        self.every = start["params"].get("every", 50)

    def on_tick(self, obs):
        if obs["tick"] < 20 or obs["tick"] % self.every:
            return []
        acct = obs["account"]
        stocks = list(obs["market"])
        acts = [{"type": "cancel_all"}]
        budget = acct["cash"] / len(stocks)
        for s in stocks:
            m = obs["market"][s]
            if not m["asks"]:
                continue
            px = int(m["asks"][0][0] * 1.01) + 1
            q = int(budget // (px * 1.002))
            if q > 0:
                acts.append({"type": "limit", "stock": s, "side": "buy", "price": px, "qty": q})
        return acts
