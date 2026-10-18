"""Accounting edge cases: receipt ordering, preview duplication and noncash fees."""
import copy
import unittest
from debugger_data import analyze_account


def stats(**values):
    result=dict(trades=0,trade_fees=0,idle_fees=0,idle_cash_fees=0,
                idle_stock_fees=0,interest=0,dividends=0,combo_dividends=0)
    result.update(values)
    return result


def frame(tick,cash,shares,*,end=False,fills=(),income=(),st=None,post_fills=()):
    return dict(tick=tick,end=end,decided=not end,pre=None if end else dict(shares=[shares]),
        post=dict(cash=cash,shares=[shares],gross=shares*100,equity=cash+shares*100),
        fills=list(fills),income=list(income),stats=st or stats(),post_fills=list(post_fills),
        swift_verified=None,quotes=[[99,101,1,1,100,100,0,1,1,0,0]])


def fill(side,qty,price,fee,tick):
    return dict(side=side,stock=0,qty=qty,price=price,fee=fee,tick=tick,maker=False)


def analyze(frames,cash=1000,shares=0):
    start=dict(cash=cash,shares=[shares],rules=dict(ticks_total=len(frames)-1,stocks=[dict(initial_price=100)]))
    result=dict(initial_equity=cash+shares*100,cash=frames[-1]["post"]["cash"],
                shares=frames[-1]["post"]["shares"],stats=frames[-1]["stats"])
    return analyze_account(start,result,frames,[100])


class LedgerTests(unittest.TestCase):
    def test_scaled_position_is_one_episode_until_flat(self):
        trades=[fill("Buy",4,100,4,1),fill("Buy",2,110,2,2),
                fill("Sell",1,120,1,3),fill("Buy",1,115,1,4),
                fill("Sell",6,90,6,5)]
        a=analyze([frame(0,1000,0),frame(1,911,0,fills=trades,st=stats(trades=5,trade_fees=14)),
                   frame(2,911,0,end=True,st=stats(trades=5,trade_fees=14))])
        e,=a["episodes"]
        self.assertTrue(e["closed"])
        self.assertEqual((e["buy_qty"],e["sell_qty"],e["peak_qty"]),(7,7,6))
        self.assertEqual(e["price_pnl"],-89)
        self.assertEqual(e["price_pnl"],sum(p["price_pnl"] for p in a["pairs"]))
        self.assertEqual(e["fill_ids"],list(range(5)))
        self.assertEqual([f["episode_id"] for f in a["fills"]],[0]*5)

    def test_flat_then_reentry_in_same_receipt_batch_is_two_episodes(self):
        trades=[fill("Buy",1,100,1,0),fill("Sell",1,110,1,0),fill("Buy",2,105,1,0)]
        a=analyze([frame(0,797,2,fills=trades,st=stats(trades=3,trade_fees=3)),
                   frame(1,797,2,end=True,st=stats(trades=3,trade_fees=3))])
        closed,opened=a["episodes"]
        self.assertEqual(closed["price_pnl"],8)
        self.assertEqual(closed["fill_ids"],[0,1])
        self.assertFalse(opened["closed"])
        self.assertEqual(opened["remaining"],2)
        self.assertIsNone(opened["price_pnl"])

    def test_preview_is_not_counted_twice_and_final_flush_is_counted(self):
        buy=fill("Buy",2,100,2,0);sell=fill("Sell",1,130,1,1)
        f0=frame(0,798,2,post_fills=[buy]);f0["pre"]["shares"]=[0]
        f1=frame(1,927,1,fills=[buy],post_fills=[sell],st=stats(trades=2,trade_fees=3));f1["pre"]["shares"]=[2]
        end=frame(2,927,1,end=True,fills=[sell],st=stats(trades=2,trade_fees=3))
        a=analyze([f0,f1,end])
        self.assertEqual(len(a["fills"]),2)
        self.assertEqual(a["pairs"][0]["price_pnl"],28)
        self.assertEqual(a["pairs"][0]["holding_ticks"],1)
        self.assertEqual(a["stocks"][0]["open_lots"][0]["qty"],1)
        for k in ["cash_error","fill_count_error","trade_fee_error","unknown_inventory"]:
            self.assertEqual(a["validation"][k],0,k)

    def test_partial_fifo_initial_inventory_and_buy_fee_allocation(self):
        buy=fill("Buy",2,120,2,0);sell=fill("Sell",3,150,3,1)
        st=stats(trades=2,trade_fees=5)
        f0=frame(0,1000,2)
        f1=frame(1,1205,1,fills=[buy,sell],st=st)
        a=analyze([f0,f1,frame(2,1205,1,end=True,st=st)],shares=2)
        self.assertEqual([p["qty"] for p in a["pairs"]],[2,1])
        self.assertEqual([p["initial"] for p in a["pairs"]],[True,False])
        self.assertEqual([p["price_pnl"] for p in a["pairs"]],[98,28])
        self.assertEqual(a["validation"]["cash_error"],0)

    def test_stock_fee_is_destroyed_inventory_not_cash_or_a_sale(self):
        st=stats(idle_fees=105,idle_cash_fees=5,idle_stock_fees=100)
        a=analyze([frame(0,1000,2),frame(1,995,1,income=[{"IdleFee":{"paid":105}}],st=st),
                   frame(2,995,1,end=True,st=st)],shares=2)
        self.assertEqual(a["validation"]["cash_error"],0)
        self.assertEqual(a["pairs"],[])
        self.assertEqual(a["adjustments"][0]["kind"],"inventory_reduction")
        self.assertEqual(a["curves"][-1][6],0)
        self.assertTrue(a["episodes"][0]["adjusted"])
        self.assertFalse(a["episodes"][0]["closed"])
        self.assertIsNone(a["episodes"][0]["price_pnl"])

    def test_destroyed_last_share_is_not_a_trade_exit(self):
        st=stats(idle_fees=100,idle_stock_fees=100)
        a=analyze([frame(0,1000,1),frame(1,1000,0,st=st),frame(2,1000,0,end=True,st=st)],shares=1)
        e,=a["episodes"]
        self.assertTrue(e["closed"])
        self.assertTrue(e["initial"])
        self.assertIsNone(e["exit_tick"])
        self.assertIsNone(e["exit_price"])
        self.assertIsNone(e["price_pnl"])

    def test_dividend_combo_interest_and_residual_reconcile(self):
        st=stats(dividends=20,combo_dividends=-3,interest=2)
        inc=[{"Dividend":{"stock":0,"amount":20}},{"Combo":{"amount":-3}},{"Interest":{"amount":2}}]
        a=analyze([frame(0,1000,1),frame(1,1015,1,income=inc,st=st),frame(2,1015,1,end=True,st=st)],shares=1)
        self.assertEqual(a["validation"]["cash_error"],0)
        self.assertEqual(a["validation"]["dividend_error"],0)
        self.assertEqual(a["curves"][-1][6],0)

    def test_unknown_inventory_has_no_invented_profit(self):
        f=fill("Sell",1,100,0,1)
        a=analyze([frame(0,1000,1),frame(1,1100,0,fills=[f],st=stats(trades=1)),
                   frame(2,1100,0,end=True,st=stats(trades=1))])
        self.assertEqual(a["validation"]["unknown_inventory"],1)
        self.assertIsNone(a["pairs"][0]["price_pnl"])

    def test_old_incomplete_recordings_are_rejected(self):
        f=frame(0,1000,0);del f["stats"]["idle_cash_fees"]
        with self.assertRaisesRegex(ValueError,"record a new game"):
            analyze([copy.deepcopy(f)])


if __name__=="__main__":unittest.main()
