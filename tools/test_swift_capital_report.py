"""Statistical checks for matching controls, clustering and endowment changes."""
import unittest
from swift_capital import cases
from swift_capital_report import effect


def row(seed, order, target_return, hunter_return, hunter_capital):
    return {"seed": seed, "order": order,
            "target": {"return_pct": target_return, "pnl_yuan": target_return},
            "predator": {"return_pct": hunter_return, "pnl_yuan": hunter_capital * hunter_return / 100}}


class CapitalStatistics(unittest.TestCase):
    def test_common_tenfold_start_is_not_duplicated(self):
        treatments = cases()
        self.assertEqual(len(treatments), 35)
        self.assertEqual(sum(c[3] == 10 for c in treatments), 5)
        self.assertEqual(len({c[0] for c in treatments}), 35)

    def test_control_matching_requires_same_seed_and_same_order(self):
        treatment = [row(1, "first", -10, -3, 1000), row(1, "second", 99, 99, 1000)]
        control = [row(1, "first", 20, -10, 1000), row(2, "second", 0, 0, 1000)]
        result = effect(treatment, control)
        self.assertEqual(result["runs"], 1)
        self.assertEqual(result["target_return_pct"], -30)
        self.assertEqual(result["predator_pnl_yuan"], 70)

    def test_opposite_order_effects_cluster_to_zero(self):
        treatment = [row(1, "first", 10, 20, 1000), row(1, "second", -10, -20, 1000),
                     row(2, "first", 30, 40, 1000), row(2, "second", -30, -40, 1000)]
        control = [row(r["seed"], r["order"], 0, 0, 1000) for r in treatment]
        result = effect(treatment, control)
        self.assertEqual(result["independent_seeds"], 2)
        self.assertEqual(result["target_return_pct_ci95"], [0, 0])
        self.assertEqual(result["predator_pnl_yuan_ci95"], [0, 0])

    def test_lower_percentage_loss_can_cost_more_money(self):
        treatment = [row(1, "first", -5, -1, 1000000)]
        control = [row(1, "first", -5, -5, 10000)]
        result = effect(treatment, control)
        self.assertEqual(result["predator_return_pct"], 4)
        self.assertEqual(result["predator_pnl_yuan"], -9500)


if __name__ == "__main__":
    unittest.main()
