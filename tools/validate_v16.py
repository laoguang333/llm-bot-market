"""Validate the Lattice promotion against archived v15, serially.

Run with: python tools/arena_gate.py --cwd . -- python tools/validate_v16.py
No bot implementation other than the reviewed Lattice source is inspected.
"""

import hashlib
import json
import os
from pathlib import Path
import subprocess
import time
import tomllib

ROOT = Path(__file__).resolve().parents[1]


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    if os.name == "nt":
        from build_release import windows_build_budget
        budget = windows_build_budget()
    else:
        budget = os.cpu_count()
    paths = {v: ROOT / f"scenarios/baseline-{v}.toml" for v in ["v15", "v16"]}
    configs = {v: tomllib.loads(p.read_text(encoding="utf-8")) for v, p in paths.items()}
    old, new = configs["v15"], configs["v16"]
    entrant = dict(bot="lattice", owner="lattice-incumbent", count=1,
                   cash=2_000_000, shares_each=20, news_multiplier=1.0)
    assert new["bots"][:-1] == old["bots"]
    assert new["bots"][-1] == entrant
    assert {k: v for k, v in old.items() if k != "bots"} == {
        k: v for k, v in new.items() if k != "bots"
    }
    assert paths["v16"].read_bytes() == (ROOT / "scenarios/baseline.toml").read_bytes()
    assert sum(g["count"] for g in new["bots"]) == 103
    assert sum(g.get("count_max", g["count"]) for g in new["bots"]) == 227
    assert len({g["bot"] for g in new["bots"]}) == 17
    assert sum(g["bot"] == "lattice" for g in new["bots"]) == 1
    prior = json.loads((ROOT / "docs/lattice-results.json").read_text(encoding="utf-8"))
    reviewed_hash = prior["freeze"]["sha256"]["crates/bots/src/lattice.rs"]
    source = ROOT / "crates/bots/src/lattice.rs"
    # The reviewed source used LF; Git may check it out as CRLF on Windows.
    assert hashlib.sha256(source.read_bytes().replace(b"\r\n", b"\n")).hexdigest() == reviewed_hash

    binary = ROOT / "target/release" / ("llm-market.exe" if os.name == "nt" else "llm-market")
    out = ROOT / "runs/v16-validation"
    out.mkdir(parents=True, exist_ok=False)
    seeds = [7, 11, 19]
    manifest = {
        "seeds": seeds, "repeats": 1, "threads": 4, "cpu_budget": budget,
        "binary_sha256": sha(binary), "configs": {v: sha(p) for v, p in paths.items()},
        "lattice_source_sha256": sha(source),
        "review": "Lattice source unchanged from reviewed v15 challenge; no engine or old bot edits.",
    }
    (out / "manifest.json").write_text(json.dumps(manifest, indent=2), encoding="utf-8")
    rows = []
    for index, seed in enumerate(seeds):
        pair = {}
        for version in (["v15", "v16"] if index % 2 == 0 else ["v16", "v15"]):
            dest = out / f"{version}-{seed}"
            command = [str(binary), "run", str(paths[version]), "--out", str(dest),
                       "--seed", str(seed), "--threads", "4"]
            started = time.perf_counter()
            with dest.with_suffix(".log").open("w", encoding="utf-8") as log:
                subprocess.run(command, cwd=ROOT, stdout=log, stderr=subprocess.STDOUT, check=True)
            elapsed = time.perf_counter() - started
            summary = json.loads((dest / "summary.json").read_text(encoding="utf-8"))
            accounts = summary["accounts"]
            assert all(not a["disabled"] and a["stats"]["bot_errors"] == 0
                       and a["stats"]["skipped"] == 0 for a in accounts)
            assert summary["audits"] == 20
            assert summary["threads"] == 4
            lattice = [a for a in accounts if a["strategy"] == "lattice"]
            if version == "v16":
                assert len(lattice) == 1
                a = lattice[0]
                assert a["account"] == a["owner"] == entrant["owner"]
                assert a["initial_cash"] == entrant["cash"]
                assert a["initial_shares"] == [20] * len(new["stocks"])
                assert a["news_multiplier"] == 1.0
                assert a["forecast_access"]["horizon"] == a["forecasts_received"] == 0
                assert a["stats"]["liquidations"] == a["stats"]["rejects"] == 0
            else:
                assert not lattice
            rows.append({
                "version": version, "seed": seed, "repeat": 0, "command": command,
                "elapsed_seconds": elapsed, "simulation_seconds": summary["wall_seconds"],
                "accounts": len(accounts), "audits": summary["audits"],
                "quality": summary["quality"], "lattice": lattice,
                "all_errors": sum(a["stats"]["bot_errors"] for a in accounts),
                "all_skipped": sum(a["stats"]["skipped"] for a in accounts),
                "all_disabled": sum(bool(a["disabled"]) for a in accounts),
                "news_sha256": sha(dest / "news.csv"),
            })
            pair[version] = summary
            print(f"{version} seed={seed} accounts={len(accounts)} "
                  f"simulation={summary['wall_seconds']:.3f}s quality={summary['quality']['pass']}",
                  flush=True)
        before, after = pair["v15"], pair["v16"]
        assert before["terminal_payments"] == after["terminal_payments"]
        assert before["final_dividends"] == after["final_dividends"]
        assert sha(out / f"v15-{seed}/news.csv") == sha(out / f"v16-{seed}/news.csv")
        original = {a["account"]: a for a in before["accounts"]}
        updated = {a["account"]: a for a in after["accounts"]}
        assert set(updated) == set(original) | {entrant["owner"]}
        for name, a in original.items():
            for field in ["owner", "strategy", "initial_cash", "initial_shares",
                          "news_multiplier", "forecast_access"]:
                assert a[field] == updated[name][field]
    result = {
        "manifest": manifest, "rows": rows,
        "audits": sum(r["audits"] for r in rows),
        "note": "Three paired worlds, one run per arm: promotion regression, not challenge certification.",
    }
    (ROOT / "docs/baseline-v16-results.json").write_text(
        json.dumps(result, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print("Validated: one ordinary entrant; old accounts and exogenous world unchanged.", flush=True)


if __name__ == "__main__":
    main()
