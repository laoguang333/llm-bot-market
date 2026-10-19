"""Serial release comparison of 3000 / 5000 / 10000 tick markets.

Run prepare once, then pilot/bench/batch through arena_gate.py, then report.
All bot implementations are unchanged. These are timing/cadence checks, not
challenger acceptance or a fixed-economic-environment comparison.
"""
import argparse
import csv
import json
import os
from pathlib import Path
import platform
import shutil
import statistics as stats
import time
import tomllib
import zipfile

from build_release import windows_build_budget
import research_v7 as common

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "runs/horizon-study"
ARMS = ["current3000", "compact5000", "long10000"]
PERF_SEEDS = [7, 11, 27003, 27006]
BATCH_SEEDS = list(range(28001, 28021))


def scenario(text, ticks, rate):
    text = text.replace("ticks = 3000\n", f"ticks = {ticks}\n", 1)
    text = text.replace("events_per_1000_ticks = 20.0\n", f"events_per_1000_ticks = {rate:.1f}\n", 1)
    # Preserve the relative locations of the existing terminal information.
    text = text.replace("terminal_hints = [[0, 30], [1000, 16], [2000, 6]]",
                        f"terminal_hints = [[0, 30], [{ticks // 3}, 16], [{ticks * 2 // 3}, 6]]")
    return text


