"""Paired v7 trend revision, keeping bot logic, ordinary news and 3000 ticks fixed.

Run prepare, then run through arena_gate.py, then performance, then report.
The archived eight-node pulse model remains a separate reproducible control.
"""
import argparse
from collections import Counter
import csv
import hashlib
import json
import os
from pathlib import Path
import shutil
import statistics as stats
import zipfile

import research_v7 as common
from build_release import windows_build_budget

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "runs/v7-trend-study"
ARMS = ["off", "pulses", "trend"]
SEEDS = list(range(27001, 27041))


def prepare():
    OUT.mkdir(parents=True, exist_ok=True)
    if not (OUT / "plan.json").exists():
        common.write(OUT / "plan.json", dict(seeds=SEEDS, arms=ARMS, threads=4, bot_changes=False,
                     purpose="Paired shape revision, not independent challenger acceptance"))
    paths = dict(off="baseline-v7-no-dragon.toml", pulses="baseline-v7-pulses.toml", trend="baseline-v7.toml")
    folder = OUT / "configs"
    folder.mkdir(exist_ok=False)
    for arm, name in paths.items():
        shutil.copy2(ROOT / "scenarios" / name, folder / f"{arm}.toml")
    shutil.copy2(ROOT / "target/release/llm-market.exe", OUT / "after.exe")
    sources = sorted((ROOT / "crates").rglob("*.rs")) + sorted((ROOT / "crates").rglob("Cargo.toml"))
    sources += [ROOT / "Cargo.toml", ROOT / "Cargo.lock", Path(__file__), ROOT / "tools/research_v7.py"]
    manifest = {str(p.relative_to(ROOT)): common.sha(p) for p in sources}
    common.write(OUT / "manifest.json", dict(source=manifest, binary_sha256=common.sha(OUT / "after.exe"),
                 configs={arm: common.sha(folder / f"{arm}.toml") for arm in ARMS}))
    with zipfile.ZipFile(OUT / "frozen-sources.zip", "x") as archive:
        for path in sources:
            archive.write(path, str(path.relative_to(ROOT)))
        for path in folder.iterdir():
            archive.write(path, "configs/" + path.name)


def run():
    for seed in SEEDS:
        for arm in (ARMS if seed % 2 else ARMS[::-1]):
            common.run_one(OUT / "after.exe", OUT / "configs" / f"{arm}.toml", OUT / arm / f"seed-{seed}", seed)
        print(f"Completed {seed} ({seed - SEEDS[0] + 1}/40)", flush=True)


def performance():
    directory = OUT / "performance"
    directory.mkdir(exist_ok=False)
    records = []
    for arm in ARMS:
        common.run_one(OUT / "after.exe", OUT / "configs" / f"{arm}.toml", directory / f"warm-{arm}", 27003)
    for seed in [27003, 27006, 27007]:
        for rep in range(3):
            for arm in (ARMS if rep % 2 else ARMS[::-1]):
                s, elapsed = common.run_one(OUT / "after.exe", OUT / "configs" / f"{arm}.toml", directory / f"{arm}-{seed}-{rep}", seed)
                records.append(dict(arm=arm, seed=seed, repeat=rep, process_seconds=elapsed, engine_seconds=s["wall_seconds"],
                                    news_events=s["news_events"], trades=s["trades"]))
    common.write(OUT / "performance.json", records)
    if (OUT / "before.exe").exists():
        for seed in [7, 11, 19]:
            for arm in ["off", "pulses"]:
                pairs = []
                for label in ["before", "after"]:
                    path = directory / f"compat-{arm}-{label}-{seed}"
                    s, _ = common.run_one(OUT / f"{label}.exe", OUT / "configs" / f"{arm}.toml", path, seed, threads=1)
                    pairs.append((s, path))
                assert common.sha(pairs[0][1] / "trades.csv") == common.sha(pairs[1][1] / "trades.csv")
                assert common.sha(pairs[0][1] / "news.csv") == common.sha(pairs[1][1] / "news.csv")
                state = lambda s: [(a["cash"], a["shares"], a["equity"], a["news_received_by_stage"]) for a in s["accounts"]]
                assert state(pairs[0][0]) == state(pairs[1][0])
        print("Performance and legacy off/pulse compatibility passed.", flush=True)
    else:
        print("Performance complete; legacy compatibility not run (before.exe absent).", flush=True)


