"""Frozen capital experiment: one Swift, one anonymous public-data opponent.

Every world runs serially under arena_gate. Resume skips only verified completed
jobs; partial jobs are not silently overwritten or selectively re-run.
"""
import argparse
import copy
from datetime import datetime, timezone
import json
from pathlib import Path
import shutil
import subprocess
import sys
import tomllib

from swift_stress import ROOT, sha, toml_text
from swift_hunt import read_world, summarize, source_manifest

BASE = ROOT / "runs/swift-capital-20261018"
MODES = {"clock": 7, "relay": 6, "pursuit": 0, "cash": 4, "blind": 5}
STRUCTURES = [("base", 10)] + [(a, m) for m in [30, 100, 300] for a in ["reserve", "scaled"]]


def cases():
    return [(f"{mode}-{allocation}-{multiple}", mode, allocation, multiple)
            for allocation, multiple in STRUCTURES for mode in MODES]


def available_cases():
    return cases() + [(f"relay-cash_active-{m}", "relay", "cash_active", m) for m in [100, 300]]


def prepare(stage, first, seeds, selected=None):
    out = BASE / stage
    if out.exists():
        raise SystemExit(f"Already exists: {out}")
    swift = json.loads((ROOT / "docs/swift-stress-plan.json").read_text(encoding="utf-8"))["sources"]["crates/bots/src/swift.rs"]
    assert sha(ROOT / "crates/bots/src/swift.rs") == swift
    out.mkdir(parents=True)
    binary = out / "hunt-eval-frozen.exe"
    shutil.copy2(ROOT / "target/release/hunt-eval.exe", binary)
    config = tomllib.loads((ROOT / "scenarios/baseline-v5.toml").read_text(encoding="utf-8"))
    jobs = []
    treatments = [("alone", None, None, None)] + [c for c in (available_cases() if selected else cases()) if selected is None or c[0] in selected]
    if selected:
        assert set(selected) <= {c[0] for c in available_cases()}
    for key, mode, allocation, multiple in treatments:
        for order in (["alone"] if mode is None else ["predator_first", "target_first"]):
            cfg = copy.deepcopy(config)
            target = {"bot": "swift", "owner": "target", "count": 1, "cash": 2_000_000, "shares_each": 20}
            predator = {"bot": "falcon", "owner": "predator", "count": 1, "cash": 20_000_000,
                        "shares_each": 200, "params": {"mode": float(MODES[mode])}} if mode else None
            cfg["bots"] += [target] if mode is None else ([predator, target] if order == "predator_first" else [target, predator])
            path = out / f"{key}-{order}.toml"
            path.write_text(toml_text(cfg), encoding="utf-8")
            jobs.append({"id": f"{key}-{order}", "case": key, "mode": mode, "allocation": allocation,
                         "multiple": multiple, "order": order, "first_seed": first, "seeds": seeds,
                         "scenario": path.relative_to(ROOT).as_posix(), "scenario_sha256": sha(path), "config": cfg})
    sources = source_manifest()
    for path in [Path(__file__), ROOT / "tools/swift_hunt.py", ROOT / "tools/swift_stress.py"]:
        sources[path.relative_to(ROOT).as_posix()] = sha(path)
    plan = {"stage": stage, "created_utc": datetime.now(timezone.utc).isoformat(),
            "binary": binary.relative_to(ROOT).as_posix(), "binary_sha256": sha(binary),
            "sources": sources, "swift_sha256": swift, "jobs": jobs,
            "notes": ["Capital setup uses public opening prices only, with all fees unchanged.",
                      "Reserve keeps 200 shares/stock and a fixed 10x initial-equity trading allocation.",
                      "Scaled increases both starting shares and fixed allocation in proportion to total wealth.",
                      "10x treatments coincide and are measured once; no duplicated evidence."]}
    (out / "plan.json").write_text(json.dumps(plan, ensure_ascii=False, indent=2)+"\n", encoding="utf-8")
    print(f"Prepared {stage}: {len(jobs)*seeds} runs", flush=True)


def verify(plan):
    assert sha(ROOT / plan["binary"]) == plan["binary_sha256"]
    for rel, digest in plan["sources"].items():
        assert sha(ROOT / rel) == digest, rel
    for job in plan["jobs"]:
        assert sha(ROOT / job["scenario"]) == job["scenario_sha256"]


def read_capital_world(path, job, seed):
    row = read_world(path, job, seed)
    summary = json.loads((path / "summary.json").read_text(encoding="utf-8"))
    row.update({"mode": job["mode"], "allocation": job["allocation"], "multiple": job["multiple"],
                "wall_seconds": summary["wall_seconds"], "trades": summary["trades"]})
    if job["mode"]:
        setup = json.loads((path / "capital-setup.json").read_text(encoding="utf-8"))
        assert setup["predator_initial_equity"] == row["predator"]["initial_equity"]
        assert setup["target_initial_equity"] == row["target"]["initial_equity"]
        a = next(a for a in summary["accounts"] if a["owner"] == "predator")
        assert a["initial_cash"] == setup["predator_initial_cash"]
        assert all(q == setup["predator_shares_each"] for q in a["initial_shares"])
        row["capital_setup"] = setup
    assert row["all_skipped"] == row["all_errors"] == row["all_disabled"] == 0
    assert row["audits"] == 6
    return row


def run(stage):
    out = BASE / stage
    plan = json.loads((out / "plan.json").read_text(encoding="utf-8"))
    verify(plan)
    for i, job in enumerate(plan["jobs"]):
        dest = out / job["id"]
        if (dest / "done.json").exists():
            continue
        if dest.exists():
            raise SystemExit(f"Partial job requires inspection: {dest}")
        cmd = [str(ROOT / plan["binary"]), str(ROOT / job["scenario"]), "--out", str(dest),
               "--first-seed", str(job["first_seed"]), "--seeds", str(job["seeds"])]
        if job["mode"]:
            cmd += ["--capital-multiple", str(job["multiple"]), "--allocation",
                    "scaled" if job["allocation"] == "base" else job["allocation"]]
        print(f"START {stage} {i+1}/{len(plan['jobs'])} {job['id']}", flush=True)
        with (out / f"{job['id']}.log").open("w", encoding="utf-8") as log:
            subprocess.run(cmd, stdout=log, stderr=subprocess.STDOUT, cwd=ROOT, check=True)
        rows = [read_capital_world(dest / f"seed-{s}", job, s)
                for s in range(job["first_seed"], job["first_seed"]+job["seeds"])]
        result = {"job": job, "worlds": rows, "summary": summarize(rows)}
        (dest / "done.json").write_text(json.dumps(result, ensure_ascii=False, indent=2)+"\n", encoding="utf-8")
        x = result["summary"]
        print("DONE", job["id"], "Swift", round(x["target"]["mean_return_pct"], 2),
              "predator", round(x.get("predator", {}).get("mean_return_pct", 0), 2), "wins", x.get("wins"), flush=True)
    verify(plan)


if __name__ == "__main__":
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("command", choices=["prepare", "run"])
    p.add_argument("--stage", required=True)
    p.add_argument("--first-seed", type=int)
    p.add_argument("--seeds", type=int)
    p.add_argument("--cases", nargs="+")
    a = p.parse_args()
    if a.command == "prepare":
        if a.first_seed is None or not a.seeds or a.seeds < 1:
            p.error("prepare requires --first-seed and positive --seeds")
        prepare(a.stage, a.first_seed, a.seeds, a.cases)
    else:
        run(a.stage)
