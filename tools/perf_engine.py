"""Compare release executables with interleaved, sequential full-match runs.

Example: python tools/perf_engine.py --binary before=runs/before.exe
         --binary after=target/release/llm-market.exe --out runs/perf-comparison
"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import statistics
import subprocess
import time


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", action="append", required=True, metavar="LABEL=PATH")
    parser.add_argument("--scenario", default="scenarios/baseline.toml")
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--threads", type=int, nargs="+", default=[1, 4])
    parser.add_argument("--seeds", type=int, nargs="+", default=[7, 11, 19])
    parser.add_argument("--repeats", type=int, default=3)
    parser.add_argument("--ticks", type=int)
    args = parser.parse_args()
    cpu_count = os.cpu_count() or 1
    if args.repeats < 1 or any(t < 1 for t in args.threads):
        parser.error("repeats and threads must be positive")
    if os.name == "nt" and max(args.threads) > max(1, cpu_count // 2):
        parser.error("Windows benchmarks must share at most half the logical CPUs")
    binaries = []
    for item in args.binary:
        label, sep, path = item.partition("=")
        if not sep or not label or not all(c.isalnum() or c in "-_" for c in label):
            parser.error("--binary expects a simple LABEL=PATH")
        if any(label == prior[0] for prior in binaries):
            parser.error("binary labels must be unique")
        binaries.append((label, Path(path).resolve()))
    args.out.mkdir(parents=True, exist_ok=False)
    result = {
        "platform": platform.platform(),
        "logical_cpus": cpu_count,
        "scenario": str(Path(args.scenario).resolve()),
        "scenario_sha256": hashlib.sha256(Path(args.scenario).read_bytes()).hexdigest(),
        "binaries": {label: {"path": str(path), "sha256": hashlib.sha256(path.read_bytes()).hexdigest()}
                     for label, path in binaries},
        "runs": [],
    }
    # One unmeasured warmup per executable, also retained for inspection.
    cases = [(True, 0, args.threads[0], args.seeds[0])]
    cases += [(False, repeat, threads, seed) for repeat in range(args.repeats)
              for threads in args.threads for seed in args.seeds]
    for case_index, (warmup, repeat, threads, seed) in enumerate(cases):
        # Alternate executable order to reduce temperature/background-load bias.
        for label, binary in binaries[::1 if case_index % 2 == 0 else -1]:
            name = f"{label}-{'warmup' if warmup else repeat}-t{threads}-s{seed}"
            output = args.out / name
            command = [str(binary), "run", args.scenario, "--out", str(output),
                       "--threads", str(threads), "--seed", str(seed)]
            if args.ticks is not None:
                command += ["--ticks", str(args.ticks)]
            start = time.perf_counter()
            process = subprocess.run(command, capture_output=True, text=True, encoding="utf-8")
            elapsed = time.perf_counter() - start
            (args.out / f"{name}.log").write_text(process.stdout + process.stderr, encoding="utf-8")
            process.check_returncode()
            summary = json.loads((output / "summary.json").read_text(encoding="utf-8"))
            decisions = sum(a["stats"]["decisions"] for a in summary["accounts"])
            row = {"label": label, "warmup": warmup, "repeat": repeat, "seed": seed,
                   "threads": summary["threads"], "engine_seconds": summary["wall_seconds"],
                   "total_seconds": elapsed, "accounts": len(summary["accounts"]),
                   "decisions": decisions, "trades": summary["trades"], "command": command}
            result["runs"].append(row)
            (args.out / "comparison.json").write_text(json.dumps(result, indent=2), encoding="utf-8")
            print(f"{name}: engine={row['engine_seconds']:.4f}s total={elapsed:.4f}s "
                  f"decisions={decisions} trades={row['trades']}", flush=True)
    for label, _ in binaries:
        for threads in args.threads:
            rows = [r for r in result["runs"] if not r["warmup"]
                    and r["label"] == label and r["threads"] == threads]
            print(f"{label} threads={threads} median engine="
                  f"{statistics.median(r['engine_seconds'] for r in rows):.4f}s, "
                  f"total={statistics.median(r['total_seconds'] for r in rows):.4f}s")


if __name__ == "__main__":
    main()
