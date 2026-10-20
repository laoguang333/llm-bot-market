"""Fixed v6 acceptance, reviewed bots and one shared Windows CPU budget.

Invoke through arena_gate.py. No tuning or automatic restart of old studies.
"""
import argparse
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import shutil
import statistics as stats
import subprocess
import sys
import time
import tomllib
import zipfile

from build_release import windows_build_budget

ROOT = Path(__file__).resolve().parents[1]
DEFAULT_OUT = ROOT / "runs/baseline-v6-20261019"
PROFILES = {"swift-standard", "swift-early", "swift-selective", "swift-flow", "swift-depth"}


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def write(path, value):
    path.write_text(json.dumps(value, ensure_ascii=False, indent=2)+"\n", encoding="utf-8")


def sources():
    paths = sorted((ROOT / "crates").rglob("*.rs")) + sorted((ROOT / "crates").rglob("Cargo.toml"))
    paths += [ROOT / p for p in ["Cargo.toml", "Cargo.lock", "scenarios/baseline-v5.toml",
              "scenarios/baseline-v6-five-swift.toml", "docs/baseline-v6-protocol.md", "docs/baseline-v6-bot-review.md"]]
    return {p.relative_to(ROOT).as_posix(): sha(p) for p in paths}


def tool_sources():
    return {p: sha(ROOT / p) for p in ["tools/validate_baseline_v6.py", "tools/build_release.py", "tools/arena_gate.py"]}


def check_configs():
    v5 = tomllib.loads((ROOT / "scenarios/baseline-v5.toml").read_text(encoding="utf-8"))
    v6 = tomllib.loads((ROOT / "scenarios/baseline-v6-five-swift.toml").read_text(encoding="utf-8"))
    assert v6["bots"][:-5] == v5["bots"]
    assert {k:v for k,v in v6.items() if k != "bots"} == {k:v for k,v in v5.items() if k != "bots"}
    targets = v6["bots"][-5:]
    assert {b["owner"] for b in targets} == PROFILES
    assert len({tuple(sorted(b["params"].items())) for b in targets}) == 5
    assert all(b["bot"] == "swift" and b["count"] == 1 and b["cash"] == 2_000_000
               and b["shares_each"] == 20 and "count_max" not in b for b in targets)


def checked(command, out, label, env):
    start = time.perf_counter()
    print("START", label, flush=True)
    with (out / f"{label}.log").open("w", encoding="utf-8") as log:
        result = subprocess.run(command, cwd=ROOT, env=env, stdout=log, stderr=subprocess.STDOUT)
    seconds = time.perf_counter() - start
    write(out / f"{label}.command.json", {"command": command, "wall_seconds": seconds, "exit_code": result.returncode})
    if result.returncode:
        print((out / f"{label}.log").read_text(encoding="utf-8")[-16000:], flush=True)
        raise SystemExit(f"Failed {label}; preserve this output before any repair")
    print(f"PASS {label} ({seconds:.2f}s)", flush=True)
    return seconds


def checks(out, budget, env):
    out.mkdir(parents=True, exist_ok=True)
    check_configs()
    before = sources()
    commands = [
        ("fmt", ["cargo", "fmt", "--all", "--", "--check"]),
        ("clippy", ["cargo", "clippy", "--workspace", "--all-targets", "--jobs", str(budget), "--", "-D", "warnings"]),
        ("tests", ["cargo", "test", "--workspace", "--jobs", str(budget), "--", "--test-threads=1"]),
        ("python-tests", [sys.executable, "-m", "unittest", "discover", "-s", "tools", "-p", "test_*.py"]),
        ("release", ["cargo", "build", "--release", "--bins", "--jobs", str(budget)]),
    ]
    runs = {label: checked(command, out, label, env) for label, command in commands}
    assert sources() == before, "Sources changed during checks; re-review before running"
    write(out / "checks.json", {"sources": before, "cpu_budget": budget, "wall_seconds": runs})


def prepare(out, budget):
    if (out / "plan.json").exists():
        raise SystemExit("Plan already exists; do not replace frozen inputs")
    check_configs()
    checked_sources = json.loads((out / "checks.json").read_text(encoding="utf-8"))["sources"]
    assert sources() == checked_sources
    binary = out / ("llm-market-frozen.exe" if os.name == "nt" else "llm-market-frozen")
    shutil.copy2(ROOT / "target/release" / binary.name.replace("-frozen", ""), binary)
    jobs = [{"id": f"{version}-{kind}", "version": version,
             "scenario": f"scenarios/baseline-{version}.toml", "kind": kind,
             "first_seed": 20001, "seeds": 20, "threads": 4, "repeats": 1}
            for kind in ["background", "swift-challenge"] for version in ["v5", "v6"]]
    frozen = sources()
    with zipfile.ZipFile(out / "sources.zip", "w", zipfile.ZIP_DEFLATED) as archive:
        for rel in frozen:
            archive.write(ROOT / rel, rel)
    write(out / "plan.json", {"created_utc": datetime.now(timezone.utc).isoformat(),
          "git_head": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(),
          "dirty_worktree": subprocess.check_output(["git", "status", "--porcelain"], cwd=ROOT, text=True).splitlines(),
          "sources": frozen, "tools": tool_sources(), "source_archive_sha256": sha(out / "sources.zip"),
          "binary": str(binary.relative_to(ROOT)), "binary_sha256": sha(binary),
          "logical_cpus": os.cpu_count(), "cpu_budget": budget, "jobs": jobs,
          "notes": ["Same current engine for v5 and v6, including pre-existing uncommitted fixes.",
                    "Source snapshot retained locally; Git HEAD alone does not identify this build.",
                    "Four workers; process affinity bounds all child processes to the shared Windows budget.",
                    "Every configuration/seed is run once; no return-based reruns or parameter fitting."]})