def report():
    common.OUT = OUT
    cfg = __import__("tomllib").loads((OUT / "configs/trend.toml").read_text())
    initial = [s["dividend"] for s in cfg["stocks"]]
    names = [s["name"] for s in cfg["stocks"]]
    worlds = {}
    for arm in ARMS:
        rows = []
        for seed in SEEDS:
            row = common.world_row(arm, seed)
            path = OUT / arm / f"seed-{seed}"
            s = common.read(path / "summary.json")
            row["news_events"] = s["news_events"]
            row["bad_debt_cents"] = sum(a["stats"]["bad_debt"] for a in s["accounts"])
            affected = [0] * len(names)
            effective = [0] * len(names)
            with (path / "news.csv").open() as handle:
                for event in csv.DictReader(handle):
                    for change in event["changes"].split():
                        i = names.index(change.split(":")[0])
                        affected[i] += 1
                        effective[i] += int(event["effective"]) < cfg["ticks"]
            row["news_per_stock"] = affected
            row["effective_news_per_stock"] = effective
            regime = row["dragon"]
            if regime:
                chosen = regime["selected_stock"]
                previous = [0] * len(initial)
                positive = []; negative = []; news_counts = [0] * len(initial)
                for e in regime["episodes"]:
                    stock = e["stock"]; delta = e["extra_rate"] - previous[stock]
                    fractional = delta / (initial[stock] + previous[stock])
                    if delta > 0: positive.append(fractional)
                    if delta < 0: negative.append(fractional)
                    news_counts[stock] += delta != 0
                    previous[stock] = e["extra_rate"]
                row["growth_updates"] = dict(per_stock=news_counts, max_positive=max(positive, default=0),
                      median_positive=stats.median(positive) if positive else 0,
                      max_negative=min(negative, default=0), selected_extra_multiple=previous[chosen]/initial[chosen] if chosen is not None else None)
                row["dragon"] = {k: v for k, v in regime.items() if k != "episodes"}
            prices = [[] for _ in initial]; dividends = [[] for _ in initial]
            with (path / "bars.csv").open() as handle:
                for bar in csv.DictReader(handle):
                    i = names.index(bar["stock"])
                    p = int(bar["close"]) if bar["close"] else (prices[i][-1] if prices[i] else None)
                    prices[i].append(p); dividends[i].append(int(bar["dividend"]))
            row["price_jump_counts"] = {str(w): sum(any(b / a >= 2 for a, b in zip(ps, ps[w:]) if a and b) for ps in prices) for w in [20, 100]}
            if regime and regime["selected_stock"] is not None:
                i = regime["selected_stock"]
                row["selected_path"] = dict(stock=names[i], final_multiple=dividends[i][-1]/initial[i], peak_multiple=max(dividends[i])/initial[i],
                     largest_price_up_20=max((b/a for a,b in zip(prices[i],prices[i][20:]) if a and b), default=1.0),
                     largest_dividend_up=max((b-a)/max(a,1) for a,b in zip(dividends[i],dividends[i][1:])))
            rows.append(row)
        worlds[arm] = rows
    summaries = {arm: common.summarize_worlds(ws) for arm, ws in worlds.items()}
    for arm, ws in worlds.items():
        summaries[arm]["news_events"] = stats.mean(w["news_events"] for w in ws)
        summaries[arm]["double_in_20"] = sum(w["price_jump_counts"]["20"] for w in ws)
        summaries[arm]["double_in_100"] = sum(w["price_jump_counts"]["100"] for w in ws)
        summaries[arm]["quality_failures"] = dict(Counter(c["name"] for w in ws for c in w["quality"]["checks"] if not c["pass"]))
        summaries[arm]["mean_news_per_stock"] = stats.mean(n for w in ws for n in w["news_per_stock"])
        summaries[arm]["mean_effective_news_per_stock"] = stats.mean(n for w in ws for n in w["effective_news_per_stock"])
        summaries[arm]["bad_debt_cents"] = sum(w["bad_debt_cents"] for w in ws)
    paired = {}
    for left in ["off", "pulses"]:
        paired["trend_minus_" + left] = {
            g: {metric: common.delta_summary([b["groups"][g][metric] - a["groups"][g][metric]
                                             for a, b in zip(worlds[left], worlds["trend"])])
                for metric in ["return_pct", "percentile"]}
            for g in summaries["trend"]["groups"]}
    perf = common.read(OUT / "performance.json")
    performance_summary = {arm: {key: stats.median(r[key] for r in perf if r["arm"] == arm) for key in ["process_seconds", "engine_seconds"]} for arm in ARMS}
    data = dict(plan=common.read(OUT / "plan.json"), manifest=common.read(OUT / "manifest.json"),
                summary=summaries, worlds=worlds, paired=paired, performance=perf,
                performance_summary=performance_summary, analysis_tool_sha256=common.sha(Path(__file__)))
    common.write(ROOT / "docs/baseline-v7-results.json", data)
    for arm, s in summaries.items():
        print(arm, "news", s["news_events"], "quality", s["quality_passes"], "double100", s["double_in_100"], "Titan", s["groups"]["titan-value"]["return_pct"])


def main():
    global OUT
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("command", choices=["prepare", "run", "performance", "report"])
    p.add_argument("--out", type=Path, default=OUT)
    a = p.parse_args(); OUT = a.out.resolve()
    if os.name == "nt":
        print("Windows CPU budget:", windows_build_budget(), flush=True)
    globals()[a.command]()


if __name__ == "__main__":
    main()
