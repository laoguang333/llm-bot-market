"""Frozen per-tick dividend compatibility, performance and market study.

Run through tools/arena_gate.py. All child processes share the Windows half-CPU budget.
No parameter fitting; historical binaries/scenarios are never overwritten.
"""
import argparse
import copy
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import time
import tomllib
import zipfile

from build_release import windows_build_budget
from swift_stress import toml_text

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "runs/per-tick-20261019"


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def write(path, value):
    path.write_text(json.dumps(value, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")


def prepare(budget):
    if (OUT / "plan.json").exists():
        raise SystemExit("Plan already frozen")
    for regime in ["periodic", "per-tick"]:
        cfg = tomllib.loads((OUT / f"{regime}-v6.toml").read_text(encoding="utf-8"))
        cfg = copy.deepcopy(cfg)
        for layer in ["market", "sector", "stock"]:
            cfg["world"]["news"][layer]["events_per_1000_ticks"] = 0.0
        (OUT / f"{regime}-no-news.toml").write_text(toml_text(cfg), encoding="utf-8")
    binary = OUT / "after.exe"
    shutil.copy2(ROOT / "target/release/llm-market.exe", binary)
    files = sorted((ROOT / "crates").rglob("*.rs")) + sorted((ROOT / "crates").rglob("Cargo.toml"))
    files += [ROOT / p for p in ["Cargo.toml", "Cargo.lock", "tools/per_tick_study.py", "docs/per-tick-protocol.md"]]
    sources = {p.relative_to(ROOT).as_posix(): sha(p) for p in files}
    with zipfile.ZipFile(OUT / "after-sources.zip", "w", zipfile.ZIP_DEFLATED) as z:
        for rel in sources:
            z.write(ROOT / rel, rel)
    conditions = {
        "old-periodic": {"binary": "before.exe", "scenario": "periodic-v6.toml"},
        "new-periodic": {"binary": "after.exe", "scenario": "periodic-v6.toml"},
        "new-per-tick": {"binary": "after.exe", "scenario": "per-tick-v6.toml"},
    }
    performance = []
    for threads in [1, 4]:
        for name in conditions:
            performance.append(dict(condition=name, threads=threads, seed=22000, repeat=0, warmup=True))
    for repeat in range(3):
        for seed in range(22001, 22006):
            for threads in [1, 4]:
                order = list(conditions)
                rotation = (repeat + seed + threads) % 3
                for name in order[rotation:] + order[:rotation]:
                    performance.append(dict(condition=name, threads=threads, seed=seed, repeat=repeat+1, warmup=False))
    markets = []
    for family in ["v5", "v6"]:
        for bot in ["background", "swift", "news_v2"]:
            for regime in ["periodic", "per-tick"]:
                markets.append(dict(id=f"{family}-{bot}-{regime}", scenario=f"{regime}-{family}.toml", bot=bot))
    for regime in ["periodic", "per-tick"]:
        markets.append(dict(id=f"no-news-background-{regime}", scenario=f"{regime}-no-news.toml", bot="background"))
    plan = dict(created_utc=datetime.now(timezone.utc).isoformat(), cpu_budget=budget,
                sources=sources, scenarios={p.name:sha(p) for p in OUT.glob("*.toml")},
                binaries={name:sha(OUT/name) for name in ["before.exe", "after.exe"]},
                conditions=conditions, performance=performance, markets=markets,
                market_first_seed=23001, market_seeds=20, market_threads=4,
                ticks=3000, market_repeats=1, cash=2_000_000, shares=20,
                note="Existing dirty user engine changes are in both builds. Snapshots identify sources; no strategy tuning.")
    write(OUT / "plan.json", plan)
    shutil.copy2(OUT / "plan.json", ROOT / "docs/per-tick-plan.json")
    print("Frozen: 90 timed matches + 6 warmups, 280 market matches", flush=True)


def verify(plan):
    for name, digest in plan["binaries"].items():
        assert sha(OUT / name) == digest
    for name, digest in plan["scenarios"].items():
        assert sha(OUT / name) == digest
    for name, digest in plan["sources"].items():
        assert sha(ROOT / name) == digest, name


def command(cmd, dest):
    marker = dest.with_suffix(".done.json")
    if marker.exists():
        return json.loads(marker.read_text(encoding="utf-8"))
    if dest.exists():
        raise SystemExit(f"Incomplete output preserved; investigate {dest}")
    dest.parent.mkdir(parents=True, exist_ok=True)
    start = time.perf_counter()
    with dest.with_suffix(".log").open("w", encoding="utf-8") as log:
        proc = subprocess.run(cmd, cwd=ROOT, stdout=log, stderr=subprocess.STDOUT)
    record = dict(command=cmd, wall_seconds=time.perf_counter()-start, exit_code=proc.returncode)
    if proc.returncode:
        print(dest.with_suffix(".log").read_text(encoding="utf-8")[-6000:])
        raise SystemExit(proc.returncode)
    write(marker, record)
    return record


def run_performance(plan):
    for i, job in enumerate(plan["performance"]):
        dest = OUT / "performance" / f"{i:03}-{job['condition']}-t{job['threads']}-s{job['seed']}-r{job['repeat']}"
        c = plan["conditions"][job["condition"]]
        cmd = [str(OUT/c["binary"]), "run", str(OUT/c["scenario"]), "--out", str(dest),
               "--seed", str(job["seed"]), "--threads", str(job["threads"])]
        record = command(cmd, dest)
        result = json.loads((dest / "summary.json").read_text(encoding="utf-8"))
        print(f"PERF {i+1}/{len(plan['performance'])} {dest.name}: engine={result['wall_seconds']:.3f}s process={record['wall_seconds']:.3f}s", flush=True)


def run_markets(plan):
    for i, job in enumerate(plan["markets"]):
        dest = OUT / "market" / job["id"]
        cmd = [str(OUT / "after.exe"), "series" if job["bot"] == "background" else "challenge",
               str(OUT/job["scenario"]), "--out", str(dest), "--first-seed", str(plan["market_first_seed"]),
               "--seeds", str(plan["market_seeds"]), "--threads", str(plan["market_threads"])]
        if job["bot"] != "background":
            cmd += ["--bot", job["bot"], "--cash", str(plan["cash"]), "--shares", str(plan["shares"])]
        print(f"MARKET {i+1}/{len(plan['markets'])} START {job['id']}", flush=True)
        record = command(cmd, dest)
        print(f"MARKET DONE {job['id']}: {record['wall_seconds']:.2f}s", flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=["prepare", "performance", "markets"])
    args = parser.parse_args()
    budget = windows_build_budget() if os.name == "nt" else (os.cpu_count() or 1)
    print(f"Shared CPU budget: {budget}", flush=True)
    if args.command == "prepare":
        prepare(budget)
        return
    plan = json.loads((OUT / "plan.json").read_text(encoding="utf-8"))
    verify(plan)
    if args.command == "performance":
        run_performance(plan)
    else:
        run_markets(plan)
    verify(plan)


if __name__ == "__main__":
    main()