def verify(plan):
    assert sources() == plan["sources"], "Frozen sources changed"
    assert tool_sources() == plan["tools"], "Frozen runner changed"
    assert sha(ROOT / plan["binary"]) == plan["binary_sha256"]


def run(out, env):
    plan = json.loads((out / "plan.json").read_text(encoding="utf-8"))
    verify(plan)
    for job in plan["jobs"]:
        dest = out / job["id"]
        if dest.exists():
            raise SystemExit(f"Output exists; never silently rerun: {dest}")
        command = [str(ROOT / plan["binary"]), "series" if job["kind"] == "background" else "challenge",
                   str(ROOT / job["scenario"]), "--out", str(dest), "--first-seed", str(job["first_seed"]),
                   "--seeds", str(job["seeds"]), "--threads", str(job["threads"])]
        if job["kind"] != "background":
            command += ["--bot", "swift"]
        checked(command, out, job["id"], env)
    verify(plan)


def summarize_accounts(accounts):
    returns = [(a["return_ratio"]-1)*100 for a in accounts]
    return {"observations": len(accounts), "mean_percentile": stats.mean(a["percentile"] for a in accounts),
            "mean_return_pct": stats.mean(returns), "median_return_pct": stats.median(returns),
            "losses": sum(r < 0 for r in returns), "mean_volume_yuan": stats.mean(a["stats"]["volume_notional"]/100 for a in accounts),
            **{key: sum(a["stats"][key] for a in accounts) for key in ["rejects", "liquidations", "skipped", "bot_errors", "decisions"]},
            "deferred_idle_fee": sum(a["deferred_idle_fee"] for a in accounts)}


def report(out):
    plan = json.loads((out / "plan.json").read_text(encoding="utf-8"))
    batches, worlds = [], []
    for job in plan["jobs"]:
        batch = []
        for seed in range(job["first_seed"], job["first_seed"]+job["seeds"]):
            p = out / job["id"] / f"seed-{seed}" / "summary.json"
            w = json.loads(p.read_text(encoding="utf-8"))
            assert w["seed"] == seed and w["ticks"] == 3000 and w["threads"] == job["threads"] and w["audits"] == 6
            assert len({a["owner"] for a in w["accounts"]}) == len(w["accounts"])
            for a in w["accounts"]:
                assert a["stats"]["decisions"] == 3000 and a["stats"]["skipped"] == a["stats"]["bot_errors"] == 0
                assert not a["disabled"]
            swifts = [a for a in w["accounts"] if a["owner"] in PROFILES]
            assert {a["owner"] for a in swifts} == (PROFILES if job["version"] == "v6" else set())
            for a in swifts:
                assert a["initial_cash"] == 2_000_000 and all(q == 20 for q in a["initial_shares"])
                assert a["strategy"] == "swift"
            batch.append(w)
            worlds.append({"batch": job["id"], **w})
        accounts = [a for w in batch for a in w["accounts"]]
        checks = {}
        for name in [c["name"] for c in batch[0]["quality"]["checks"]]:
            values = [c for w in batch for c in w["quality"]["checks"] if c["name"] == name]
            finite = [c["value"] for c in values if c["value"] is not None]
            checks[name] = {"pass_count": sum(c["pass"] for c in values), "median": stats.median(finite) if finite else None,
                            "threshold": values[0]["threshold"], "at_least": values[0]["at_least"]}
        b = {**job, "runs": len(batch), "market_quality_passes": sum(w["quality"]["pass"] for w in batch),
             "quality_checks": checks, "engine_wall_seconds": sum(w["wall_seconds"] for w in batch),
             "command_wall_seconds": json.loads((out / f"{job['id']}.command.json").read_text())["wall_seconds"],
             "all_accounts": summarize_accounts(accounts),
             "by_strategy": {name: summarize_accounts([a for a in accounts if a["strategy"] == name]) for name in sorted({a["strategy"] for a in accounts})},
             "swift_profiles": {owner: summarize_accounts([a for a in accounts if a["owner"] == owner]) for owner in sorted(PROFILES) if any(a["owner"] == owner for a in accounts)}}
        if job["kind"] != "background":
            b["challenge"] = json.loads((out / job["id"] / "challenge.json").read_text(encoding="utf-8"))
            b["challenger"] = summarize_accounts([a for a in accounts if a["owner"] == "challenger"])
        batches.append(b)
    data = {"plan": plan, "runs": len(worlds), "batches": batches,
            "audits": sum(w["audits"] for w in worlds), "worlds": worlds}
    write(ROOT / "docs/baseline-v6-results.json", data)
    shutil.copy2(out / "plan.json", ROOT / "docs/baseline-v6-plan.json")
    for b in batches:
        print(b["id"], "quality", b["market_quality_passes"], "/", b["runs"],
              "Swift", b.get("challenge", {}).get("mean"), "pass", b.get("challenge", {}).get("pass"), flush=True)
    return data


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("command", choices=["checks", "prepare", "run", "report"])
    p.add_argument("--out", type=Path, default=DEFAULT_OUT)
    a = p.parse_args()
    out = a.out.resolve()
    budget = windows_build_budget() if os.name == "nt" else (os.cpu_count() or 1)
    print(f"CPU budget: {budget}; child processes share affinity", flush=True)
    env = dict(os.environ, RUSTC_WRAPPER="", CARGO_BUILD_JOBS=str(budget), RUST_TEST_THREADS="1")
    if a.command == "checks":
        checks(out, budget, env)
    elif a.command == "prepare":
        prepare(out, budget)
    elif a.command == "run":
        run(out, env)
    else:
        report(out)


if __name__ == "__main__":
    main()
