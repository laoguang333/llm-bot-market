class Bot:
    def __init__(self, start):
        pass

    def on_tick(self, obs):
        if obs["tick"] == 3:
            raise ValueError("boom")
        if obs["tick"] == 5:
            raise SystemExit(1)
        print("noise on stdout must not break the protocol")
        return [{"type": "limit", "stock": "X", "side": "buy", "price": 100, "qty": 1}]
