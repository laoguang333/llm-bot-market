"""Informed trader: acts on news it hears before it is public.

On a private dividend increase it buys aggressively, on a decrease it sells,
then unwinds part of the position after the news becomes public.
"""


class Bot:
    def __init__(self, start):
        p = start["params"]
        self.lot = p.get("lot", 30)
        self.min_change = p.get("min_change", 0.03)
        self.trades = {}  # news id -> remaining lots to trade (+buy / -sell)

    def on_tick(self, obs):
        acct = obs["account"]
        for n in obs["news"]:
            m = obs["market"][n["stock"]]
            rel = n["dividend_change"] / max(1, m["dividend"])
            if not n["public"] and abs(rel) >= self.min_change and n["id"] not in self.trades:
                self.trades[n["id"]] = [n["stock"], 3 if rel > 0 else -3]
            elif n["public"] and n["id"] in self.trades:
                stock, left = self.trades[n["id"]]
                self.trades[n["id"]] = [stock, -1 if left >= 0 else 1]  # take some profit
        acts = []
        for nid, (s, left) in list(self.trades.items()):
            if left == 0:
                continue
            m = obs["market"][s]
            pos = acct["positions"][s]
            if left > 0 and m["asks"]:
                px = int(m["asks"][0][0] * 1.003) + 1
                q = min(self.lot, int(acct["cash_available"] // (px * 1.01)))
                if q > 0:
                    acts.append({"type": "limit", "stock": s, "side": "buy", "price": px, "qty": q, "tif": "ioc"})
                self.trades[nid][1] -= 1
            elif left < 0 and m["bids"]:
                q = min(self.lot, pos["available"])
                if q > 0:
                    px = max(1, int(m["bids"][0][0] * 0.997))
                    acts.append({"type": "limit", "stock": s, "side": "sell", "price": px, "qty": q, "tif": "ioc"})
                self.trades[nid][1] += 1
        return acts
