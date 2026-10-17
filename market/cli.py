"""Command line: python -m market run SCENARIO --out DIR"""

import argparse
import json
import tomllib
from pathlib import Path

from .simulation import Simulation, load_config


def main() -> None:
    ap = argparse.ArgumentParser(prog="python -m market")
    sub = ap.add_subparsers(dest="cmd", required=True)
    run = sub.add_parser("run", help="run one scenario")
    run.add_argument("scenario", type=Path)
    run.add_argument("--out", type=Path, required=True, help="output directory (must not exist)")
    run.add_argument("--seed", type=int, help="override the scenario seed")
    run.add_argument("--ticks", type=int, help="override the scenario length")
    run.add_argument("--set", action="append", default=[], metavar="KEY=VALUE",
                     help="override a scenario value, e.g. --set fees.carry_rate=0 (TOML value syntax)")
    run.add_argument("--no-report", action="store_true", help="skip the PNG report")
    args = ap.parse_args()

    cfg = load_config(args.scenario)
    if args.seed is not None:
        cfg["seed"] = args.seed
    if args.ticks is not None:
        cfg["ticks"] = args.ticks
    for item in args.set:
        key, _, raw = item.partition("=")
        *parents, leaf = key.split(".")
        node = cfg
        for k in parents:
            node = node.setdefault(k, {})
        node[leaf] = tomllib.loads(f"v = {raw}")["v"]
    res = Simulation(cfg, args.out).run()
    if not args.no_report:
        try:
            from .report import write_report
            write_report(args.out)
        except ImportError:
            print("matplotlib not installed; skipped report.png")
    print(json.dumps({k: res[k] for k in ("ticks", "accounts_n", "wall_seconds", "final_marks",
                                          "final_dividend", "news_events")}, ensure_ascii=False))
    print(f"{'strategy':<12}{'n':>4}{'pnl_vs_start':>16}{'volume':>16}{'carry_fees':>12}{'dividends':>14}")
    for name, g in sorted(res["by_strategy"].items(), key=lambda kv: -kv[1]["pnl_vs_start"] / kv[1]["accounts"]):
        n = g["accounts"]
        print(f"{name:<12}{n:>4}{g['pnl_vs_start'] / n:>16.0f}{g['volume_notional'] / n:>16.0f}"
              f"{g['carry_fees'] / n:>12.0f}{g['dividends'] / n:>14.0f}")
    disabled = [a for a in res["accounts"] if a["disabled"]]
    for a in disabled:
        print(f"disabled {a['account']}: {a['disabled'][:200]}")
