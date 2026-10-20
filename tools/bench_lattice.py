"""Reproduce one serial Lattice challenge against archived v15.

Use through arena_gate.py. This script does not inspect other bots' source.
"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import statistics
import subprocess
import time


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("name")
    parser.add_argument("--first-seed", type=int, required=True)
    parser.add_argument("--seeds", type=int, default=20)
    parser.add_argument("--params", default="")
    parser.add_argument("--threads", type=int, default=4)
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    if os.name == "nt":
        from build_release import windows_build_budget
        windows_build_budget()
    out = root / "runs" / "lattice" / args.name
    if out.exists():
        parser.error(f"Refusing to overwrite {out}")
    out.parent.mkdir(parents=True, exist_ok=True)
    binary = root / "target" / "release" / ("llm-market.exe" if os.name == "nt" else "llm-market")
    paths = [root / "crates/bots/src/lattice.rs", root / "scenarios/baseline-v15.toml",
             root / "Cargo.lock", binary]
    manifest = {
        "git_head": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=root, text=True).strip(),
        "sha256": {p.relative_to(root).as_posix(): hashlib.sha256(p.read_bytes()).hexdigest() for p in paths},
        "threads": args.threads, "repetitions": 1, "params": args.params,
        "scenario": "scenarios/baseline-v15.toml",
        "seeds": list(range(args.first_seed, args.first_seed + args.seeds)),
    }
    command = [str(binary), "challenge", "scenarios/baseline-v15.toml", "--bot", "lattice",
               "--out", str(out), "--first-seed", str(args.first_seed),
               "--seeds", str(args.seeds), "--threads", str(args.threads)]
    if args.params:
        command += ["--params", args.params]
    # Freeze evidence before execution, including failed or interrupted runs.
    out.with_suffix(".manifest.json").write_text(json.dumps(manifest, indent=2), encoding="utf-8")
    out.with_suffix(".source.rs").write_bytes(paths[0].read_bytes())
    started = time.perf_counter()
    with out.with_suffix(".log").open("w", encoding="utf-8") as log:
        subprocess.run(command, cwd=root, stdout=log, stderr=subprocess.STDOUT, check=True)
    manifest["elapsed_seconds"] = time.perf_counter() - started
    manifest["command"] = command
    rows = []
    score = json.loads((out / "challenge.json").read_text(encoding="utf-8"))
    for seed in score["seeds"]:
        result = json.loads((out / f"seed-{seed}" / "summary.json").read_text(encoding="utf-8"))
        account = next(a for a in result["accounts"] if a["owner"] == "challenger")
        assert account["news_multiplier"] == 1.0
        assert account["forecast_access"]["horizon"] == 0
        assert account["initial_cash"] == 2_000_000
        assert account["initial_shares"] == [20] * len(account["initial_shares"])
        rows.append({
            "seed": seed, "percentile": account["percentile"], "rank": account["rank"],
            "return_ratio": account["return_ratio"], "stats": account["stats"],
            "disabled": account["disabled"], "wall_seconds": result["wall_seconds"],
            "audits": result["audits"], "quality": result["quality"],
            "all_skipped": sum(a["stats"]["skipped"] for a in result["accounts"]),
            "all_disabled": sum(bool(a["disabled"]) for a in result["accounts"]),
            "all_errors": sum(a["stats"]["bot_errors"] for a in result["accounts"]),
        })
    report = {"manifest": manifest, "score": score, "worlds": rows}
    (out / "evidence.json").write_text(json.dumps(report, indent=2), encoding="utf-8")
    compact = {
        "name": args.name, "mean": score["mean"], "sd": score["sd"],
        "top25": score["top25_share"], "pass": score["pass"],
        "mean_return": statistics.mean(r["return_ratio"] for r in rows),
        "worst_percentile": min(r["percentile"] for r in rows),
        "wins": sum(r["rank"] == 1 for r in rows),
        "elapsed_seconds": manifest["elapsed_seconds"],
        "sum_stats": {k: sum(r["stats"][k] for r in rows) for k in
                      ["liquidations", "rejects", "bot_errors", "skipped"]},
    }
    print(json.dumps(compact, ensure_ascii=False), flush=True)


if __name__ == "__main__":
    main()
