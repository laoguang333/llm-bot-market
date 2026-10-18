"""Single Swift versus a public-data predator. Development and frozen validation.

All scenarios have one unchanged Swift; order reversal and same-wealth controls.
The report sees post-game identities, while bots never see the trade CSV.
"""
import argparse
import copy
import csv
from datetime import datetime, timezone
import hashlib
import json
from pathlib import Path
import statistics as stats
import subprocess
import tomllib

from swift_stress import ROOT, sha, toml_text
from swift_stress_report import drawdown, mean_ci

BASE = ROOT / "runs/swift-hunt-20261018"
DEV = [
    ("pursuit", "falcon", {"mode": 0.0}),
    ("reverse", "falcon", {"mode": 1.0}),
    ("supply", "falcon", {"mode": 2.0}),
    ("pressure", "falcon", {"mode": 3.0}),
    ("cash", "falcon", {"mode": 4.0}),
    ("blind", "falcon", {"mode": 5.0}),
    ("coral", "coral", {}),
]


def source_manifest():
    files = list((ROOT / "crates").rglob("*.rs")) + list((ROOT / "crates").rglob("Cargo.toml"))
    files += [ROOT / "Cargo.toml", ROOT / "Cargo.lock", Path(__file__)]
    return {p.relative_to(ROOT).as_posix(): sha(p) for p in sorted(files)}


def prepare(stage, first, seeds, candidates):
    out = BASE / stage
    if out.exists():
        raise SystemExit(f"Stage already exists: {out}")
    frozen_swift = json.loads((ROOT / "docs/swift-stress-plan.json").read_text(encoding="utf-8"))["sources"]["crates/bots/src/swift.rs"]
    assert sha(ROOT / "crates/bots/src/swift.rs") == frozen_swift
    out.mkdir(parents=True)
    base = tomllib.loads((ROOT / "scenarios/baseline.toml").read_text(encoding="utf-8"))
    jobs = []
    for key, bot, params in [("alone", None, {})] + candidates:
        for order in (["alone"] if bot is None else ["predator_first", "target_first"]):
            cfg = copy.deepcopy(base)
            target = {"bot": "swift", "owner": "target", "count": 1, "cash": 2_000_000, "shares_each": 20}
            predator = {"bot": bot, "owner": "predator", "count": 1, "cash": 20_000_000, "shares_each": 200, "params": params}
            cfg["bots"] += [target] if bot is None else ([predator, target] if order == "predator_first" else [target, predator])
            path = out / f"{key}-{order}.toml"
            path.write_text(toml_text(cfg), encoding="utf-8")
            jobs.append({"id": f"{key}-{order}", "case": key, "order": order, "first_seed": first,
                         "seeds": seeds, "scenario": str(path.relative_to(ROOT)), "scenario_sha256": sha(path), "config": cfg})
    binary = ROOT / "target/release/hunt-eval.exe"
    plan = {"stage": stage, "created_utc": datetime.now(timezone.utc).isoformat(),
            "binary": str(binary.relative_to(ROOT)), "binary_sha256": sha(binary),
            "sources": source_manifest(), "swift_sha256": frozen_swift, "jobs": jobs,
            "note": "One fixed Swift. Predator sees only anonymous public market data and its own account. Post-game analysis may use identities. First/second order is tested separately."}
    (out / "plan.json").write_text(json.dumps(plan, ensure_ascii=False, indent=2)+"\n", encoding="utf-8")
    print(f"Prepared {stage}: {len(jobs)*seeds} runs", flush=True)


def read_world(path, job, seed):
    data = json.loads((path / "summary.json").read_text())
    def read_account(owner):
        a = next((a for a in data["accounts"] if a["owner"] == owner), None)
        if a is None:
            return None
        with (path / f"{owner}-equity.csv").open() as f:
            trace = [{k: int(v) for k,v in row.items()} for row in csv.DictReader(f)]
        assert len(trace) == 3000 and a["stats"]["decisions"] == 3000
        eq = a["initial_equity"]
        return {"account": a["account"], "initial_equity": eq, "final_equity": a["equity"],
                "return_pct": (a["return_ratio"]-1)*100, "pnl_yuan": (a["equity"]-eq)/100,
                "percentile": a["percentile"],
                "drawdown_pct": drawdown([eq]+[r["equity"] for r in trace]+[a["equity"]]),
                "mean_gross_leverage": stats.mean(r["gross"]/max(1,r["equity"]) for r in trace),
                "turnover": a["stats"]["volume_notional"]/eq,
                "dividend_pct": (a["stats"]["dividends"]+a["stats"]["combo_dividends"])/eq*100,
                "stats": a["stats"], "counterparty_notional": a["notional_by_counterparty"]}
    target, predator = read_account("target"), read_account("predator")
    if predator:
        assert predator["initial_equity"] == target["initial_equity"]*10
    return {"seed": seed, "case": job["case"], "order": job["order"], "target": target, "predator": predator,
            "audits": data["audits"], "quality_pass": data["quality"]["pass"],
            "all_skipped": sum(a["stats"]["skipped"] for a in data["accounts"]),
            "all_errors": sum(a["stats"]["bot_errors"] for a in data["accounts"]),
            "all_disabled": sum(bool(a["disabled"]) for a in data["accounts"])}


