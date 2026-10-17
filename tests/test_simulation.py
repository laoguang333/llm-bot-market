import json
import tempfile
import unittest
from pathlib import Path

from market.simulation import Simulation, load_config
from market.world import World

ROOT = Path(__file__).resolve().parents[1]


def tiny_cfg(bots, ticks=40, **extra):
    cfg = {
        "_dir": str(ROOT / "tests"), "seed": 3, "ticks": ticks,
        "sandbox": {"step_timeout_ms": 150, "max_late_ticks": 50},
        "world": {"dividend_period": 10, "news": {"events_per_1000_ticks": 0}},
        "stocks": [{"name": "X", "dividend": 10, "initial_price": 1000}],
        "bots": bots,
    }
    cfg.update(extra)
    return cfg


class TestSandbox(unittest.TestCase):
    def test_late_crashing_and_broken_bots_are_contained(self):
        bots = [
            {"file": "bots/slow.py", "owner": "s", "cash": 10_000, "shares": {"X": 5}},
            {"file": "bots/crash.py", "owner": "c", "cash": 10_000, "shares": {"X": 5}},
            {"file": "bots/badload.py", "owner": "b", "cash": 10_000, "shares": {"X": 5}},
        ]
        with tempfile.TemporaryDirectory() as d:
            res = Simulation(tiny_cfg(bots), Path(d) / "out").run()
        slow, crash, bad = res["accounts"]
        self.assertGreaterEqual(slow["timeouts"], 2)
        self.assertEqual(slow["disabled"], "")
        self.assertEqual(crash["errors"], 1)
        self.assertTrue(crash["disabled"])  # SystemExit at tick 5 kills the process
        self.assertEqual(crash["orders"], 4)  # ticks 0,1,2,4
        self.assertIn("start failed", bad["disabled"])


class TestWorld(unittest.TestCase):
    def test_staged_release_reaches_everyone_once_before_effect(self):
        cfg = {"dividend_period": 10, "news": {"events_per_1000_ticks": 20, "lead_ticks": 50,
                                                "stages": [[0, 0.1], [20, 0.5], [40, 1.0]]}}
        w = World(cfg, [{"name": "X", "dividend": 100}], 20, 1000, 1)
        self.assertTrue(w.events)
        seen = {}
        for t in range(1100):
            for ev, accts, public in w.releases(t):
                self.assertLess(t, ev.effective)
                for a in accts:
                    self.assertNotIn((ev.event_id, a), seen)
                    seen[(ev.event_id, a)] = t
        self.assertEqual(len(seen), 20 * len(w.events))
        first = [t for (e, _), t in seen.items() if e == w.events[0].event_id and t == w.events[0].first_reveal]
        self.assertEqual(len(first), 2)  # 10% of 20 accounts


class TestBaselineScenario(unittest.TestCase):
    def test_short_baseline_run_conserves_and_reproduces(self):
        cfg = load_config(ROOT / "scenarios" / "baseline.toml")
        outs = []
        with tempfile.TemporaryDirectory() as d:
            for k in range(2):
                c = dict(cfg, ticks=150)
                res = Simulation(c, Path(d) / f"out{k}").run()
                self.assertFalse([a for a in res["accounts"] if a["disabled"]])
                self.assertGreater(sum(a["trades"] for a in res["accounts"]), 0)
                outs.append((Path(d) / f"out{k}" / "trades.csv").read_text())
                json.loads((Path(d) / f"out{k}" / "summary.json").read_text())
        self.assertEqual(outs[0], outs[1])


if __name__ == "__main__":
    unittest.main()
