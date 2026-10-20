"""Guard against claiming victory over a group by selecting its weakest member."""
import unittest
from swift_crowding_hunt import group_metrics
from swift_crowding_hunt_report import paired


def account(initial, final):
    return {"initial_equity": initial, "final_equity": final,
            "return_pct": (final / initial - 1) * 100}


class CrowdingStatistics(unittest.TestCase):
    def test_beating_worst_is_not_beating_group_or_all(self):
        group = group_metrics([account(100, 50), account(100, 160), account(100, 160)], {"return_pct": 0})
        self.assertEqual(group["beaten_count"], 1)
        self.assertFalse(group["predator_beats_group"])
        self.assertFalse(group["predator_beats_all"])
        self.assertAlmostEqual(group["return_pct"], 70 / 3)

    def test_group_win_is_not_all_accounts_win(self):
        group = group_metrics([account(100, 50), account(100, 90), account(100, 120)], {"return_pct": -5})
        self.assertTrue(group["predator_beats_group"])
        self.assertFalse(group["predator_beats_all"])
        self.assertTrue(group["both_lose_predator_wins"])
        self.assertEqual(group["loss_count"], 2)

    def test_group_return_uses_total_capital(self):
        group = group_metrics([account(100, 200), account(900, 900)])
        self.assertAlmostEqual(group["return_pct"], 10)

    def test_matching_requires_complete_seed_and_order_pairs(self):
        rows = [{"seed": 1, "order": "a", "x": 10}, {"seed": 1, "order": "b", "x": -10}]
        refs = [{**r, "x": 0} for r in rows]
        result, _ = paired(rows, refs, {"x": lambda r: r["x"]})
        self.assertEqual(result["independent_seeds"], 1)
        self.assertEqual(result["x"]["ci95"], [0, 0])
        with self.assertRaises(AssertionError):
            paired(rows, refs[:1], {"x": lambda r: r["x"]})
        with self.assertRaises(AssertionError):
            paired(rows, [refs[0], refs[0]], {"x": lambda r: r["x"]})


if __name__ == "__main__":
    unittest.main()
