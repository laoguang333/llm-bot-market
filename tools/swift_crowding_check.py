"""Post-discovery diagnostic: hold extra account count/wealth fixed, change peers.

This follow-up was motivated by the first stress batch's crowding failure. It is
not part of the original preregistered suite and does not tune Swift.
Uses the original frozen llm-market executable and fresh seeds 9001..9020.
"""
import argparse
import copy
from datetime import datetime, timezone
import json
from pathlib import Path
import statistics as stats
import subprocess
import tomllib

from swift_stress import ROOT, sha, toml_text

OUT = ROOT / "runs/swift-crowding-20261018"


def prepare():
    if (OUT / "plan.json").exists():
        raise SystemExit("Already prepared")
    base = tomllib.loads((ROOT / "scenarios/baseline.toml").read_text(encoding="utf-8"))
    binary = ROOT / "target/release/llm-market.exe"
    frozen = json.loads((ROOT / "docs/swift-v5-results.json").read_text())["batches"][-1]
    assert sha(binary) == frozen["binary_sha256"]
    OUT.mkdir(parents=True, exist_ok=True)
    jobs = []
    for key, peer_bot, params in [("v5", None, {}), ("swift10", "swift", {}),
                                  ("coral10", "coral", {}),
                                  ("cash10", "swift", {"entry": 100.0, "exit": 100.0})]:
        cfg = copy.deepcopy(base)
        if peer_bot:
            cfg["bots"] += [{"bot": peer_bot, "owner": f"peer_{i}", "count": 1,
                             "cash": 2_000_000, "shares_each": 20, "params": params} for i in range(10)]
        cfg["bots"].append({"bot": "swift", "owner": "challenger", "count": 1,
                            "cash": 2_000_000, "shares_each": 20})
        path = OUT / f"{key}.toml"
        path.write_text(toml_text(cfg), encoding="utf-8")
        jobs.append({"key": key, "scenario": str(path.relative_to(ROOT)), "config": cfg,
                     "scenario_sha256": sha(path), "first_seed": 9001, "seeds": 20})
    plan = {"created_utc": datetime.now(timezone.utc).isoformat(), "binary": str(binary.relative_to(ROOT)),
            "binary_sha256": sha(binary), "script_sha256": sha(Path(__file__)), "jobs": jobs,
            "note": "Post-discovery diagnostic, frozen strategy, 80 additional runs; all 3 added-peer treatments use the same count and initial wealth."}
    (OUT / "plan.json").write_text(json.dumps(plan, ensure_ascii=False, indent=2)+"\n", encoding="utf-8")
    print("Prepared 80 fresh diagnostic runs.")


def run():
    plan = json.loads((OUT / "plan.json").read_text(encoding="utf-8"))
    assert sha(ROOT / plan["binary"]) == plan["binary_sha256"]
    assert sha(Path(__file__)) == plan["script_sha256"]
    for job in plan["jobs"]:
        dest = OUT / job["key"]
        if (dest / "done.json").exists():
            continue
        assert sha(ROOT / job["scenario"]) == job["scenario_sha256"]
        if dest.exists():
            raise SystemExit(f"Incomplete run: {dest}")
        cmd = [str(ROOT / plan["binary"]), "series", str(ROOT / job["scenario"]),
               "--out", str(dest), "--first-seed", str(job["first_seed"]), "--seeds", str(job["seeds"])]
        print("START crowd-diagnostic", job["key"], flush=True)
        with (OUT / f"{job['key']}.log").open("w", encoding="utf-8") as log:
            subprocess.run(cmd, cwd=ROOT, stdout=log, stderr=subprocess.STDOUT, check=True)
        rows = []
        for seed in range(job["first_seed"], job["first_seed"]+job["seeds"]):
            summary = json.loads((dest / f"seed-{seed}/summary.json").read_text())
            a = next(a for a in summary["accounts"] if a["owner"] == "challenger")
            peers = [x for x in summary["accounts"] if x["owner"].startswith("peer_")]
            rows.append({"seed": seed, "percentile": a["percentile"],
                         "return_pct": (a["return_ratio"]-1)*100,
                         "peer_returns_pct": {x["owner"]: (x["return_ratio"]-1)*100 for x in peers},
                         "all_skipped": sum(x["stats"]["skipped"] for x in summary["accounts"]),
                         "all_errors": sum(x["stats"]["bot_errors"] for x in summary["accounts"]),
                         "all_disabled": sum(bool(x["disabled"]) for x in summary["accounts"]),
                         "audits": summary["audits"]})
        record = {"key": job["key"], "command": cmd, "worlds": rows,
                  "mean_percentile": stats.mean(x["percentile"] for x in rows),
                  "mean_return_pct": stats.mean(x["return_pct"] for x in rows),
                  "median_return_pct": stats.median(x["return_pct"] for x in rows),
                  "loss_worlds": sum(x["return_pct"] < 0 for x in rows)}
        (dest / "done.json").write_text(json.dumps(record, ensure_ascii=False, indent=2)+"\n", encoding="utf-8")
        print("DONE crowd-diagnostic", job["key"], "rank", round(record["mean_percentile"], 2),
              "return", round(record["mean_return_pct"], 2), "losses", record["loss_worlds"], flush=True)
    result = {"plan": plan, "results": [json.loads((OUT / j["key"] / "done.json").read_text(encoding="utf-8")) for j in plan["jobs"]]}
    (ROOT / "docs/swift-crowding-results.json").write_text(json.dumps(result, ensure_ascii=False, indent=2)+"\n", encoding="utf-8")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("mode", choices=["prepare", "run"])
    {"prepare": prepare, "run": run}[parser.parse_args().mode]()
