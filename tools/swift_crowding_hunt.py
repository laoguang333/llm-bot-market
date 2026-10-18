"""Frozen 1/3/5/10 Swift experiment with one cash-funded anonymous hunter."""
import argparse
import copy
import csv
from datetime import datetime, timezone
import json
from pathlib import Path
import shutil
import statistics as stats
import subprocess
import tomllib

from swift_stress import ROOT, sha, toml_text
from swift_stress_report import drawdown
from swift_hunt import source_manifest
from swift_capital import read_capital_world, verify

BASE = ROOT / "runs/swift-crowding-hunt-20261018"


def group_metrics(accounts, predator=None):
    initial = sum(a["initial_equity"] for a in accounts)
    final = sum(a["final_equity"] for a in accounts)
    returns = [a["return_pct"] for a in accounts]
    result = {"count": len(accounts), "initial_equity": initial, "final_equity": final,
              "return_pct": (final / initial - 1) * 100, "pnl_yuan": (final - initial) / 100,
              "median_return_pct": stats.median(returns), "best_return_pct": max(returns),
              "worst_return_pct": min(returns), "first_return_pct": returns[0],
              "last_return_pct": returns[-1], "loss_count": sum(r < 0 for r in returns)}
    if predator:
        p = predator["return_pct"]
        beaten = sum(p > r for r in returns)
        result.update({"beaten_count": beaten, "beaten_share": beaten / len(accounts),
                       "predator_beats_all": beaten == len(accounts),
                       "predator_beats_group": p > result["return_pct"],
                       "advantage_pp": p - result["return_pct"],
                       "both_lose": p < 0 and result["return_pct"] < 0,
                       "both_lose_predator_wins": result["return_pct"] < p < 0})
    return result


def prepare(stage, first, seeds, counts):
    out = BASE / stage
    if out.exists():
        raise SystemExit(f"Already exists: {out}")
    assert seeds > 0 and counts and set(counts) <= {1, 3, 5, 10} and len(set(counts)) == len(counts)
    swift = json.loads((ROOT / "docs/swift-stress-plan.json").read_text(encoding="utf-8"))["sources"]["crates/bots/src/swift.rs"]
    assert sha(ROOT / "crates/bots/src/swift.rs") == swift
    base = tomllib.loads((ROOT / "scenarios/baseline-v5.toml").read_text(encoding="utf-8"))
    out.mkdir(parents=True)
    binary = out / "hunt-eval-frozen.exe"
    shutil.copy2(ROOT / "target/release/hunt-eval.exe", binary)
    jobs = []
    for n in counts:
        for mode, multiple in [(None, None)] + [(mode, m) for m in [10, 100] for mode in ["cash", "relay"]]:
            key = f"n{n}-{mode or 'alone'}" + (f"-{multiple}" if mode else "")
            for order in (["predator_first", "target_first"] if mode else ["alone"]):
                cfg = copy.deepcopy(base)
                swifts = [{"bot": "swift", "owner": "target" if i == 0 else f"swift-extra-{i}",
                           "count": 1, "cash": 2_000_000, "shares_each": 20} for i in range(n)]
                predator = {"bot": "falcon", "owner": "predator", "count": 1,
                            "cash": 20_000_000, "shares_each": 200,
                            "params": {"mode": 6.0 if mode == "relay" else 4.0}}
                cfg["bots"] += swifts if not mode else ([predator] + swifts if order == "predator_first" else swifts + [predator])
                path = out / f"{key}-{order}.toml"
                path.write_text(toml_text(cfg), encoding="utf-8")
                jobs.append({"id": f"{key}-{order}", "case": key, "mode": mode,
                             "multiple": multiple, "allocation": "cash_active" if mode else None,
                             "swift_count": n, "order": order, "first_seed": first, "seeds": seeds,
                             "scenario": path.relative_to(ROOT).as_posix(), "scenario_sha256": sha(path)})
    sources = source_manifest()
    for path in [Path(__file__), ROOT / "tools/swift_capital.py", ROOT / "tools/swift_stress.py",
                 ROOT / "tools/swift_stress_report.py", ROOT / "docs/swift-crowding-hunt-protocol.md"]:
        sources[path.relative_to(ROOT).as_posix()] = sha(path)
    plan = {"stage": stage, "created_utc": datetime.now(timezone.utc).isoformat(),
            "binary": binary.relative_to(ROOT).as_posix(), "binary_sha256": sha(binary),
            "sources": sources, "swift_sha256": swift, "jobs": jobs,
            "notes": ["Hunter wealth is relative to ONE Swift, fixed across group sizes.",
                      "All targets are unchanged standard Swift accounts.",
                      "Hunter has 200 shares per stock; remaining wealth is cash.",
                      "Two orders are clustered by seed, not independent worlds."]}
    (out / "plan.json").write_text(json.dumps(plan, ensure_ascii=False, indent=2)+"\n", encoding="utf-8")
    print(f"Prepared {stage}: {len(jobs)*seeds} runs", flush=True)


