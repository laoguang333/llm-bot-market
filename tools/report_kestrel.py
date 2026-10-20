"""Validate frozen Kestrel challenge artifacts and rebuild docs/kestrel-results.json.

Run after the dev, holdout and confirmation commands documented in docs/kestrel.md.
This reads completed experiments; it does not run bots or select parameters.
"""
import hashlib
import json
from pathlib import Path
import re
import statistics


ROOT = Path(__file__).resolve().parents[1]


def read(path):
    return json.loads(path.read_text(encoding="utf-8"))


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def summarize(name, seeds, freeze):
    path = ROOT / "runs/bench" / name
    score = read(path / "challenge.json")
    manifest = {k.replace("\\", "/"): v for k, v in read(path / "manifest.json").items()}
    require(score["seeds"] == seeds, f"unexpected seeds: {name}")
    require(score["bot"] == "kestrel", f"unexpected bot: {name}")
    for file, digest in freeze["sha256"].items():
        key = "binary_sha256" if file.endswith(".exe") else file
        require(manifest[key] == digest, f"freeze mismatch: {name}: {file}")
    rows, accounts, summaries = [], [], []
    for seed in seeds:
        summary = read(path / f"seed-{seed}/summary.json")
        challengers = [a for a in summary["accounts"] if a["owner"] == "challenger"]
        require(len(challengers) == 1, f"challenger count: {name}/{seed}")
        a = challengers[0]
        require(a["strategy"] == "kestrel", "wrong strategy")
        require(summary["ticks"] == 10000 and summary["threads"] == 4, "wrong scenario execution")
        require(a["initial_cash"] == 2_000_000 and a["initial_shares"] == [20] * 10, "wrong capital")
        require(a["news_multiplier"] == 1.0, "enhanced ordinary news")
        require(a["forecast_access"]["horizon"] == 0 and a["forecasts_received"] == 0, "extra research")
        require(a["stats"]["decisions"] == 10000, "missing decisions")
        require(abs(a["return_ratio"] - a["equity"] / a["initial_equity"]) < 1e-9, "return denominator")
        rows.append({
            "seed": seed, "percentile": a["percentile"], "rank": a["rank"],
            "return_pct": (a["return_ratio"] - 1.0) * 100.0,
            "initial_equity_cents": a["initial_equity"], "final_equity_cents": a["equity"],
            "accounts": len(summary["accounts"]), "wall_seconds": summary["wall_seconds"],
            "audits": summary["audits"], "quality_pass": summary["quality"]["pass"],
        })
        accounts.append(a)
        summaries.append(summary)
    percentiles = [r["percentile"] for r in rows]
    require(percentiles == score["percentiles"], "score disagrees with raw summaries")
    mean = statistics.mean(percentiles)
    top25 = sum(p >= 75 for p in percentiles) / len(rows)
    require(abs(mean - score["mean"]) < 1e-9, "incorrect mean")
    require(score["pass"] == (mean >= 90 and top25 >= 0.8), "incorrect pass flag")
    all_accounts = [a for s in summaries for a in s["accounts"]]
    # Accounting identity, not causal signal attribution. Adding back total idle
    # fees also adds back the mark-value charge for burned shares; the residual
    # therefore includes inventory/terminal valuation effects, not just trade P&L.
    accounting = []
    for a in accounts:
        st, initial = a["stats"], a["initial_equity"]
        parts = {key: sign * st[key] / initial * 100.0 for key, sign in [
            ("dividends", 1), ("combo_dividends", 1), ("trade_fees", -1),
            ("idle_fees", -1), ("interest", -1), ("bad_debt", 1),
        ]}
        parts["net"] = (a["equity"] / initial - 1.0) * 100.0
        parts["trading_inventory_terminal_residual"] = parts["net"] - sum(
            value for key, value in parts.items() if key != "net")
        accounting.append(parts)
    crowd_counts = [sum(a["strategy"] == "profit_effect" for a in s["accounts"])
                    for s in summaries]
    crowd_notional = sum(value for a in accounts for owner, value in a["notional_by_counterparty"].items()
                         if owner.startswith("profit_effect#"))
    total_notional = sum(sum(a["notional_by_counterparty"].values()) for a in accounts)
    return {
        "name": name, "seeds": seeds, "repeats": 1, "pass": score["pass"],
        "mean_percentile": mean, "sd_percentile": score["sd"],
        "min_percentile": min(percentiles), "top25_share": top25,
        "first_place_count": sum(a["rank"] == 1 for a in accounts),
        "mean_return_pct": statistics.mean(r["return_pct"] for r in rows),
        "median_return_pct": statistics.median(r["return_pct"] for r in rows),
        "min_return_pct": min(r["return_pct"] for r in rows),
        "positive_return_count": sum(r["return_pct"] > 0 for r in rows),
        "wall_seconds_sum": sum(r["wall_seconds"] for r in rows),
        "wall_seconds_median": statistics.median(r["wall_seconds"] for r in rows),
        "audits": sum(r["audits"] for r in rows),
        "quality_pass_count": sum(r["quality_pass"] for r in rows),
        "challenger_stats_sum": {k: sum(a["stats"][k] for a in accounts) for k in [
            "decisions", "rejects", "skipped", "bot_errors", "liquidations", "bad_debt",
            "trade_fees", "idle_fees", "interest", "dividends", "combo_dividends",
        ]},
        "mean_decision_us": sum(a["stats"]["decision_nanos_total"] for a in accounts)
            / sum(a["stats"]["decisions"] for a in accounts) / 1000,
        "all_accounts_skipped": sum(a["stats"]["skipped"] for a in all_accounts),
        "all_accounts_errors": sum(a["stats"]["bot_errors"] for a in all_accounts),
        "all_accounts_disabled": sum(bool(a["disabled"]) for a in all_accounts),
        "accounting_return_percentage_points": {
            key: statistics.mean(a[key] for a in accounting) for key in accounting[0]},
        "profit_effect_accounts_range": [min(crowd_counts), max(crowd_counts)],
        "profit_effect_counterparty_notional": crowd_notional,
        "all_counterparty_notional": total_notional,
        "profit_effect_counterparty_notional_share": crowd_notional / max(total_notional, 1),
        "command": read(path / "command.json"), "rows": rows,
    }, manifest


