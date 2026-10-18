"""Summarize the predeclared Swift v5 experiments without selecting parameters."""
import json
from pathlib import Path
import statistics

ROOT = Path(__file__).resolve().parents[1]
BATCHES = [
    ("development_initial", "swift-flow-v1"),
    ("development_extended", "swift-flow-dev40"),
    ("official_legacy", "swift-official-v1"),
    ("fresh_a", "swift-fresh-a-v1"),
    ("fresh_b", "swift-fresh-b-v1"),
]


def read_batch(label, name):
    root = ROOT / "runs/bench" / name
    score = json.loads((root / "challenge.json").read_text())
    manifest = json.loads((root / "manifest.json").read_text())
    accounts, all_accounts, audits = [], [], 0
    for seed in score["seeds"]:
        summary = json.loads((root / f"seed-{seed}/summary.json").read_text())
        accounts.append(next(a for a in summary["accounts"] if a["owner"] == "challenger"))
        all_accounts.extend(summary["accounts"])
        audits += summary["audits"]
    ret = [(a["return_ratio"]-1)*100 for a in accounts]
    p = score["percentiles"]
    return {
        "set": label, "path": str(root.relative_to(ROOT)), "seeds": score["seeds"],
        "mean_percentile": score["mean"], "sd_percentile": score["sd"],
        "top_quartile_share": score["top25_share"], "pass": score["pass"],
        "percentiles": p, "min_percentile": min(p),
        "first_place_worlds": sum(a["rank"] == 1 for a in accounts),
        "mean_return_pct": statistics.mean(ret), "median_return_pct": statistics.median(ret),
        "min_return_pct": min(ret), "losing_worlds": sum(r < 0 for r in ret),
        "audits": audits,
        "all_accounts_skipped": sum(a["stats"]["skipped"] for a in all_accounts),
        "all_accounts_disabled": sum(bool(a["disabled"]) for a in all_accounts),
        "all_accounts_bot_errors": sum(a["stats"]["bot_errors"] for a in all_accounts),
        "challenger_rejects": sum(a["stats"]["rejects"] for a in accounts),
        "challenger_liquidations": sum(a["stats"]["liquidations"] for a in accounts),
        "challenger_decisions": sum(a["stats"]["decisions"] for a in accounts),
        "mean_decision_us": statistics.mean(a["stats"]["decision_nanos_total"] / a["stats"]["decisions"] / 1000 for a in accounts),
        "mean_dividend_income_cents": statistics.mean(a["stats"]["dividends"] + a["stats"]["combo_dividends"] for a in accounts),
        "mean_non_dividend_net_pnl_cents": statistics.mean(a["equity"] - a["initial_equity"] - a["stats"]["dividends"] - a["stats"]["combo_dividends"] for a in accounts),
        "binary_sha256": manifest["binary_sha256"],
        "source_manifest": manifest,
    }


def main():
    batches = [read_batch(*x) for x in BATCHES]
    validation = batches[2:]
    assert all(b["source_manifest"] == validation[0]["source_manifest"] for b in validation), "validation source/binary changed"
    fresh = batches[3:]
    p = [p for b in fresh for p in b["percentiles"]]
    report = {
        "strategy": "swift", "frozen_commit": "5289b76",
        "scenario": "scenarios/baseline-v5.toml", "cash_cents": 2_000_000, "shares_each": 20,
        "threads": 4, "ticks": 3000,
        "gate": {"mean_percentile_min": 90, "top_quartile_share_min": 0.8},
        "development_trials": "One parameter set; 20 initial and 40 additional development worlds. No parameter sweep.",
        "fresh_combined": {"worlds": len(p), "mean_percentile": statistics.mean(p),
            "top_quartile_share": sum(x >= 75 for x in p)/len(p), "min_percentile": min(p),
            "mean_standard_error": statistics.stdev(p)/len(p)**0.5},
        "batches": batches,
    }
    out = ROOT / "docs/swift-v5-results.json"
    out.write_text(json.dumps(report, ensure_ascii=False, indent=2)+"\n", encoding="utf-8")
    for b in batches:
        print(json.dumps({k: v for k, v in b.items() if k not in {"source_manifest", "percentiles", "seeds"}}, ensure_ascii=False))
    print(json.dumps(report["fresh_combined"]))


if __name__ == "__main__":
    main()
