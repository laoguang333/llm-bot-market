import unittest

from market.exchange import Account, Exchange


def make(fees=None, rules=None, n=3, owners=None):
    owners = owners or [f"o{i}" for i in range(n)]
    accts = [Account(i, f"a{i}", owners[i], 1_000_000, {"X": 100}) for i in range(n)]
    cfg = {"fees": {"trade_fee_bps": 0, "carry_period": 10, "carry_rate": 0.01, "credit_ratio": 0.01, **(fees or {})},
           "rules": rules or {}}
    return Exchange(cfg, ["X"], accts), accts


def limit(side, price, qty, **kw):
    return {"type": "limit", "stock": "X", "side": side, "price": price, "qty": qty, **kw}


class TestMatching(unittest.TestCase):
    def test_price_time_priority_and_partial_fill(self):
        ex, (a, b, c) = make()
        ex.apply_actions(a, [limit("sell", 101, 5)])
        ex.apply_actions(b, [limit("sell", 100, 5)])
        ex.apply_actions(a, [limit("sell", 100, 5)])
        ex.apply_actions(c, [limit("buy", 101, 12)])
        fills = [(f["price"], f["qty"]) for f in c.inbox["fills"]]
        self.assertEqual(fills, [(100, 5), (100, 5), (101, 2)])
        self.assertEqual(b.inbox["fills"][0]["qty"], 5)  # b was first at 100
        self.assertEqual(c.shares["X"], 112)
        self.assertEqual(c.cash, 1_000_000 - 1000 - 202)
        self.assertEqual(ex.books["X"].depth("sell", 5), [[101, 3]])
        ex.check_invariants()

    def test_market_buy_limited_by_cash(self):
        ex, (a, b, _) = make()
        ex.initial_cash -= b.cash - 250
        b.cash = 250
        ex.apply_actions(a, [limit("sell", 100, 10)])
        ex.apply_actions(b, [{"type": "market", "stock": "X", "side": "buy", "qty": 10}])
        self.assertEqual(b.shares["X"], 102)
        self.assertEqual(b.cash, 50)
        ex.check_invariants()

    def test_no_short_selling_or_overspending(self):
        ex, (a, _, _) = make()
        ex.apply_actions(a, [limit("sell", 100, 60), limit("sell", 100, 60)])
        self.assertEqual(a.stats["rejects"], 1)
        ex.apply_actions(a, [limit("buy", 10_000, 101)])
        self.assertEqual(a.inbox["rejects"][-1]["reason"], "not enough cash")
        ex.check_invariants()

    def test_cancel_releases_reservation(self):
        ex, (a, _, _) = make(fees={"trade_fee_bps": 10})
        ex.apply_actions(a, [limit("buy", 1000, 10)])
        self.assertEqual(a.reserved_cash, 10 * (1000 + 1))
        oid = a.inbox["acks"][0]["order_id"]
        ex.apply_actions(a, [{"type": "cancel", "order_id": oid}])
        self.assertEqual(a.reserved_cash, 0)
        self.assertFalse(ex.books["X"].orders)
        ex.check_invariants()

    def test_fees_paid_to_treasury(self):
        ex, (a, b, _) = make(fees={"trade_fee_bps": 10})
        ex.apply_actions(a, [limit("sell", 1000, 10)])
        ex.apply_actions(b, [limit("buy", 1000, 10)])
        self.assertEqual(a.cash, 1_000_000 + 10_000 - 10)
        self.assertEqual(b.cash, 1_000_000 - 10_000 - 10)
        self.assertEqual(ex.treasury, 20)
        ex.check_invariants()


class TestSelfTrade(unittest.TestCase):
    def test_same_account_never_trades(self):
        ex, (a, _, _) = make(rules={"stp_scope": "account"})
        ex.apply_actions(a, [limit("sell", 100, 5), limit("buy", 100, 5)])
        self.assertEqual(a.stats["trades"], 0)
        self.assertEqual(a.stats["stp_cancels"], 1)
        self.assertEqual(ex.books["X"].depth("buy", 5), [[100, 5]])
        self.assertEqual(ex.books["X"].depth("sell", 5), [])
        ex.check_invariants()

    def test_same_owner_blocked_under_owner_scope(self):
        ex, (a, b, c) = make(owners=["same", "same", "other"])
        ex.apply_actions(a, [limit("sell", 100, 5)])
        ex.apply_actions(c, [limit("sell", 101, 5)])
        ex.apply_actions(b, [limit("buy", 101, 5)])
        self.assertEqual(a.stats["trades"], 0)
        self.assertEqual(b.inbox["fills"][0]["price"], 101)  # skipped the sibling, hit the other owner
        ex.check_invariants()

    def test_same_owner_allowed_under_account_scope(self):
        ex, (a, b, _) = make(owners=["same", "same", "other"], rules={"stp_scope": "account"})
        ex.apply_actions(a, [limit("sell", 100, 5)])
        ex.apply_actions(b, [limit("buy", 100, 5)])
        self.assertEqual(b.shares["X"], 105)


class TestCarryAndDividends(unittest.TestCase):
    def test_carry_fee_offset_by_volume(self):
        ex, (a, b, c) = make()
        ex.apply_actions(a, [limit("sell", 1000, 50)])
        ex.apply_actions(b, [limit("buy", 1000, 50)])  # 50_000 notional -> 500 credit each
        for t in range(10):
            ex.tick = t
            ex.end_of_tick()
        # c held 1_000_000 cash: fee 10_000, no credit
        self.assertEqual(c.stats["carry_fees"], 10_000)
        # a averaged 1_050_000 -> fee 10_500, credit 500
        self.assertEqual(a.stats["carry_fees"], 10_000)
        self.assertEqual(b.stats["carry_fees"], 950_000 // 100 - 500)
        self.assertEqual(c.credit_notional, 0)
        ex.check_invariants()

    def test_all_in_stock_pays_no_carry(self):
        ex, (a, _, _) = make()
        a.cash = 0
        ex.initial_cash -= 1_000_000
        for t in range(10):
            ex.tick = t
            ex.end_of_tick()
        self.assertEqual(a.stats["carry_fees"], 0)

    def test_maker_only_credit(self):
        ex, (a, b, _) = make(fees={"credit_side": "maker"})
        ex.apply_actions(a, [limit("sell", 1000, 10)])
        ex.apply_actions(b, [limit("buy", 1000, 10)])
        self.assertEqual(a.credit_notional, 10_000)
        self.assertEqual(b.credit_notional, 0)

    def test_dividends_on_all_shares(self):
        ex, (a, _, _) = make()
        ex.apply_actions(a, [limit("sell", 1000, 40)])  # reserved shares still earn
        ex.pay_dividends({"X": 7})
        self.assertEqual(a.cash, 1_000_000 + 700)
        self.assertEqual(ex.treasury, -2100)
        ex.check_invariants()


if __name__ == "__main__":
    unittest.main()
