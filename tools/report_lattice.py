"""Collect Lattice's complete experiments, including failed development arms."""

import json
from pathlib import Path
import re
import statistics


def main():
    root = Path(__file__).resolve().parents[1]
    experiments = {}
    for path in sorted((root / "runs/lattice").glob("*/evidence.json")):
        evidence = json.loads(path.read_text(encoding="utf-8"))
        evidence["manifest"]["sha256"] = {
            k.replace("\\", "/"): v for k, v in evidence["manifest"]["sha256"].items()
        }
        rows = evidence["worlds"]
        failures = {}
        for row in rows:
            for check in row["quality"]["checks"]:
                if not check["pass"]:
                    failures[check["name"]] = failures.get(check["name"], 0) + 1
        experiments[path.parent.name] = {
            "manifest": evidence["manifest"],
            "score": evidence["score"],
            "mean_final_equity_multiple": statistics.mean(r["return_ratio"] for r in rows),
            "minimum_percentile": min(r["percentile"] for r in rows),
            "wins": sum(r["rank"] == 1 for r in rows),
            "maximum_leverage": max(r["stats"]["max_leverage"] for r in rows),
            "mean_decision_us": statistics.mean(
                r["stats"]["decision_nanos_total"] / max(1, r["stats"]["decisions"]) / 1000
                for r in rows
            ),
            "audits": sum(r["audits"] for r in rows),
            "sum_stats": {k: sum(r["stats"][k] for r in rows) for k in [
                "liquidations", "rejects", "bot_errors", "skipped", "trade_fees", "idle_fees",
                "interest", "dividends", "combo_dividends",
            ]},
            "all_accounts": {k: sum(r[k] for r in rows) for k in
                             ["all_skipped", "all_disabled", "all_errors"]},
            "quality_pass_worlds": sum(r["quality"]["pass"] for r in rows),
            "quality_failed_world_counts": failures,
            "worlds": rows,
        }
    report = {
        "base_commit": "ea8c9f4",
        "scenario": "baseline v15; unchanged population plus one ordinary challenger",
        "date": "2026-10-21",
        "note": "Seeds 1-40 are development; 1001-1020 are a previously used project benchmark. "
                "73001-73020 were reserved for this bot before tuning. No selected reruns.",
        "freeze": json.loads((root / "runs/lattice/freeze.json").read_text(encoding="utf-8")),
        "checks": {
            "workspace_tests_passed": sum(map(int, re.findall(
                r"test result: ok\. (\d+) passed",
                (root / "runs/lattice/checks.log").read_text(encoding="utf-8"),
            ))),
            "workspace": json.loads(
                (root / "runs/lattice/checks.json").read_text(encoding="utf-8")),
            "scoped": json.loads(
                (root / "runs/lattice/scoped-checks.json").read_text(encoding="utf-8")),
            "unchanged_upstream_issues": {
                "format": ["crates/market/src/lib.rs", "crates/market/tests/engine.rs"],
                "clippy": {
                    "file": "crates/market/src/social.rs",
                    "lint": "items_after_test_module",
                },
                "verified_unchanged_from": "ea8c9f4",
            },
        },
        "experiments": experiments,
    }
    target = root / "docs/lattice-results.json"
    for name in ["train", "validation", "traditional", "reserved"]:
        if name in experiments:
            for key in ["crates/bots/src/lattice.rs", "target/release/llm-market.exe",
                        "scenarios/baseline.toml"]:
                assert experiments[name]["manifest"]["sha256"][key] == report["freeze"]["sha256"][key]
    target.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    for name, arm in experiments.items():
        print(json.dumps({
            "name": name,
            "mean": round(arm["score"]["mean"], 3),
            "sd": round(arm["score"]["sd"], 3),
            "top25": arm["score"]["top25_share"],
            "multiple": round(arm["mean_final_equity_multiple"], 3),
            "worst": round(arm["minimum_percentile"], 3),
            "leverage": round(arm["maximum_leverage"], 3),
            "quality_pass": arm["quality_pass_worlds"],
            "decision_us": round(arm["mean_decision_us"], 3),
        }), flush=True)


if __name__ == "__main__":
    main()