def summarize(rows):
    result = {"runs": len(rows)}
    for who in ["target", "predator"]:
        these = [r[who] for r in rows if r[who]]
        if not these:
            continue
        result[who] = {"mean_return_pct": stats.mean(r["return_pct"] for r in these),
                       "median_return_pct": stats.median(r["return_pct"] for r in these),
                       "losses": sum(r["return_pct"] < 0 for r in these),
                       "mean_drawdown_pct": stats.mean(r["drawdown_pct"] for r in these),
                       "worst_drawdown_pct": max(r["drawdown_pct"] for r in these),
                       "mean_pnl_yuan": stats.mean(r["pnl_yuan"] for r in these)}
    paired = [r for r in rows if r["predator"]]
    if paired:
        diff = [r["predator"]["return_pct"]-r["target"]["return_pct"] for r in paired]
        result.update({"mean_advantage_pp": stats.mean(diff), "median_advantage_pp": stats.median(diff),
                       "wins": sum(d > 0 for d in diff),
                       "both_lose": sum(r["target"]["return_pct"]<0 and r["predator"]["return_pct"]<0 for r in paired),
                       "both_lose_predator_wins": sum(r["target"]["return_pct"]<0 and r["predator"]["return_pct"]<0 and r["predator"]["return_pct"]>r["target"]["return_pct"] for r in paired)})
    return result


def run(stage):
    out = BASE / stage
    plan = json.loads((out / "plan.json").read_text(encoding="utf-8"))
    assert sha(ROOT / plan["binary"]) == plan["binary_sha256"]
    for rel, digest in plan["sources"].items():
        assert sha(ROOT / rel) == digest, rel
    for job in plan["jobs"]:
        dest = out / job["id"]
        assert sha(ROOT / job["scenario"]) == job["scenario_sha256"]
        if (dest / "done.json").exists():
            continue
        if dest.exists():
            raise SystemExit(f"Incomplete job: {dest}")
        cmd = [str(ROOT / plan["binary"]), str(ROOT / job["scenario"]), "--out", str(dest),
               "--first-seed", str(job["first_seed"]), "--seeds", str(job["seeds"])]
        print("START", stage, job["id"], flush=True)
        with (out / (job["id"]+".log")).open("w", encoding="utf-8") as log:
            subprocess.run(cmd, stdout=log, stderr=subprocess.STDOUT, cwd=ROOT, check=True)
        rows = [read_world(dest / f"seed-{seed}", job, seed) for seed in range(job["first_seed"],job["first_seed"]+job["seeds"])]
        result = {"job": job, "worlds": rows, "summary": summarize(rows)}
        (dest / "done.json").write_text(json.dumps(result, ensure_ascii=False, indent=2)+"\n", encoding="utf-8")
        print("DONE", job["id"], json.dumps(result["summary"], ensure_ascii=False), flush=True)


if __name__ == "__main__":
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("mode", choices=["prepare-dev", "run", "prepare-validation"])
    p.add_argument("--stage", default="dev")
    p.add_argument("--first-seed", type=int, default=10001)
    p.add_argument("--seeds", type=int, default=8)
    p.add_argument("--candidate", choices=[r[0] for r in DEV], default="pursuit")
    a = p.parse_args()
    if a.mode == "run": run(a.stage)
    elif a.mode == "prepare-dev": prepare(a.stage,a.first_seed,a.seeds,DEV)
    else: prepare(a.stage,a.first_seed,a.seeds,[r for r in DEV if r[0] in [a.candidate,"cash","blind","coral"]])
