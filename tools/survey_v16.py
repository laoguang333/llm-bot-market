"""Frozen v16 ecology survey; no bot source is read or modified.

Run after a release build, through tools/arena_gate.py. Twenty worlds, two
baseline repetitions, plus one ordinary Almanac and Osprey challenge per world.
"""

from collections import Counter, defaultdict
import hashlib
import json
import os
from pathlib import Path
import statistics
import subprocess
import time

ROOT = Path(__file__).resolve().parents[1]
SEEDS = list(range(81001, 81021))
ARMS = ["baseline-0", "baseline-1", "almanac", "osprey"]


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def write_json(path, value):
    path.write_text(json.dumps(value, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")


def group(account):
    if account["strategy"] == "swift":
        return account["owner"]
    return account["strategy"]


def summarize_run(summary):
    groups = defaultdict(list)
    owners = {a["account"]: group(a) for a in summary["accounts"]}
    for account in summary["accounts"]:
        groups[group(account)].append(account)
    capital = sum(a["initial_equity"] for a in summary["accounts"])
    volume = sum(a["stats"]["volume_notional"] for a in summary["accounts"])
    rows = {}
    for name, accounts in groups.items():
        mean = lambda fn: statistics.mean(fn(a) for a in accounts)
        counterparties = Counter()
        for a in accounts:
            for owner, amount in a["notional_by_counterparty"].items():
                counterparties[owners.get(owner, owner)] += amount
        rows[name] = {
            "count": len(accounts),
            "percentile": mean(lambda a: a["percentile"]),
            "return_ratio": mean(lambda a: a["return_ratio"]),
            "profitable_share": mean(lambda a: a["return_ratio"] > 1),
            "top25_share": mean(lambda a: a["percentile"] >= 75),
            "firsts": sum(a["rank"] == 1 for a in accounts),
            "worst_return_ratio": min(a["return_ratio"] for a in accounts),
            "liquidated_share": mean(lambda a: a["stats"]["liquidations"] > 0),
            "insolvent_share": mean(lambda a: a["equity"] < 0),
            "max_leverage": max(a["stats"]["max_leverage"] for a in accounts),
            "initial_capital_share": sum(a["initial_equity"] for a in accounts) / capital,
            "volume_share": sum(a["stats"]["volume_notional"] for a in accounts) / volume,
            "dividends_ratio": mean(lambda a: a["stats"]["dividends"] / a["initial_equity"]),
            "cost_ratio": mean(lambda a: sum(a["stats"][k] for k in
                                          ["trade_fees", "idle_fees", "interest"]) / a["initial_equity"]),
            "decision_us": sum(a["stats"]["decision_nanos_total"] for a in accounts) /
                           sum(a["stats"]["decisions"] for a in accounts) / 1000,
            "rejects": sum(a["stats"]["rejects"] for a in accounts),
            "bad_debt": sum(a["stats"]["bad_debt"] for a in accounts),
            "counterparty_share": {k: v / sum(counterparties.values())
                                   for k, v in counterparties.items()} if counterparties else {},
        }
    return rows


def aggregate(rows):
    """Equal weight per seed, then per repetition, then per account in a group."""
    by_seed = defaultdict(list)
    for seed, row in rows:
        by_seed[seed].append(row)
    worlds = [{k: statistics.mean(r[k] for r in values)
               for k in values[0] if isinstance(values[0][k], (int, float))}
              for values in by_seed.values()]
    result = {k: statistics.mean(r[k] for r in worlds) for k in worlds[0]}
    result.update(
        seeds=len(worlds), runs=len(rows),
        percentile_sd_across_worlds=statistics.stdev(r["percentile"] for r in worlds),
        median_world_return_ratio=statistics.median(r["return_ratio"] for r in worlds),
        min_world_return_ratio=min(r["return_ratio"] for r in worlds),
        min_account_return_ratio=min(r["worst_return_ratio"] for _, r in rows),
        max_leverage=max(r["max_leverage"] for _, r in rows),
        total_firsts=sum(r["firsts"] for _, r in rows),
        total_rejects=sum(r["rejects"] for _, r in rows),
        total_bad_debt=sum(r["bad_debt"] for _, r in rows),
    )
    cp = defaultdict(list)
    for _, row in rows:
        for name in row["counterparty_share"]:
            cp[name].append(row["counterparty_share"][name])
    result["counterparty_share"] = {k: sum(v) / len(rows) for k, v in cp.items()}
    return result


def main():
    if os.name == "nt":
        from build_release import windows_build_budget
        budget = windows_build_budget()
    else:
        budget = os.cpu_count()
    binary = ROOT / "target/release" / ("llm-market.exe" if os.name == "nt" else "llm-market")
    scenario = ROOT / "scenarios/baseline-v16.toml"
    out = ROOT / "runs/ecology-v16"
    out.mkdir(parents=True, exist_ok=False)
    manifest = {
        "git_head": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(),
        "scenario": scenario.relative_to(ROOT).as_posix(),
        "scenario_sha256": sha(scenario), "binary_sha256": sha(binary),
        "seeds": SEEDS, "arms": ARMS, "threads": 4, "cpu_budget": budget,
        "baseline_repetitions": 2, "challenger_repetitions": 1,
        "review": "Existing reviewed, frozen bot versions; no strategy, engine or parameter changes.",
        "method": "No tuning. Equal weight per world; account averages within strategy. "
                  "Swift capital variants separate. Challenge arms not pooled with baseline.",
    }
    write_json(out / "manifest.json", manifest)
    records = []
    for index, seed in enumerate(SEEDS):
        order = ARMS[index % 4:] + ARMS[:index % 4]
        worlds = {}
        for arm in order:
            dest = out / f"{arm}-{seed}"
            baseline = arm.startswith("baseline")
            command = [str(binary), "run" if baseline else "challenge", str(scenario),
                       "--out", str(dest), "--threads", "4"]
            if baseline:
                command += ["--seed", str(seed)]
            else:
                command += ["--bot", arm, "--seeds", "1", "--first-seed", str(seed)]
            started = time.perf_counter()
            with dest.with_suffix(".log").open("w", encoding="utf-8") as log:
                subprocess.run(command, cwd=ROOT, stdout=log, stderr=subprocess.STDOUT, check=True)
            elapsed = time.perf_counter() - started
            folder = dest if baseline else dest / f"seed-{seed}"
            summary = json.loads((folder / "summary.json").read_text(encoding="utf-8"))
            assert summary["audits"] == 20 and summary["threads"] == 4
            assert all(not a["disabled"] and a["stats"]["bot_errors"] == 0
                       for a in summary["accounts"])
            if not baseline:
                a = next(a for a in summary["accounts"] if a["owner"] == "challenger")
                assert a["strategy"] == arm and a["initial_cash"] == 2_000_000
                assert a["initial_shares"] == [20] * len(summary["final_dividends"])
                assert a["news_multiplier"] == 1 and a["forecast_access"]["horizon"] == 0
                assert a["forecasts_received"] == 0
            record = {
                "arm": arm, "seed": seed, "command": command,
                "elapsed_seconds": elapsed, "simulation_seconds": summary["wall_seconds"],
                "summary_sha256": sha(folder / "summary.json"), "news_sha256": sha(folder / "news.csv"),
                "accounts": len(summary["accounts"]), "audits": summary["audits"],
                "quality": summary["quality"],
                "errors": sum(a["stats"]["bot_errors"] for a in summary["accounts"]),
                "skipped": sum(a["stats"]["skipped"] for a in summary["accounts"]),
                "disabled": sum(bool(a["disabled"]) for a in summary["accounts"]),
                "groups": summarize_run(summary),
            }
            records.append(record)
            worlds[arm] = summary
            write_json(out / "checkpoint.json", records)
            print(f"{len(records)}/80 {arm} seed={seed} wall={summary['wall_seconds']:.2f}s "
                  f"quality={summary['quality']['pass']}", flush=True)
        reference = worlds["baseline-0"]
        original = {a["account"]: a for a in reference["accounts"]}
        for arm, summary in worlds.items():
            assert summary["terminal_payments"] == reference["terminal_payments"]
            assert summary["final_dividends"] == reference["final_dividends"]
            current = {a["account"]: a for a in summary["accounts"]}
            assert set(current) == set(original) | ({"challenger"} if arm in ["almanac", "osprey"] else set())
            for name, account in original.items():
                for field in ["owner", "strategy", "initial_cash", "initial_shares",
                              "news_multiplier", "forecast_access"]:
                    assert account[field] == current[name][field]
        assert len({r["news_sha256"] for r in records if r["seed"] == seed}) == 1
    assert sha(binary) == manifest["binary_sha256"] and sha(scenario) == manifest["scenario_sha256"]
    baseline_groups = defaultdict(list)
    for r in records:
        if r["arm"].startswith("baseline"):
            for name, row in r["groups"].items():
                baseline_groups[name].append((r["seed"], row))
    aggregate_baseline = {name: aggregate(rows) for name, rows in baseline_groups.items()}
    challengers = {name: aggregate([(r["seed"], r["groups"][name])
                                   for r in records if r["arm"] == name])
                   for name in ["almanac", "osprey"]}
    results = {"manifest": manifest, "baseline": aggregate_baseline,
               "challengers": challengers, "runs": records}
    write_json(ROOT / "docs/ecology-v16-results.json", results)
    for name, row in sorted(aggregate_baseline.items(), key=lambda x: -x[1]["percentile"]):
        print(f"{name:20} percentile={row['percentile']:.2f} "
              f"net={(row['return_ratio']-1)*100:+.2f}% top25={row['top25_share']:.1%}", flush=True)
    for name, row in challengers.items():
        print(f"CHALLENGER {name} percentile={row['percentile']:.2f} "
              f"net={(row['return_ratio']-1)*100:+.2f}% top25={row['top25_share']:.1%}", flush=True)


if __name__ == "__main__":
    main()
