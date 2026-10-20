"""Conserved-capital experiments. Run serially through arena_gate.py."""

import argparse
from collections import Counter, defaultdict
import csv
import hashlib
import json
import os
from pathlib import Path
import statistics
import subprocess
import time
import tomllib

ROOT = Path(__file__).resolve().parents[1]
SOURCES = [
    "crates/market/src/capital.rs", "crates/market/src/config.rs",
    "crates/market/src/engine.rs", "crates/market/src/lib.rs",
]


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def save(path, data):
    path.write_text(json.dumps(data, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")


def opening(summary):
    accounts = summary["accounts"]
    return {
        "cash": sum(a["initial_cash"] for a in accounts),
        "shares": [sum(a["initial_shares"][s] for a in accounts)
                   for s in range(len(accounts[0]["initial_shares"]))],
        "equity": sum(a["initial_equity"] for a in accounts),
    }


def summarize(summary, folder):
    accounts = summary["accounts"]
    initial = sum(a["initial_equity"] for a in accounts)
    gross_loss = sum(max(0, a["initial_equity"] - a["equity"]) for a in accounts)
    groups = {}
    for strategy in sorted({a["strategy"] for a in accounts}):
        aa = [a for a in accounts if a["strategy"] == strategy]
        capital = sum(a["initial_equity"] for a in aa)
        loss = sum(max(0, a["initial_equity"] - a["equity"]) for a in aa)
        groups[strategy] = {
            "accounts": len(aa), "initial_capital_share": capital / initial,
            "gross_loss_share": loss / gross_loss if gross_loss else 0,
            "net_return": sum(a["equity"] - a["initial_equity"] for a in aa) / capital,
            "percentile": statistics.mean(a["percentile"] for a in aa),
            "liquidated_accounts": sum(a["stats"]["liquidations"] > 0 for a in aa),
            "insolvent_accounts": sum(a["equity"] < 0 for a in aa),
        }
    with (folder / "books.csv").open(encoding="utf-8", newline="") as f:
        depths = [min(int(r["bid_depth_2pct"]), int(r["ask_depth_2pct"]))
                  if r["bid"] and r["ask"] else 0 for r in csv.DictReader(f)]
    return {
        "groups": groups, "quality": summary["quality"],
        "opening": opening(summary),
        "max_group_loss_share": max(g["gross_loss_share"] for g in groups.values()),
        "max_group_capital_share": max(g["initial_capital_share"] for g in groups.values()),
        "market_net_return": sum(a["equity"] - a["initial_equity"] for a in accounts) / initial,
        "gross_loss_per_initial": gross_loss / initial,
        "liquidated_accounts": sum(a["stats"]["liquidations"] > 0 for a in accounts),
        "insolvent_accounts": sum(a["equity"] < 0 for a in accounts),
        "bad_debt_per_initial": sum(a["stats"]["bad_debt"] for a in accounts) / initial,
        "mean_thin_side_depth_cents": statistics.mean(depths),
        "median_thin_side_depth_cents": statistics.median(depths),
        "simulation_seconds": summary["wall_seconds"],
        "audits": summary["audits"],
        "errors": sum(a["stats"]["bot_errors"] for a in accounts),
        "skipped": sum(a["stats"]["skipped"] for a in accounts),
        "disabled": sum(bool(a["disabled"]) for a in accounts),
    }


def compare(before, after, cfg):
    assert opening(before) == opening(after)
    assert before["terminal_payments"] == after["terminal_payments"]
    assert before["final_dividends"] == after["final_dividends"]
    aa, bb = before["accounts"], after["accounts"]
    assert len(aa) == len(bb)
    named = {g["owner"] for g in cfg["bots"] if "owner" in g}
    plan = cfg["capital_redistribution"]
    totals = defaultdict(lambda: [0, [0] * len(cfg["stocks"])])
    for a, b in zip(aa, bb):
        for field in ["account", "owner", "strategy", "news_multiplier", "forecast_access",
                      "forecasts_received", "news_received_by_stage"]:
            assert a[field] == b[field], (field, a["account"])
        dc = b["initial_cash"] - a["initial_cash"]
        dq = [y - x for x, y in zip(a["initial_shares"], b["initial_shares"])]
        if a["owner"] in named or a["strategy"] not in {plan["from_bot"], *plan["recipients"]}:
            assert dc == 0 and not any(dq)
        elif a["strategy"] == plan["from_bot"]:
            assert dc == -plan["cash_each"]
            assert all(q == -plan["shares_each"] for q in dq)
        else:
            assert dc >= 0 and all(q >= 0 for q in dq)
            totals[a["strategy"]][0] += dc
            totals[a["strategy"]][1] = [x + y for x, y in zip(totals[a["strategy"]][1], dq)]
    donors = sum(a["strategy"] == plan["from_bot"] and a["owner"] not in named for a in aa)
    weight_sum = sum(plan["recipients"].values())
    cumulative = 0
    for name, weight in sorted(plan["recipients"].items()):
        next_weight = cumulative + weight
        cash = donors * plan["cash_each"]
        shares = donors * plan["shares_each"]
        assert totals[name][0] == cash * next_weight // weight_sum - cash * cumulative // weight_sum
        assert all(q == shares * next_weight // weight_sum - shares * cumulative // weight_sum
                   for q in totals[name][1])
        cumulative = next_weight


def aggregate(rows):
    result = {}
    for arm in sorted({r["arm"] for r in rows}):
        rr = [r for r in rows if r["arm"] == arm]
        result[arm] = {
            "runs": len(rr), "quality_pass": sum(r["quality"]["pass"] for r in rr),
            "quality_failures": dict(Counter(c["name"] for r in rr
                                             for c in r["quality"]["checks"] if not c["pass"])),
            "quality_means": {c["name"]: statistics.mean(
                next(x["value"] for x in r["quality"]["checks"] if x["name"] == c["name"])
                for r in rr) for c in rr[0]["quality"]["checks"]},
            "groups": {g: {k: statistics.mean(r["groups"][g][k] for r in rr)
                           for k in rr[0]["groups"][g]} for g in rr[0]["groups"]},
            "max_loss_below_60_share": statistics.mean(r["max_group_loss_share"] <= 0.6 for r in rr),
            "max_capital_over_runs": max(r["max_group_capital_share"] for r in rr),
            "audits": sum(r["audits"] for r in rr),
            "cli_seconds_total": sum(r["cli_seconds"] for r in rr),
            "simulation_seconds_total": sum(r["simulation_seconds"] for r in rr),
        }
        for key in ["max_group_loss_share", "market_net_return", "gross_loss_per_initial",
                    "liquidated_accounts", "insolvent_accounts", "bad_debt_per_initial",
                    "mean_thin_side_depth_cents", "median_thin_side_depth_cents"]:
            result[arm][key] = statistics.mean(r[key] for r in rr)
    base = result["v17"]
    for arm, value in result.items():
        if arm == "v17":
            continue
        value["acceptance"] = {
            "grid_loss_below_35": value["groups"]["grid"]["gross_loss_share"] < 0.35,
            "all_group_mean_loss_below_35": max(
                g["gross_loss_share"] for g in value["groups"].values()) <= 0.35,
            "80pct_runs_max_loss_below_60": value["max_loss_below_60_share"] >= 0.8,
            "all_initial_group_capital_below_35": value["max_capital_over_runs"] <= 0.35,
            "quality_pass_not_lower": value["quality_pass"] >= base["quality_pass"],
            "two_sided_drop_at_most_5pp": value["quality_means"]["two_sided_share"] >=
                base["quality_means"]["two_sided_share"] - 0.05,
        }
        value["accepted"] = all(value["acceptance"].values())
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("stage", choices=["dev", "holdout"])
    args = parser.parse_args()
    if os.name == "nt":
        from build_release import windows_build_budget
        budget = windows_build_budget()
    else:
        budget = os.cpu_count()
    subprocess.run(["git", "diff", "--quiet", "cdcf87c", "--", "crates/bots", "crates/bot-api"],
                   cwd=ROOT, check=True)
    binary = ROOT / "target/release" / ("llm-market.exe" if os.name == "nt" else "llm-market")
    freeze = ROOT / "runs/capital-v18-selection.json"
    if args.stage == "dev" and freeze.exists():
        raise FileExistsError(f"Preserve the existing frozen selection before rerunning: {freeze}")
    selected = json.loads(freeze.read_text()) if args.stage == "holdout" else None
    arms = ["v17", selected["candidate"]] if selected else ["v17", "balanced", "quotes"]
    seeds = list(range(84001, 84021)) if selected else list(range(83001, 83011))
    paths = {a: ROOT / "scenarios" / (
        "baseline-v17.toml" if a == "v17" else f"capital-v18-{a}.toml") for a in arms}
    configs = {a: tomllib.loads(p.read_text(encoding="utf-8")) for a, p in paths.items()}
    for a in arms[1:]:
        assert {k: v for k, v in configs[a].items() if k != "capital_redistribution"} == configs["v17"]
    source_hashes = {p: sha(ROOT / p) for p in SOURCES}
    manifest = {
        "stage": args.stage, "seeds": seeds, "repeats": 2, "threads": 4, "cpu_budget": budget,
        "git_head": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(),
        "binary_sha256": sha(binary), "source_hashes": source_hashes,
        "scenario_hashes": {a: sha(p) for a, p in paths.items()},
        "review": "Existing reviewed bots and API unchanged from cdcf87c. "
                  "Only opening endowments change; no hidden-world access, no bot source inspection.",
    }
    if selected:
        assert manifest["binary_sha256"] == selected["binary_sha256"]
        assert manifest["source_hashes"] == selected["source_hashes"]
        assert manifest["scenario_hashes"][selected["candidate"]] == selected["scenario_sha256"]
    out = ROOT / "runs" / f"capital-v18-{args.stage}"
    out.mkdir(parents=True, exist_ok=False)
    save(out / "manifest.json", manifest)
    rows = []
    for index, seed in enumerate(seeds):
        for repeat in range(2):
            rotation = (index + repeat) % len(arms)
            order = arms[rotation:] + arms[:rotation]
            pair = {}
            for arm in order:
                dest = out / f"{arm}-{seed}-r{repeat}"
                command = [str(binary), "run", str(paths[arm]), "--out", str(dest),
                           "--seed", str(seed), "--threads", "4"]
                started = time.perf_counter()
                with dest.with_suffix(".log").open("w", encoding="utf-8") as log:
                    subprocess.run(command, cwd=ROOT, stdout=log, stderr=subprocess.STDOUT, check=True)
                elapsed = time.perf_counter() - started
                summary = json.loads((dest / "summary.json").read_text(encoding="utf-8"))
                row = summarize(summary, dest)
                assert row["audits"] == 20 and summary["threads"] == 4
                assert row["errors"] == row["skipped"] == row["disabled"] == 0
                row.update(arm=arm, seed=seed, repeat=repeat, cli_seconds=elapsed,
                           summary_sha256=sha(dest / "summary.json"), news_sha256=sha(dest / "news.csv"),
                           command=command)
                rows.append(row)
                pair[arm] = (summary, row["news_sha256"])
                save(out / "progress.json", rows)
                print(f"{len(rows)}/{len(seeds)*2*len(arms)} {arm} seed={seed} r={repeat} "
                      f"quality={row['quality']['pass']} grid_loss={row['groups']['grid']['gross_loss_share']:.3f} "
                      f"max_loss={row['max_group_loss_share']:.3f}", flush=True)
            for arm in arms[1:]:
                compare(pair["v17"][0], pair[arm][0], configs[arm])
                assert pair["v17"][1] == pair[arm][1]
    assert sha(binary) == manifest["binary_sha256"]
    assert all(sha(ROOT / p) == h for p, h in source_hashes.items())
    assert all(sha(paths[a]) == manifest["scenario_hashes"][a] for a in arms)
    result = {"manifest": manifest, "aggregate": aggregate(rows), "rows": rows}
    save(ROOT / "docs" / f"capital-v18-{args.stage}-results.json", result)
    if not selected:
        candidates = [a for a in arms[1:] if result["aggregate"][a]["accepted"]]
        if candidates:
            chosen = min(candidates, key=lambda a: (
                -result["aggregate"][a]["quality_pass"],
                result["aggregate"][a]["max_group_loss_share"], a != "balanced"))
            save(freeze, {"candidate": chosen, "binary_sha256": manifest["binary_sha256"],
                          "source_hashes": source_hashes, "scenario_sha256": sha(paths[chosen])})
            print(f"FROZEN SELECTION: {chosen}", flush=True)
        else:
            print("NO CANDIDATE PASSED. Holdout remains unused.", flush=True)
    print(json.dumps({a: {k: v for k, v in value.items() if k != "groups"}
                      for a, value in result["aggregate"].items()}, indent=2), flush=True)


if __name__ == "__main__":
    main()
