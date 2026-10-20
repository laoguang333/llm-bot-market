"""Validate v17's two ordinary entrants against archived v16, serially."""

from collections import Counter
import hashlib
import json
import os
from pathlib import Path
import statistics
import subprocess
import time
import tomllib

from survey_v16 import summarize_run

ROOT = Path(__file__).resolve().parents[1]


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    if os.name == "nt":
        from build_release import windows_build_budget
        budget = windows_build_budget()
    else:
        budget = os.cpu_count()
    paths = {v: ROOT / f"scenarios/baseline-{v}.toml" for v in ["v16", "v17"]}
    configs = {v: tomllib.loads(p.read_text(encoding="utf-8")) for v, p in paths.items()}
    old, new = configs["v16"], configs["v17"]
    entrants = [dict(bot=name, owner=f"{name}-incumbent", count=1,
                     cash=2_000_000, shares_each=20, news_multiplier=1.0)
                for name in ["almanac", "osprey"]]
    assert new["bots"] == old["bots"] + entrants
    assert {k: v for k, v in old.items() if k != "bots"} == {
        k: v for k, v in new.items() if k != "bots"}
    assert paths["v17"].read_bytes() == (ROOT / "scenarios/baseline.toml").read_bytes()
    assert sum(g["count"] for g in new["bots"]) == 105
    assert sum(g.get("count_max", g["count"]) for g in new["bots"]) == 229
    assert len({g["bot"] for g in new["bots"]}) == 19
    prior = json.loads((ROOT / "docs/ecology-v16-results.json").read_text(encoding="utf-8"))
    binary = ROOT / "target/release" / ("llm-market.exe" if os.name == "nt" else "llm-market")
    # Rebuilds on another host may have different binary hashes. Require the
    # reviewed source tree instead, and record the actual binary separately.
    subprocess.run(["git", "diff", "--quiet", "c1cb5eb", "--",
                    "crates", "Cargo.toml", "Cargo.lock"], cwd=ROOT, check=True)
    out = ROOT / "runs/v17-validation"
    out.mkdir(parents=True, exist_ok=False)
    seeds = list(range(82001, 82021))
    manifest = {
        "git_head": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(),
        "seeds": seeds, "repeats": 1, "threads": 4, "cpu_budget": budget,
        "binary_sha256": sha(binary), "configs": {v: sha(p) for v, p in paths.items()},
        "matches_prior_binary": sha(binary) == prior["manifest"]["binary_sha256"],
        "review": "Reviewed source tree c1cb5eb from 80-run v16 survey. Configuration only: "
                  "add ordinary Almanac and Osprey v2; no engine or strategy edits.",
    }
    (out / "manifest.json").write_text(json.dumps(manifest, indent=2), encoding="utf-8")
    rows = []
    for index, seed in enumerate(seeds):
        pair = {}
        for version in (["v16", "v17"] if index % 2 == 0 else ["v17", "v16"]):
            dest = out / f"{version}-{seed}"
            command = [str(binary), "run", str(paths[version]), "--out", str(dest),
                       "--seed", str(seed), "--threads", "4"]
            started = time.perf_counter()
            with dest.with_suffix(".log").open("w", encoding="utf-8") as log:
                subprocess.run(command, cwd=ROOT, stdout=log, stderr=subprocess.STDOUT, check=True)
            elapsed = time.perf_counter() - started
            summary = json.loads((dest / "summary.json").read_text(encoding="utf-8"))
            accounts = summary["accounts"]
            assert summary["audits"] == 20 and summary["threads"] == 4
            assert all(not a["disabled"] and a["stats"]["bot_errors"] == 0 for a in accounts)
            added = [a for a in accounts if a["strategy"] in ["almanac", "osprey"]]
            if version == "v17":
                assert len(added) == 2
                for entrant in entrants:
                    a = next(a for a in added if a["strategy"] == entrant["bot"])
                    assert a["account"] == a["owner"] == entrant["owner"]
                    assert a["initial_cash"] == 2_000_000
                    assert a["initial_shares"] == [20] * len(new["stocks"])
                    assert a["news_multiplier"] == 1 and a["forecast_access"]["horizon"] == 0
                    assert a["forecasts_received"] == 0
            else:
                assert not added
            rows.append({
                "version": version, "seed": seed, "command": command,
                "elapsed_seconds": elapsed, "simulation_seconds": summary["wall_seconds"],
                "accounts": len(accounts), "audits": summary["audits"],
                "quality": summary["quality"], "groups": summarize_run(summary),
                "entrants": [{k: v for k, v in a.items() if k != "notional_by_counterparty"}
                             for a in added],
                "all_errors": sum(a["stats"]["bot_errors"] for a in accounts),
                "all_skipped": sum(a["stats"]["skipped"] for a in accounts),
                "all_disabled": sum(bool(a["disabled"]) for a in accounts),
                "news_sha256": sha(dest / "news.csv"),
                "summary_sha256": sha(dest / "summary.json"),
            })
            pair[version] = summary
            print(f"{len(rows)}/40 {version} seed={seed} accounts={len(accounts)} "
                  f"simulation={summary['wall_seconds']:.3f}s quality={summary['quality']['pass']}",
                  flush=True)
        before, after = pair["v16"], pair["v17"]
        assert before["terminal_payments"] == after["terminal_payments"]
        assert before["final_dividends"] == after["final_dividends"]
        assert sha(out / f"v16-{seed}/news.csv") == sha(out / f"v17-{seed}/news.csv")
        original = {a["account"]: a for a in before["accounts"]}
        updated = {a["account"]: a for a in after["accounts"]}
        assert set(updated) == set(original) | {e["owner"] for e in entrants}
        for name, a in original.items():
            for field in ["owner", "strategy", "initial_cash", "initial_shares",
                          "news_multiplier", "forecast_access"]:
                assert a[field] == updated[name][field]
    assert sha(binary) == manifest["binary_sha256"]
    assert all(sha(paths[v]) == manifest["configs"][v] for v in paths)
    aggregate = {}
    for version in ["v16", "v17"]:
        rs = [r for r in rows if r["version"] == version]
        aggregate[version] = {
            "quality_pass": sum(r["quality"]["pass"] for r in rs),
            "quality_failures": dict(Counter(c["name"] for r in rs
                                             for c in r["quality"]["checks"] if not c["pass"])),
            "simulation_seconds": sum(r["simulation_seconds"] for r in rs),
            "cli_seconds": sum(r["elapsed_seconds"] for r in rs),
            "groups": {g: {k: statistics.mean(r["groups"][g][k] for r in rs)
                           for k in ["percentile", "return_ratio", "profitable_share",
                                     "top25_share", "liquidated_share", "insolvent_share"]}
                       for g in rs[0]["groups"]},
        }
    result = {"manifest": manifest, "aggregate": aggregate, "rows": rows,
              "audits": sum(r["audits"] for r in rows)}
    (ROOT / "docs/baseline-v17-results.json").write_text(
        json.dumps(result, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(aggregate, ensure_ascii=False, indent=2), flush=True)


if __name__ == "__main__":
    main()
