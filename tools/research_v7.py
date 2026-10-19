"""Fixed, paired v7 study. Run through arena_gate.py; all simulations are serial.

No strategy tuning. The ordinary 10x Titan baseline is compared to 1x capital,
and the regime is decomposed into common candidate cycles and late continuation.
"""
import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import shutil
import statistics as stats
import subprocess
import time
import tomllib
import zipfile

from build_release import windows_build_budget

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "runs/v7-dragon-study"
ARMS = ["v6", "titan10_no_dragon", "titan1_no_dragon", "candidates_only", "dragon_titan10", "dragon_titan1"]


def read(path):
    return json.loads(path.read_text(encoding="utf-8"))


def write(path, data):
    path.write_text(json.dumps(data, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def manifest():
    files = sorted((ROOT / "crates").rglob("*.rs")) + sorted((ROOT / "crates").rglob("Cargo.toml"))
    files += [ROOT / name for name in ["Cargo.toml", "Cargo.lock", "tools/research_v7.py",
              "scenarios/baseline-v6.toml", "scenarios/baseline-v7-pulses.toml", "scenarios/baseline-v7-no-dragon.toml"]]
    return {str(p.relative_to(ROOT)): sha(p) for p in files}


def prepare():
    OUT.mkdir(parents=True, exist_ok=True)
    if not (OUT / "plan.json").exists():
        write(OUT / "plan.json", dict(seeds=list(range(27001, 27041)), threads=4, cpu_budget=10,
              repeats_per_world_arm=1, arms=ARMS, no_strategy_tuning=True, dragon_probability=0.5,
              notes="Fixed paired worlds; strategy averages equally weighted by world. Same ordinary information and fees. Capital replacement changes total supply. Performance is separate and serial."))
    plan = read(OUT / "plan.json")
    assert plan["seeds"] == list(range(27001, 27041)) and plan["arms"] == ARMS
    v6 = (ROOT / "scenarios/baseline-v6.toml").read_text(encoding="utf-8")
    off = (ROOT / "scenarios/baseline-v7-no-dragon.toml").read_text(encoding="utf-8")
    full = (ROOT / "scenarios/baseline-v7-pulses.toml").read_text(encoding="utf-8")
    old, new = tomllib.loads(v6), tomllib.loads(off)
    assert new["bots"][:-1] == old["bots"]
    assert {k: v for k, v in new.items() if k != "bots"} == {k: v for k, v in old.items() if k != "bots"}
    assert new["bots"][-1] == dict(bot="titan", owner="titan-value", count=1, cash=20_000_000, shares_each=200, news_multiplier=1.0)
    assert tomllib.loads(full)["world"]["dragon"].get("model", "pulses") == "pulses"
    def small(text):
        return text.replace("cash = 20_000_000\nshares_each = 200", "cash = 2_000_000\nshares_each = 20")
    configs = dict(v6=v6, titan10_no_dragon=off, titan1_no_dragon=small(off),
                   candidates_only=full.replace("probability = 0.5", "probability = 0.0"),
                   dragon_titan10=full, dragon_titan1=small(full))
    folder = OUT / "configs"
    folder.mkdir(exist_ok=False)
    for key, text in configs.items():
        (folder / f"{key}.toml").write_text(text, encoding="utf-8")
    shutil.copy2(ROOT / "target/release/llm-market.exe", OUT / "after.exe")
    write(OUT / "manifest.json", dict(source=manifest(), binary_sha256=sha(OUT / "after.exe"),
          before_binary_sha256=sha(OUT / "before.exe") if (OUT / "before.exe").exists() else None,
          configs={k: sha(folder / f"{k}.toml") for k in configs}))
    with zipfile.ZipFile(OUT / "frozen-sources.zip", "x") as archive:
        for path in manifest():
            archive.write(ROOT / path, path)
        for name in ["plan.json", "manifest.json"]:
            archive.write(OUT / name, name)


def run_one(binary, config, directory, seed, threads=4):
    command = [str(binary), "run", str(config), "--out", str(directory), "--seed", str(seed), "--threads", str(threads)]
    start = time.perf_counter()
    result = subprocess.run(command, cwd=ROOT, capture_output=True, text=True, encoding="utf-8")
    elapsed = time.perf_counter() - start
    if result.returncode:
        raise RuntimeError(f"{command}:\n{result.stdout}\n{result.stderr}")
    write(directory / "command.json", dict(command=command, wall_seconds=elapsed))
    summary = read(directory / "summary.json")
    assert summary["threads"] == threads and summary["audits"] >= 6
    assert all(a["news_multiplier"] == 1.0 and not a["disabled"] and a["stats"]["bot_errors"] == 0 for a in summary["accounts"])
    return summary, elapsed


def run():
    plan = read(OUT / "plan.json")
    for seed in plan["seeds"]:
        # Alternate order without selecting worlds or changing strategy parameters.
        arms = ARMS if seed % 2 else ARMS[::-1]
        for arm in arms:
            directory = OUT / arm / f"seed-{seed}"
            if directory.exists():
                raise RuntimeError(f"refusing to reuse an existing run: {directory}")
            run_one(OUT / "after.exe", OUT / "configs" / f"{arm}.toml", directory, seed)
        print(f"Completed paired world {seed} ({seed - plan['seeds'][0] + 1}/40)", flush=True)


def performance():
    if not (OUT / "before.exe").exists():
        raise RuntimeError("Performance compatibility requires a saved prior release: prepare --before-binary PATH")
    folder = OUT / "performance"
    folder.mkdir(exist_ok=False)
    jobs = [("before_v6", OUT / "before.exe", "v6"), ("after_v6", OUT / "after.exe", "v6"),
            ("titan_no_dragon", OUT / "after.exe", "titan10_no_dragon"), ("v7", OUT / "after.exe", "dragon_titan10")]
    for label, binary, config in jobs:
        run_one(binary, OUT / "configs" / f"{config}.toml", folder / f"warm-{label}", 7)
    rows = []
    for seed in [7, 11, 19]:
        for rep in range(3):
            for label, binary, config in (jobs if rep % 2 == 0 else jobs[::-1]):
                s, elapsed = run_one(binary, OUT / "configs" / f"{config}.toml", folder / f"{label}-{seed}-{rep}", seed)
                rows.append(dict(label=label, seed=seed, repeat=rep, threads=4, process_seconds=elapsed,
                                 engine_seconds=s["wall_seconds"], accounts=len(s["accounts"]), trades=s["trades"]))
    write(OUT / "performance.json", rows)
    # Legacy single-thread equivalence also checks fees, priority and delivery as a whole.
    for seed in [7, 11, 19]:
        summaries = []
        for label in ["before", "after"]:
            directory = folder / f"compat-{label}-{seed}"
            s, _ = run_one(OUT / f"{label}.exe", OUT / "configs/v6.toml", directory, seed, threads=1)
            summaries.append(s)
        left, right = [folder / f"compat-{x}-{seed}" for x in ["before", "after"]]
        assert sha(left / "trades.csv") == sha(right / "trades.csv")
        assert sha(left / "news.csv") == sha(right / "news.csv")
        def account_state(s):
            return [(a["owner"], a["cash"], a["shares"], a["equity"], a["news_received_by_stage"]) for a in s["accounts"]]
        assert account_state(summaries[0]) == account_state(summaries[1])
    print("Performance and three exact legacy compatibility worlds passed.", flush=True)


def group_key(account):
    return account["owner"] if account["owner"] in {"swift-standard", "swift-early-single", "titan-value"} else account["strategy"]


def performance_selected():
    # The predeclared performance worlds 7/11/19 all drew no winner. Supplement
    # them with the first three selected worlds, chosen by role, not by timing.
    selected = [seed for seed in read(OUT / "plan.json")["seeds"]
                if read(OUT / "dragon_titan10" / f"seed-{seed}" / "summary.json")["dragon"]["selected_stock"] is not None][:3]
    folder = OUT / "performance-selected"
    folder.mkdir(exist_ok=False)
    rows = []
    for seed in selected:
        for rep in range(3):
            arms = ["titan10_no_dragon", "dragon_titan10"]
            for arm in (arms if rep % 2 == 0 else arms[::-1]):
                s, elapsed = run_one(OUT / "after.exe", OUT / "configs" / f"{arm}.toml",
                                     folder / f"{arm}-{seed}-{rep}", seed)
                rows.append(dict(label=arm, seed=seed, repeat=rep, threads=4, process_seconds=elapsed,
                                 engine_seconds=s["wall_seconds"], trades=s["trades"], selected_stock=s.get("dragon", {}).get("selected_stock") if s.get("dragon") else None))
    write(OUT / "performance-selected.json", dict(selection="first three of fixed seeds with a selected dragon", seeds=selected, rows=rows))


def summarize_accounts(accounts):
    returns = [(a["return_ratio"] - 1) * 100 for a in accounts]
    mean = stats.mean
    result = dict(accounts=len(accounts), return_pct=mean(returns), percentile=mean(a["percentile"] for a in accounts),
                  loss_share=mean(r < 0 for r in returns), top25_share=mean(a["percentile"] >= 75 for a in accounts))
    for key in ["dividends", "combo_dividends", "trade_fees", "idle_fees", "interest", "volume_notional"]:
        result[key + "_pct"] = mean(a["stats"][key] / a["initial_equity"] * 100 for a in accounts)
    result["trading_inventory_pct"] = result["return_pct"] - result["dividends_pct"] - result["combo_dividends_pct"] + result["trade_fees_pct"] + result["idle_fees_pct"] + result["interest_pct"]
    return result


def world_row(arm, seed):
    directory = OUT / arm / f"seed-{seed}"
    s = read(directory / "summary.json")
    accounts = s["accounts"]
    groups = sorted({group_key(a) for a in accounts})
    return dict(seed=seed, accounts=len(accounts), dragon=s.get("dragon"), terminal_payments=s["terminal_payments"],
                final_dividends=s["final_dividends"], news_sha256=sha(directory / "news.csv"),
                process_seconds=read(directory / "command.json")["wall_seconds"], engine_seconds=s["wall_seconds"],
                market_return_pct=(sum(a["equity"] for a in accounts) / sum(a["initial_equity"] for a in accounts) - 1) * 100,
                groups={g: summarize_accounts([a for a in accounts if group_key(a) == g]) for g in groups},
                audits=s["audits"], skipped=sum(a["stats"]["skipped"] for a in accounts),
                errors=sum(a["stats"]["bot_errors"] for a in accounts), disabled=sum(bool(a["disabled"]) for a in accounts),
                liquidations=sum(a["stats"]["liquidations"] for a in accounts), quality=s["quality"])


def summarize_worlds(worlds):
    groups = worlds[0]["groups"]
    return dict(worlds=len(worlds), dragon_worlds=sum(w["dragon"] is not None and w["dragon"]["selected_stock"] is not None for w in worlds),
                market_return_pct=stats.mean(w["market_return_pct"] for w in worlds),
                quality_passes=sum(w["quality"]["pass"] for w in worlds),
                skipped=sum(w["skipped"] for w in worlds), errors=sum(w["errors"] for w in worlds),
                disabled=sum(w["disabled"] for w in worlds), liquidations=sum(w["liquidations"] for w in worlds),
                audits=sum(w["audits"] for w in worlds),
                groups={g: {key: stats.mean(w["groups"][g][key] for w in worlds) for key in groups[g]} for g in groups})


def delta_summary(values):
    mean = stats.mean(values)
    error = 1.96 * stats.stdev(values) / math.sqrt(len(values)) if len(values) > 1 else 0
    return dict(n=len(values), mean=mean, median=stats.median(values), positive=sum(v > 0 for v in values),
                approximate_95_interval=[mean - error, mean + error])


def report():
    plan = read(OUT / "plan.json")
    worlds = {arm: [world_row(arm, seed) for seed in plan["seeds"]] for arm in ARMS}
    for rows in zip(*(worlds[arm] for arm in ARMS)):
        assert len({r["terminal_payments"] for r in rows}) == 1
        assert len({r["news_sha256"] for r in rows[:3]}) == 1
        assert rows[4]["news_sha256"] == rows[5]["news_sha256"]
        assert rows[0]["accounts"] + 1 == rows[1]["accounts"] == rows[4]["accounts"]
        if rows[4]["dragon"]["selected_stock"] is None:
            assert rows[3]["news_sha256"] == rows[4]["news_sha256"]
    arms = {arm: summarize_worlds(ws) for arm, ws in worlds.items()}
    pairs = {}
    for name, left, right, selected in [
        ("add_titan", "v6", "titan10_no_dragon", False),
        ("full_regime", "titan10_no_dragon", "dragon_titan10", False),
        ("continuation_selected_worlds", "candidates_only", "dragon_titan10", True),
        ("smaller_titan_no_dragon", "titan10_no_dragon", "titan1_no_dragon", False),
        ("smaller_titan_dragon", "dragon_titan10", "dragon_titan1", False)]:
        matched = [(a, b) for a, b in zip(worlds[left], worlds[right]) if not selected or b["dragon"]["selected_stock"] is not None]
        common = worlds[left][0]["groups"].keys() & worlds[right][0]["groups"].keys()
        pairs[name] = {g: {k: delta_summary([b["groups"][g][k] - a["groups"][g][k] for a, b in matched])
                               for k in ["return_pct", "percentile", "dividends_pct", "trading_inventory_pct"]} for g in sorted(common)}
    perf = read(OUT / "performance.json")
    performance_summary = {label: {key: stats.median(r[key] for r in perf if r["label"] == label)
                           for key in ["process_seconds", "engine_seconds"]} for label in sorted({r["label"] for r in perf})}
    result = dict(plan=plan, manifest=read(OUT / "manifest.json"), summary=arms, paired=pairs,
                  analysis_tool_sha256=sha(Path(__file__)), performance_selected=read(OUT / "performance-selected.json"),
                  performance_summary=performance_summary, performance=perf, worlds=worlds)
    write(ROOT / "docs/baseline-v7-pulses-results.json", result)
    for arm, data in arms.items():
        print(arm, "quality", data["quality_passes"], "market", round(data["market_return_pct"], 2),
              "dragon", data["dragon_worlds"], "Titan", data["groups"].get("titan-value"), flush=True)


def main():
    global OUT
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=["prepare", "run", "performance", "performance_selected", "report"])
    parser.add_argument("--out", type=Path, default=OUT)
    parser.add_argument("--before-binary", type=Path, help="prior release binary for performance/compatibility comparison")
    args = parser.parse_args()
    OUT = args.out.resolve()
    if args.before_binary is not None:
        if args.command != "prepare":
            parser.error("--before-binary is only used by prepare")
        OUT.mkdir(parents=True, exist_ok=True)
        if (OUT / "before.exe").exists():
            parser.error("prior binary already exists; use a fresh output directory")
        shutil.copy2(args.before_binary, OUT / "before.exe")
    if os.name == "nt":
        print("Windows CPU affinity budget:", windows_build_budget(), flush=True)
    globals()[args.command]()


if __name__ == "__main__":
    main()
