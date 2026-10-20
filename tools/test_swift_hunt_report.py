"""Tests for the scientific comparison, not a restatement of trading rules."""
import unittest
from swift_hunt import summarize
from swift_hunt_report import cluster_ci


def account(ret, capital):
    return {"return_pct":ret,"drawdown_pct":max(0,-ret),"pnl_yuan":capital*ret/100}


class HuntStatistics(unittest.TestCase):
    def test_opposite_order_effects_cancel_inside_seed_clusters(self):
        rows=[{"seed":1,"gap":-10},{"seed":1,"gap":10},
              {"seed":2,"gap":-40},{"seed":2,"gap":40}]
        interval,n=cluster_ci(rows,lambda r:r["gap"])
        self.assertEqual(n,2)
        self.assertEqual(interval,[0,0])

    def test_relative_percentage_win_can_have_larger_cash_loss(self):
        row={"target":account(-10,100),"predator":account(-3,1000)}
        result=summarize([row])
        self.assertEqual(result["wins"],1)
        self.assertEqual(result["both_lose_predator_wins"],1)
        self.assertLess(result["predator"]["mean_pnl_yuan"],result["target"]["mean_pnl_yuan"])

    def test_mutual_loss_subset_and_ties_are_not_global_successes(self):
        pairs=[(-10,-3),(-10,-20),(3,2),(5,5)]
        rows=[{"target":account(t,100),"predator":account(p,1000)} for t,p in pairs]
        result=summarize(rows)
        self.assertEqual(result["both_lose"],2)
        self.assertEqual(result["both_lose_predator_wins"],1)
        self.assertEqual(result["wins"],1)


if __name__=="__main__":
    unittest.main()
