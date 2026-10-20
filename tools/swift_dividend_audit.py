"""Read-only dividend entitlement bounds and payment-window quote diagnostics.

Uses the existing 160 traced worlds. Does not rerun, alter, or fit any bot.
The bounds do not require knowing when idle fees burned individual shares.
"""
import csv
import hashlib
import json
import os
from pathlib import Path
import statistics as stats

from build_release import windows_build_budget

ROOT = Path(__file__).resolve().parents[1]
RUN = ROOT / "runs/news-swift-20261019"
GROUPS = ["v5-swift-plain", "v6-swift-plain", "no_news-swift-plain",
          "active_no_news-swift-plain"]


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def world(group, seed):
    folder = RUN / group / f"seed-{seed}"
    summary = json.loads((folder / "summary.json").read_text(encoding="utf-8"))
    points = {}
    with (folder / "signals.csv").open(encoding="utf-8", newline="") as f:
        for r in csv.DictReader(f):
            points[int(r["tick"]), int(r["stock"])] = {
                k: float(v) if k in {"mid", "volume10", "flow10", "return30"} else int(v)
                for k, v in r.items()
            }
    assert summary["ticks"] == 3000
    changes = {h: [] for h in [0, 5, 10, 25]}
    changes_pct = {h: [] for h in changes}
    lower, upper = 0, 0
    for s in range(len(summary["final_dividends"])):
        initial = sum(a["initial_shares"][s] for a in summary["accounts"])
        final = sum(a["shares"][s] for a in summary["accounts"])
        assert initial - summary["shares_burned"][s] == final
        total_dividend = points[2995, s]["cumulative_dividend"]
        assert total_dividend >= 0
        # No issuance or shorting. The legal outstanding float can only shrink.
        upper += initial * total_dividend
        lower += final * total_dividend
        for tick in range(50, 2951, 50):
            before = points[tick - 5, s]
            dividend = points[tick, s]["cumulative_dividend"] - before["cumulative_dividend"]
            assert dividend >= 0
            if not dividend:
                continue
            for h in changes:
                after = points[tick + h, s]
                if min(before["bid"], before["ask"], after["bid"], after["ask"]) > 0:
                    changes[h].append((after["mid"] - before["mid"]) / dividend)
                    changes_pct[h].append((after["mid"] / before["mid"] - 1) * 100)
    actual = sum(a["stats"]["dividends"] for a in summary["accounts"])
    return {
        "seed": seed, "actual_ordinary_dividends_cents": actual,
        "legal_lower_bound_cents": lower, "legal_upper_bound_cents": upper,
        "above_upper_cents": max(0, actual - upper),
        "below_lower_cents": max(0, lower - actual),
        "mean_mid_change_per_dividend": {h: stats.mean(v) for h, v in changes.items()},
        "mean_mid_change_pct": {h: stats.mean(v) for h, v in changes_pct.items()},
        "hashes": {name: sha(folder / name) for name in ["summary.json", "signals.csv"]},
    }


def main():
    if os.name == "nt":
        windows_build_budget()
    result = {
        "note": "Post hoc audit, 160 existing matches / 40 seeds. Ordinary cash dividends only; excludes combos and terminal settlement. Quote windows are descriptive, not causal estimates or executed returns.",
        "bounds": "For each stock, final float * cumulative dividend <= legitimate total payment <= initial float * cumulative dividend; shares only decrease by burns. Violations prove an entitlement inconsistency, not its allocation to Swift or its impact on strategy returns.",
        "quotes": "Change from payment tick - 5 to tick + h, stocks with two-sided quotes and positive current payout; average within world, then across worlds. Other simultaneous fees/news and asynchronous personal clocks remain confounders.",
        "script_sha256": sha(Path(__file__)),
        "groups": {},
    }
    for group in GROUPS:
        rows = [world(group, seed) for seed in range(21001, 21041)]
        result["groups"][group] = {
            "worlds": rows,
            "worlds_above_upper": sum(r["above_upper_cents"] > 0 for r in rows),
            "worlds_below_lower": sum(r["below_lower_cents"] > 0 for r in rows),
            "max_proven_excess_yuan": max(r["above_upper_cents"] for r in rows) / 100,
            "mean_mid_change_per_dividend": {
                h: stats.mean(r["mean_mid_change_per_dividend"][h] for r in rows)
                for h in [0, 5, 10, 25]
            },
        }
        g = result["groups"][group]
        print(group, "above upper", g["worlds_above_upper"], "below lower", g["worlds_below_lower"],
              "max proven excess yuan", g["max_proven_excess_yuan"], flush=True)
    dest = ROOT / "docs/swift-dividend-audit.json"
    dest.write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(dest)


if __name__ == "__main__":
    main()