def read_crowding_world(path, job, seed):
    row = read_capital_world(path, job, seed)
    summary = json.loads((path / "summary.json").read_text(encoding="utf-8"))
    accounts, curves = [], []
    for i in range(job["swift_count"]):
        owner = "target" if i == 0 else f"swift-extra-{i}"
        a = next(a for a in summary["accounts"] if a["owner"] == owner)
        assert a["initial_cash"] == 2_000_000 and all(q == 20 for q in a["initial_shares"])
        assert a["initial_equity"] == row["target"]["initial_equity"]
        assert a["stats"]["decisions"] == 3000
        with (path / f"{owner}-equity.csv").open() as f:
            trace = list(csv.DictReader(f))
        assert len(trace) == 3000 and [int(r["tick"]) for r in trace] == list(range(3000))
        curve = [a["initial_equity"]] + [int(r["equity"]) for r in trace] + [a["equity"]]
        curves.append(curve)
        accounts.append({"owner": owner, "position": i, "account": a["account"],
                         "initial_equity": a["initial_equity"], "final_equity": a["equity"],
                         "return_pct": (a["return_ratio"]-1)*100,
                         "pnl_yuan": (a["equity"]-a["initial_equity"])/100,
                         "drawdown_pct": drawdown(curve), "stats": a["stats"],
                         "counterparty_notional": a["notional_by_counterparty"]})
    row["swifts"] = accounts
    row["group"] = group_metrics(accounts, row["predator"])
    row["group"]["drawdown_pct"] = drawdown([sum(point) for point in zip(*curves)])
    row["swift_count"] = job["swift_count"]
    row["quality_checks"] = summary["quality"]["checks"]
    if row["predator"]:
        a = next(a for a in summary["accounts"] if a["owner"] == "predator")
        assert all(q == 200 for q in a["initial_shares"])
        row["predator_initial_stock_share"] = sum(a["initial_shares"]) / sum(sum(a["initial_shares"]) for a in summary["accounts"])
        row["direct_predator_swift_notional"] = sum(row["predator"]["counterparty_notional"].get(a["owner"], 0) for a in accounts)
    return row


def run(stage):
    out = BASE / stage
    if (out / "STOPPED.json").exists():
        raise SystemExit("Stopped by user; this stage cannot be resumed. Preserve it and use a newly authorized stage.")
    plan = json.loads((out / "plan.json").read_text(encoding="utf-8"))
    verify(plan)
    for i, job in enumerate(plan["jobs"]):
        dest = out / job["id"]
        if (dest / "done.json").exists():
            continue
        if dest.exists():
            raise SystemExit(f"Partial job requires inspection: {dest}")
        cmd = [str(ROOT / plan["binary"]), str(ROOT / job["scenario"]), "--out", str(dest),
               "--first-seed", str(job["first_seed"]), "--seeds", str(job["seeds"]),
               "--swift-count", str(job["swift_count"])]
        if job["mode"]:
            cmd += ["--capital-multiple", str(job["multiple"]), "--allocation", "cash_active"]
        print(f"START {stage} {i+1}/{len(plan['jobs'])} {job['id']}", flush=True)
        with (out / f"{job['id']}.log").open("w", encoding="utf-8") as log:
            subprocess.run(cmd, stdout=log, stderr=subprocess.STDOUT, cwd=ROOT, check=True)
        rows = [read_crowding_world(dest / f"seed-{s}", job, s)
                for s in range(job["first_seed"], job["first_seed"]+job["seeds"])]
        result = {"job": job, "worlds": rows}
        (dest / "done.json").write_text(json.dumps(result, ensure_ascii=False, indent=2)+"\n", encoding="utf-8")
        print("DONE", job["id"], "Swift group", round(stats.mean(r["group"]["return_pct"] for r in rows), 2),
              "hunter", round(stats.mean(r["predator"]["return_pct"] for r in rows), 2) if job["mode"] else None,
              "wins", sum(r["group"].get("predator_beats_group", False) for r in rows), flush=True)
    verify(plan)


if __name__ == "__main__":
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("command", choices=["prepare", "run"])
    p.add_argument("--stage", required=True)
    p.add_argument("--first-seed", type=int)
    p.add_argument("--seeds", type=int)
    p.add_argument("--counts", nargs="+", type=int, default=[1, 3, 5, 10])
    a = p.parse_args()
    if a.command == "prepare":
        if a.first_seed is None or not a.seeds or a.seeds < 1:
            p.error("prepare requires --first-seed and positive --seeds")
        prepare(a.stage, a.first_seed, a.seeds, a.counts)
    else:
        run(a.stage)