def main():
    freeze = read(ROOT / "runs/kestrel/freeze.json")
    # Historical reports validate the recorded manifests below, not today's build.
    # The default baseline and factory can evolve after the frozen experiment.
    require(hashlib.sha256((ROOT / "scenarios/baseline-v13.toml").read_bytes()).hexdigest()
            == freeze["sha256"]["scenarios/baseline.toml"], "archived v13 differs from freeze")
    groups = [
        ("development", "kestrel-dev-1", freeze["development_seeds"]),
        ("holdout", "kestrel-holdout", freeze["holdout_seeds"]),
        ("confirmation", "kestrel-confirm", freeze["confirmation_seeds"]),
    ]
    results, manifests = {}, []
    for label, name, seeds in groups:
        results[label], manifest = summarize(name, seeds, freeze)
        manifests.append(manifest)
    require(all(m == manifests[0] for m in manifests), "compiled inputs changed between experiments")
    log = (ROOT / "runs/kestrel/checks.log").read_text(encoding="utf-8")
    output = {
        "freeze": freeze,
        "pre_run_review": read(ROOT / "runs/kestrel/pre-run-review.json"),
        "source_and_binary_identical_across_suites": True,
        "ordinary_permissions_and_capital_verified": True,
        "release_tests_passed": sum(map(int, re.findall(r"test result: ok. (\d+) passed", log))),
        "bot_package_fmt_and_strict_clippy": "passed",
        "workspace_strict_clippy": "blocked by pre-existing clippy::items_after_test_module in crates/market/src/social.rs:20",
        "development_reference": {
            "note": "Ordinary-capital Titan, development seeds only; not a held-out comparison.",
            "score": read(ROOT / "runs/bench/kestrel-reference-titan/challenge.json"),
            "diagnostics": read(ROOT / "runs/bench/kestrel-reference-titan/diagnostics.json"),
            "binary_sha256": read(ROOT / "runs/bench/kestrel-reference-titan/manifest.json")["binary_sha256"],
        },
        "results": results,
    }
    path = ROOT / "docs/kestrel-results.json"
    path.write_text(json.dumps(output, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({k: {key: value for key, value in v.items() if key not in {"rows", "command"}}
                      for k, v in results.items()}, ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
