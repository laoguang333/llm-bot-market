"""Evaluate three frozen bots separately, then together with all six seat orders.

Run after development stops, through arena_gate.py. Reports are kept under --out.
An interrupted run can resume only with the identical binary and source manifest.
"""

import argparse
from collections import Counter
from datetime import datetime, timezone
from fractions import Fraction
import hashlib
from itertools import permutations
import json
from pathlib import Path
import statistics
import subprocess


def read(path):
    return json.loads(path.read_text(encoding="utf-8"))


def write(path, value):
    path.write_text(json.dumps(value, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def mean(xs):
    return statistics.mean(xs)


def diagnostic(worlds):
    accounts = [a for world in worlds for a in world["accounts"]]
    return {
        "worlds": len(worlds),
        "quality_pass_share": mean(int(w["quality"]["pass"]) for w in worlds),
        "failed_quality_checks": dict(Counter(c["name"] for w in worlds
                                              for c in w["quality"]["checks"] if not c["pass"])),
        "all_accounts_skipped": sum(a["stats"]["skipped"] for a in accounts),
        "all_accounts_bot_errors": sum(a["stats"]["bot_errors"] for a in accounts),
        "all_accounts_disabled": sum(bool(a["disabled"]) for a in accounts),
        "audits": sum(w["audits"] for w in worlds),
        "wall_seconds": sum(w["wall_seconds"] for w in worlds),
    }


def account_metrics(accounts):
    decisions = sum(a["stats"]["decisions"] for a in accounts)
    pcts = [a["percentile"] for a in accounts]
    return {
        "mean_percentile": mean(pcts),
        "sd_percentile": statistics.stdev(pcts) if len(pcts) > 1 else 0,
        "top25_share": mean(int(p >= 75) for p in pcts),
        "mean_return_ratio": mean(a["return_ratio"] for a in accounts),
        "worst_percentile": min(pcts),
        "skipped": sum(a["stats"]["skipped"] for a in accounts),
        "bot_errors": sum(a["stats"]["bot_errors"] for a in accounts),
        "disabled": sum(bool(a["disabled"]) for a in accounts),
        "liquidations": sum(a["stats"]["liquidations"] for a in accounts),
        "decision_us": sum(a["stats"]["decision_nanos_total"] for a in accounts)
                       / max(decisions, 1) / 1000,
    }


def exact_return(account):
    return Fraction(account["equity"], account["initial_equity"])


def relative_ranks(accounts):
    """Average ranks for ties; compare exact returns instead of rounded floats."""
    returns = {bot: exact_return(a) for bot, a in accounts.items()}
    return {bot: 1 + sum(v > value for v in returns.values())
                 + (sum(v == value for v in returns.values()) - 1) / 2
            for bot, value in returns.items()}


def run_batch(root, binary, scenario, target, first_seed, count, bot=None):
    command = [str(binary), "challenge" if bot else "series", str(scenario),
               "--out", str(target), "--seeds", str(count), "--first-seed", str(first_seed)]
    if bot:
        command += ["--bot", bot]
    result_file = target / ("challenge.json" if bot else "series.json")
    if result_file.exists():
        print(f"Resume completed batch: {target.name}", flush=True)
    elif target.exists():
        raise RuntimeError(f"Incomplete batch at {target}; preserve it and use a fresh --out")
    else:
        print(f"Running {target.name}", flush=True)
        write(target.with_suffix(".command.json"), command)
        with target.with_suffix(".log").open("w", encoding="utf-8") as log:
            subprocess.run(command, cwd=root, stdout=log, stderr=subprocess.STDOUT, check=True)
    return [read(target / f"seed-{seed}" / "summary.json")
            for seed in range(first_seed, first_seed + count)]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", required=True)
    parser.add_argument("--bots", nargs=3, default=["astra_a", "astra_b", "astra_c"])
    parser.add_argument("--seeds", type=int, default=20)
    parser.add_argument("--single-first", type=int, default=1001)
    parser.add_argument("--validation-first", type=int, default=2001)
    parser.add_argument("--arena-first", type=int, default=3001)
    args = parser.parse_args()
    if len(set(args.bots)) != 3 or args.seeds < 1:
        parser.error("three distinct bots and at least one seed are required")
    root = Path(__file__).resolve().parents[1]
    out = (root / args.out).resolve()
    scenario = root / "scenarios/baseline.toml"
    binary = root / "target/release/llm-market.exe"
    if not binary.exists():
        binary = binary.with_suffix("")
    available = subprocess.check_output([str(binary), "bots"], text=True).splitlines()
    if not set(args.bots).issubset(available):
        parser.error("build all three bots before evaluating")
    paths = [root / "Cargo.toml", root / "Cargo.lock", scenario, Path(__file__).resolve()]
    paths += sorted((root / "crates").rglob("*.rs"))
    paths += sorted((root / "crates").rglob("Cargo.toml"))
    protocol = {
        "bots": args.bots, "seeds_per_batch": args.seeds,
        "single_first": args.single_first, "validation_first": args.validation_first,
        "arena_first": args.arena_first,
        "arena_orders": list(permutations(args.bots)),
        "ranking": "mean within-trio rank ascending; tie-break mean return ratio descending",
        "cash": 2_000_000, "shares_each": 20,
    }
    # Normalize tuples before comparison with a reloaded JSON manifest.
    protocol = json.loads(json.dumps(protocol))
    frozen = {"protocol": protocol, "binary_sha256": digest(binary),
              "sources": {str(p.relative_to(root)): digest(p) for p in paths}}
    out.mkdir(parents=True, exist_ok=True)
    manifest = out / "manifest.json"
    if manifest.exists():
        if read(manifest) != frozen:
            raise RuntimeError("Code, binary, or protocol changed; use a fresh output directory")
    else:
        write(manifest, frozen)
    result = {"created_utc": datetime.now(timezone.utc).isoformat(),
              "manifest": frozen, "single": {}, "arena": {}}
    for label, first in [("official", args.single_first), ("validation", args.validation_first)]:
        result["single"][label] = {}
        for bot in args.bots:
            worlds = run_batch(root, binary, scenario, out / f"{label}-{bot}", first, args.seeds, bot)
            accounts = [next(a for a in w["accounts"] if a["owner"] == "challenger") for w in worlds]
            metrics = account_metrics(accounts)
            metrics["pass"] = metrics["mean_percentile"] >= 90 and metrics["top25_share"] >= 0.8
            metrics["per_world"] = [{"seed": w["seed"], "percentile": a["percentile"],
                                     "return_ratio": a["return_ratio"]} for w, a in zip(worlds, accounts)]
            metrics["diagnostics"] = diagnostic(worlds)
            result["single"][label][bot] = metrics
            write(out / "results.json", result)
            print(f"{label} {bot}: {metrics['mean_percentile']:.2f}, "
                  f"top25={metrics['top25_share']:.0%}, pass={metrics['pass']}", flush=True)
    arena_worlds, matches = [], []
    entrant_accounts = {bot: [] for bot in args.bots}
    pairwise = {bot: {other: {"wins": 0, "ties": 0, "losses": 0}
                      for other in args.bots if other != bot} for bot in args.bots}
    for i, order in enumerate(protocol["arena_orders"]):
        arena_scenario = out / f"arena-order-{i}.toml"
        text = scenario.read_text(encoding="utf-8")
        for bot in order:
            text += f'\n[[bots]]\nbot = "{bot}"\nowner = "entrant-{bot}"\ncount = 1\ncash = 2_000_000\nshares_each = 20\n'
        arena_scenario.write_text(text, encoding="utf-8")
        worlds = run_batch(root, binary, arena_scenario, out / f"arena-order-{i}",
                           args.arena_first, args.seeds)
        arena_worlds.extend(worlds)
        for w in worlds:
            accounts = {bot: next(a for a in w["accounts"] if a["owner"] == f"entrant-{bot}")
                        for bot in args.bots}
            if len({a["initial_equity"] for a in accounts.values()}) != 1:
                raise RuntimeError("Entrants must have equal starting equity")
            ranks = relative_ranks(accounts)
            matches.append({"seed": w["seed"], "order": order, "rank": ranks,
                            "percentile": {b: a["percentile"] for b, a in accounts.items()},
                            "return_ratio": {b: a["return_ratio"] for b, a in accounts.items()}})
            for bot, a in accounts.items():
                entrant_accounts[bot].append(a)
                for other, b in accounts.items():
                    if bot == other:
                        continue
                    comparison = (exact_return(a) > exact_return(b)) - (exact_return(a) < exact_return(b))
                    pairwise[bot][other][{1: "wins", 0: "ties", -1: "losses"}[comparison]] += 1
    metrics = {}
    for bot, accounts in entrant_accounts.items():
        metrics[bot] = account_metrics(accounts)
        metrics[bot]["mean_trio_rank"] = mean(m["rank"][bot] for m in matches)
        metrics[bot]["outright_firsts"] = sum(m["rank"][bot] == 1 for m in matches)
        metrics[bot]["per_seed_mean_rank"] = {
            str(seed): mean(m["rank"][bot] for m in matches if m["seed"] == seed)
            for seed in range(args.arena_first, args.arena_first + args.seeds)}
    ranking = sorted(args.bots, key=lambda b: (metrics[b]["mean_trio_rank"], -metrics[b]["mean_return_ratio"]))
    result["arena"] = {"ranking": ranking, "bots": metrics, "pairwise": pairwise,
                       "matches": matches, "diagnostics": diagnostic(arena_worlds),
                       "independent_world_seeds": args.seeds,
                       "note": "Six seat orders per seed are correlated, not independent worlds."}
    if any(digest(root / p) != value for p, value in frozen["sources"].items()) or digest(binary) != frozen["binary_sha256"]:
        raise RuntimeError("Frozen inputs changed during evaluation")
    write(out / "results.json", result)
    print(json.dumps({"ranking": ranking, "bots": metrics,
                      "diagnostics": result["arena"]["diagnostics"]}, ensure_ascii=False), flush=True)


if __name__ == "__main__":
    main()