def prepare():
    OUT.mkdir(parents=True, exist_ok=True)
    folder = OUT / "configs"
    folder.mkdir(exist_ok=False)
    text = (ROOT / "scenarios/baseline-v7.toml").read_text(encoding="utf-8")
    configs = dict(current3000=text, compact5000=scenario(text, 5000, 20.0), long10000=scenario(text, 10000, 5.0))
    for arm, source in configs.items():
        (folder / f"{arm}.toml").write_text(source, encoding="utf-8")
        cfg = tomllib.loads(source)
        assert cfg["bots"] == tomllib.loads(text)["bots"]
    shutil.copy2(ROOT / "target/release/llm-market.exe", OUT / "market.exe")
    sources = sorted((ROOT / "crates").rglob("*.rs")) + sorted((ROOT / "crates").rglob("Cargo.toml"))
    sources += [ROOT / "Cargo.toml", ROOT / "Cargo.lock", Path(__file__), ROOT / "tools/research_v7.py"]
    common.write(OUT / "manifest.json", dict(source={str(p.relative_to(ROOT)): common.sha(p) for p in sources},
        binary_sha256=common.sha(OUT / "market.exe"), configs={arm: common.sha(folder / f"{arm}.toml") for arm in ARMS}))
    with zipfile.ZipFile(OUT / "frozen-sources.zip", "x") as archive:
        for p in sources:
            archive.write(p, str(p.relative_to(ROOT)))
        for p in folder.iterdir():
            archive.write(p, "configs/" + p.name)
    common.write(OUT / "plan.json", dict(performance_seeds=PERF_SEEDS, repeats=3,
        batch_seeds=BATCH_SEEDS, threads=4, cpu_budget=max(1, (os.cpu_count() or 1)//2),
        platform=platform.platform(), bot_changes=False,
        notes="5000 ticks targets approximately 50 ticks/news/stock, 10000 targets 100. Only trend event rate is calibrated; ordinary news, fees, capital, strategies unchanged. Terminal hints retain relative thirds. Counts deduplicate disclosure stages and include market/sector impacts."))


def one(arm, directory, seed, threads=4):
    cfg_path = OUT / "configs" / f"{arm}.toml"
    cfg = tomllib.loads(cfg_path.read_text(encoding="utf-8"))
    if directory.exists():
        raise RuntimeError(f"Refusing to overwrite {directory}")
    s, wall = common.run_one(OUT / "market.exe", cfg_path, directory, seed, threads)
    assert s["audits"] >= cfg["ticks"] // 500
    accounts = s["accounts"]
    names = [v["name"] for v in cfg["stocks"]]
    counts = dict.fromkeys(names, 0)
    effective = dict.fromkeys(names, 0)
    with (directory / "news.csv").open() as handle:
        for event in csv.DictReader(handle):
            for change in event["changes"].split():
                name = change.split(":")[0]
                counts[name] += 1
                effective[name] += int(event["effective"]) < cfg["ticks"]
    return dict(arm=arm, seed=seed, ticks=cfg["ticks"], threads=threads, accounts=len(accounts),
        process_seconds=wall, engine_seconds=s["wall_seconds"], news_events=s["news_events"],
        stock_news=counts, effective_stock_news=effective,
        mean_stock_interval=cfg["ticks"] / stats.mean(counts.values()),
        audits=s["audits"], skipped=sum(a["stats"]["skipped"] for a in accounts),
        errors=sum(a["stats"]["bot_errors"] for a in accounts),
        disabled=sum(bool(a["disabled"]) for a in accounts),
        liquidations=sum(a["stats"]["liquidations"] for a in accounts),
        bad_debt_cents=sum(a["stats"]["bad_debt"] for a in accounts),
        trades=s["trades"], output_bytes=sum(p.stat().st_size for p in directory.rglob("*") if p.is_file()),
        groups={g: common.summarize_accounts([a for a in accounts if common.group_key(a)==g])
                for g in sorted({common.group_key(a) for a in accounts})}, quality=s["quality"])


def pilot():
    records = []
    for seed in [7, 27003, 27006]:
        for arm in ["compact5000", "long10000"]:
            record = one(arm, OUT / "pilot" / f"{arm}-{seed}", seed)
            records.append(record)
            print(arm, seed, f"{record['process_seconds']:.3f}s / {record['mean_stock_interval']:.1f} ticks per stock-news", flush=True)
    common.write(OUT / "pilot.json", records)


def bench():
    records = []
    for arm in ARMS:
        one(arm, OUT / "performance" / f"warm-{arm}", PERF_SEEDS[0])
    for seed in PERF_SEEDS:
        for rep in range(3):
            for arm in (ARMS if rep%2 else ARMS[::-1]):
                r = one(arm, OUT / "performance" / f"{arm}-{seed}-{rep}", seed)
                r["repeat"] = rep
                records.append(r)
                print(arm, seed, rep, f"{r['process_seconds']:.3f}s", flush=True)
    common.write(OUT / "performance.json", records)


def batch():
    records = []
    start = time.perf_counter()
    for seed in BATCH_SEEDS:
        r = one("long10000", OUT / "batch" / f"seed-{seed}", seed)
        records.append(r)
        print(seed, f"{r['process_seconds']:.3f}s, interval {r['mean_stock_interval']:.1f}, skipped {r['skipped']}", flush=True)
    common.write(OUT / "batch.json", dict(wall_seconds=time.perf_counter()-start, records=records))


def describe(rows):
    return dict(runs=len(rows), process_median=stats.median(r["process_seconds"] for r in rows),
        process_min=min(r["process_seconds"] for r in rows), process_max=max(r["process_seconds"] for r in rows),
        engine_median=stats.median(r["engine_seconds"] for r in rows),
        stock_news_mean=stats.mean(v for r in rows for v in r["stock_news"].values()),
        mean_stock_interval=rows[0]["ticks"]/stats.mean(v for r in rows for v in r["stock_news"].values()),
        audits=sum(r["audits"] for r in rows), skipped=sum(r["skipped"] for r in rows),
        errors=sum(r["errors"] for r in rows), disabled=sum(r["disabled"] for r in rows),
        liquidations=sum(r["liquidations"] for r in rows), bad_debt_cents=sum(r["bad_debt_cents"] for r in rows),
        runs_with_liquidation=sum(r["liquidations"] > 0 for r in rows),
        runs_with_bad_debt=sum(r["bad_debt_cents"] > 0 for r in rows),
        quality_passes=sum(r["quality"]["pass"] for r in rows),
        liquidity_passes=sum(all(c["pass"] for c in r["quality"]["checks"][:5]) for r in rows),
        output_mib_mean=stats.mean(r["output_bytes"] for r in rows)/2**20,
        group_returns={g: stats.mean(r["groups"][g]["return_pct"] for r in rows) for g in rows[0]["groups"]})


def report():
    perf = common.read(OUT / "performance.json")
    batch_data = common.read(OUT / "batch.json")
    summaries = {arm: describe([r for r in perf if r["arm"]==arm]) for arm in ARMS}
    data = dict(plan=common.read(OUT / "plan.json"), manifest=common.read(OUT / "manifest.json"),
        analysis_sha256=common.sha(Path(__file__)), performance_summary=summaries, performance=perf,
        batch_summary=describe(batch_data["records"]), batch=batch_data)
    common.write(ROOT / "docs/horizon-performance-results.json", data)
    print(json.dumps(dict(performance=summaries, batch=data["batch_summary"], batch_wall=batch_data["wall_seconds"]),indent=2))


def main():
    global OUT
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("command", choices=["prepare", "pilot", "bench", "batch", "report"])
    p.add_argument("--out", type=Path, default=OUT)
    a=p.parse_args(); OUT=a.out.resolve()
    if os.name=="nt":
        print("Shared CPU budget:", windows_build_budget(), flush=True)
    globals()[a.command]()


if __name__=="__main__":
    main()
