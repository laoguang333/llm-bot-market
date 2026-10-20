"""Run an isolated CLI challenge and print compact challenger diagnostics.

Example: python tools/bench_bot.py coral-train --bot coral --first-seed 1
Full reports and the exact command are retained under runs/bench/NAME.
Use --scenario scenarios/baseline-v4.toml to replay the archived v4 benchmark.
"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import statistics
import subprocess


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("name")
    parser.add_argument("--bot", default="coral")
    parser.add_argument("--params", default="")
    parser.add_argument("--first-seed", type=int, default=1)
    parser.add_argument("--seeds", type=int, default=20)
    parser.add_argument("--threads", type=int)
    parser.add_argument("--cash", type=int, help="challenger cash in cents")
    parser.add_argument("--shares", type=int, help="challenger shares of every stock")
    parser.add_argument("--news-multiplier", type=float, default=1.0)
    parser.add_argument("--scenario", default="scenarios/baseline.toml",
                        help="repository-relative scenario path")
    args = parser.parse_args()
    if os.name == "nt":
        from build_release import windows_build_budget
        windows_build_budget()
    root = Path(__file__).resolve().parents[1]
    scenario = root / args.scenario
    out = root / "runs" / "bench" / args.name
    out.parent.mkdir(parents=True, exist_ok=True)
    command = [str(root / "target/release/llm-market.exe"), "challenge",
               str(scenario), "--bot", args.bot, "--out", str(out),
               "--seeds", str(args.seeds), "--first-seed", str(args.first_seed)]
    if args.params:
        command += ["--params", args.params]
    if args.threads is not None:
        command += ["--threads", str(args.threads)]
    if args.cash is not None:
        command += ["--cash", str(args.cash)]
    if args.shares is not None:
        command += ["--shares", str(args.shares)]
    if args.news_multiplier != 1.0:
        command += ["--news-multiplier", str(args.news_multiplier)]
    if out.exists():
        parser.error(f"output already exists: {out}")
    sources = [root / "Cargo.toml", root / "Cargo.lock", scenario]
    sources += sorted((root / "crates").rglob("*.rs"))
    sources += sorted((root / "crates").rglob("Cargo.toml"))
    manifest = {str(p.relative_to(root)): hashlib.sha256(p.read_bytes()).hexdigest() for p in sources}
    manifest["binary_sha256"] = hashlib.sha256(Path(command[0]).read_bytes()).hexdigest()
    with out.with_suffix(".log").open("w", encoding="utf-8") as log:
        subprocess.run(command, cwd=root, stdout=log, stderr=subprocess.STDOUT, check=True)
    (out / "command.json").write_text(json.dumps(command, indent=2), encoding="utf-8")
    (out / "manifest.json").write_text(json.dumps(manifest, indent=2), encoding="utf-8")
    score = json.loads((out / "challenge.json").read_text(encoding="utf-8"))
    accounts = []
    all_skipped = 0
    all_disabled = 0
    wall_seconds = 0.0
    for seed in score["seeds"]:
        result = json.loads((out / f"seed-{seed}" / "summary.json").read_text(encoding="utf-8"))
        challenger = next(a for a in result["accounts"] if a["owner"] == "challenger")
        if challenger.get("news_multiplier", 1.0) != args.news_multiplier:
            raise RuntimeError("binary did not apply the requested news multiplier; discard this run")
        accounts.append(challenger)
        all_skipped += sum(a["stats"]["skipped"] for a in result["accounts"])
        all_disabled += sum(bool(a["disabled"]) for a in result["accounts"])
        wall_seconds += result["wall_seconds"]
    diagnostics = {k: round(statistics.mean(a["stats"][k] for a in accounts), 2)
                   for k in ["skipped", "decisions", "rejects", "trade_fees", "idle_fees",
                             "interest", "dividends", "combo_dividends", "liquidations", "bot_errors"]}
    diagnostics["decision_us"] = round(statistics.mean(
        a["stats"]["decision_nanos_total"] / max(a["stats"]["decisions"], 1) / 1000
        for a in accounts), 2)
    compact = {"name": args.name, "mean": round(score["mean"], 3),
                      "sd": round(score["sd"], 3), "top25": score["top25_share"],
                      "pass": score["pass"], "percentiles": [round(x, 1) for x in score["percentiles"]],
                      "diagnostics": diagnostics, "all_accounts_skipped": all_skipped,
                      "all_accounts_disabled": all_disabled, "wall_seconds": round(wall_seconds, 2)}
    (out / "diagnostics.json").write_text(json.dumps(compact, indent=2), encoding="utf-8")
    print(json.dumps(compact, ensure_ascii=False), flush=True)


if __name__ == "__main__":
    main()
