import unittest
from analyze_foundation_cashflows import passive,rounded


class PassiveTests(unittest.TestCase):
    def test_signed_rounding_and_entitlements(self):
        self.assertEqual(rounded(25,50),1)
        self.assertEqual(rounded(-25,50),-1)
        cfg=dict(ticks=4,fees={'idle_rate':0},world={'dividend_period':1,'combo_period':1,
                'dividend_scale':50,'combo_scale':50},stocks=[{'name':'A'},{'name':'B'}],
                scoring={'terminal_payment_ticks':2},combos=[dict(a='A',b='B',a_units=2,b_units=1,
                bonus=-25,start_tick=2,end_tick=3)])
        a=dict(initial_shares=[2,1],initial_cash=100,initial_equity=110)
        out=passive(cfg,{'terminal_payments':2},a,{'A':[25]*4,'B':[0]*4})
        self.assertEqual(out['ordinary'],3)
        self.assertEqual(out['combo'],-2)
        self.assertEqual(out['terminal'],2)
        self.assertEqual(out['final_cash'],101)
        self.assertAlmostEqual(out['net_return'],103/110-1)


if __name__=='__main__':unittest.main()
