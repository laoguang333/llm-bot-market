"""Analyze the frozen stress suite. All measurements are post-game, not bot inputs."""
import argparse
import csv
import hashlib
import json
import math
from pathlib import Path
import random
import statistics as stats
import tomllib

from swift_stress import DEFAULT_OUT, ROOT


def percentile(values, q):
    xs = sorted(values)
    if not xs:
        return None
    at = (len(xs)-1)*q
    lo, hi = math.floor(at), math.ceil(at)
    return xs[lo] + (xs[hi]-xs[lo])*(at-lo)


def drawdown(values):
    peak, result = 0.0, 0.0
    for value in values:
        peak = max(peak, value)
        if peak > 0:
            result = max(result, (peak-value)/peak)
    return result*100


def mean_ci(values, seed, iterations=3000):
    rng = random.Random(seed)
    n = len(values)
    samples = [sum(rng.choices(values, k=n))/n for _ in range(iterations)]
    return [percentile(samples, .025), percentile(samples, .975)]


def finite_mean(values):
    xs = [x for x in values if x is not None and math.isfinite(x)]
    return stats.mean(xs) if xs else None


def read_world(directory, job, case, seed):
    summary = json.loads((directory / "summary.json").read_text())
    a = next(a for a in summary["accounts"] if a["owner"] == "challenger")
    probe = json.loads((directory / "probe.json").read_text())
    with (directory / "challenger-equity.csv").open() as f:
        trace = [{k: int(v) for k, v in row.items()} for row in csv.DictReader(f)]
    assert len(trace) == summary["ticks"], f"missing observations: {directory}"
    initial = a["initial_equity"]
    wealth = [initial] + [r["equity"] for r in trace]
    books = {}
    with (directory / "books.csv").open() as f:
        for row in csv.DictReader(f):
            if row["bid"] and row["ask"]:
                books.setdefault(row["stock"], []).append((int(row["tick"]), (int(row["bid"])+int(row["ask"]))/2))
    changes, log_returns = [], []
    for points in books.values():
        if len(points) < 2:
            continue
        changes.append((points[-1][1]/points[0][1]-1)*100)
        log_returns.extend(math.log(b[1]/a_[1]) for a_, b in zip(points, points[1:])
                           if b[0]-a_[0] == 10 and a_[1] > 0 and b[1] > 0)
    market_change = stats.median(changes) if changes else None
    if market_change is None:
        regime = "no_two_sided_prices"
    elif market_change < -30:
        regime = "price_down_more_than_30pct"
    elif market_change < -5:
        regime = "price_down_5_to_30pct"
    elif market_change <= 5:
        regime = "price_change_within_5pct"
    else:
        regime = "price_up_more_than_5pct"
    st = a["stats"]
    div = st["dividends"] + st["combo_dividends"]
    mark_scores = []
    own_mark_score = None
    for other in summary["accounts"]:
        marked = other["cash"] + sum(q*p for q, p in zip(other["shares"], summary["final_marks"]))
        numerator, denominator = marked*1_000_000, max(1, other["initial_equity"])
        score = numerator//denominator if numerator >= 0 else -((-numerator)//denominator)
        mark_scores.append(score)
        if other["owner"] == "challenger":
            own_mark_score = score
    mark_percentile = 100*(sum(x < own_mark_score for x in mark_scores)
                          + .5*(sum(x == own_mark_score for x in mark_scores)-1))/max(1, len(mark_scores)-1)
    cp = {}
    for owner, amount in a["notional_by_counterparty"].items():
        name = owner.split("#")[0]
        cp[name] = cp.get(name, 0) + amount
    return {
        "batch": job["id"], "case": case["key"], "seed": seed,
        "percentile": a["percentile"], "rank": a["rank"],
        "mark_to_market_percentile": mark_percentile,
        "return_pct": (a["return_ratio"]-1)*100,
        "net_ex_dividend_return_pct": (a["equity"]-initial-div)/initial*100,
        "dividend_return_pct": div/initial*100,
        "initial_equity_cents": initial, "final_equity_cents": a["equity"],
        "max_drawdown_pct": drawdown(wealth+[a["equity"]]),
        "trading_drawdown_pct": drawdown(wealth),
        "terminal_equity_gap_pct": (a["equity"]-wealth[-1])/initial*100,
        "terminal_value_pct_initial": a["terminal_value"]/initial*100,
        "turnover_multiple": st["volume_notional"]/initial,
        "fees_pct_initial": (st["trade_fees"]+st["idle_fees"]+st["interest"])/initial*100,
        "trade_fees_pct_initial": st["trade_fees"]/initial*100,
        "idle_fees_pct_initial": st["idle_fees"]/initial*100,
        "interest_pct_initial": st["interest"]/initial*100,
        "mean_gross_leverage": stats.mean(r["gross"]/r["equity"] if r["equity"] > 0 else 0 for r in trace),
        "time_in_market_share": sum(r["gross"] > initial*.01 for r in trace)/len(trace),
        "max_leverage": st["max_leverage"],
        "strategy_calls": probe["strategy_calls"], "decisions": st["decisions"],
        "rejects": st["rejects"], "liquidations": st["liquidations"],
        "challenger_errors": st["bot_errors"], "challenger_skipped": st["skipped"],
        "orders": st["orders"], "trades": st["trades"],
        "all_accounts_skipped": sum(x["stats"]["skipped"] for x in summary["accounts"]),
        "all_accounts_errors": sum(x["stats"]["bot_errors"] for x in summary["accounts"]),
        "all_accounts_disabled": sum(bool(x["disabled"]) for x in summary["accounts"]),
        "audits": summary["audits"], "quality_pass": summary["quality"]["pass"],
        "quality": {c["name"]: c["value"] for c in summary["quality"]["checks"]},
        "market_median_price_change_pct": market_change,
        "market_10tick_logreturn_sd_pct": stats.pstdev(log_returns)*100 if log_returns else None,
        "observed_regime": regime, "counterparty_notional": cp,
        "decision_us": st["decision_nanos_total"]/max(1, st["decisions"])/1000,
        "wall_seconds": summary["wall_seconds"],
        "swift_cohort_returns_pct": {x["owner"]: (x["return_ratio"]-1)*100 for x in summary["accounts"]
                                     if x["strategy"] == "swift"} if case["key"] == "crowded" else {},
    }


def summarize(rows, seed):
    p, r = [x["percentile"] for x in rows], [x["return_pct"] for x in rows]
    result = {"worlds": len(rows), "mean_percentile": stats.mean(p),
              "percentile_mean_ci95": mean_ci(p, seed),
              "top25_share": sum(v >= 75 for v in p)/len(p),
              "gate_pass": stats.mean(p) >= 90 and sum(v >= 75 for v in p)/len(p) >= .8,
              "mean_return_pct": stats.mean(r), "return_mean_ci95": mean_ci(r, seed+1),
              "median_return_pct": stats.median(r), "min_return_pct": min(r),
              "loss_worlds": sum(v < 0 for v in r),
              "first_place_worlds": sum(x["rank"] == 1 for x in rows),
              "mean_drawdown_pct": stats.mean(x["max_drawdown_pct"] for x in rows),
              "max_drawdown_pct": max(x["max_drawdown_pct"] for x in rows),
              "p95_drawdown_pct": percentile([x["max_drawdown_pct"] for x in rows], .95),
              "quality_pass_share": stats.mean(x["quality_pass"] for x in rows)}
    for key in ["mark_to_market_percentile", "net_ex_dividend_return_pct", "dividend_return_pct", "turnover_multiple", "fees_pct_initial",
                "trade_fees_pct_initial", "idle_fees_pct_initial", "interest_pct_initial", "mean_gross_leverage",
                "time_in_market_share", "strategy_calls", "decision_us", "market_median_price_change_pct",
                "market_10tick_logreturn_sd_pct", "terminal_value_pct_initial", "terminal_equity_gap_pct"]:
        result[f"mean_{key}"] = finite_mean([x[key] for x in rows])
    for key in ["rejects", "liquidations", "challenger_errors", "challenger_skipped",
                "all_accounts_skipped", "all_accounts_errors", "all_accounts_disabled", "audits"]:
        result[key] = sum(x[key] for x in rows)
    result["quality"] = {key: finite_mean([x["quality"].get(key) for x in rows]) for key in rows[0]["quality"]}
    cp = {}
    for row in rows:
        for name, amount in row["counterparty_notional"].items():
            cp[name] = cp.get(name, 0)+amount
    total = sum(cp.values())
    result["counterparty_notional_share"] = {k: v/total for k, v in sorted(cp.items(), key=lambda x: -x[1])} if total else {}
    cohort = [r["swift_cohort_returns_pct"] for r in rows if r["swift_cohort_returns_pct"]]
    if cohort:
        result["swift_cohort_mean_return_pct"] = stats.mean(stats.mean(c.values()) for c in cohort)
        result["swift_cohort_by_position"] = {
            owner: {"mean_return_pct": stats.mean(c[owner] for c in cohort),
                    "loss_worlds": sum(c[owner] < 0 for c in cohort)}
            for owner in cohort[0]}
    return result


def build_report(out, partial):
    plan = json.loads((out / "plan.json").read_text(encoding="utf-8"))
    cases = {c["key"]: c for c in plan["cases"]}
    rows, batches = [], []
    for j, job in enumerate(plan["jobs"]):
        directory = out / job["id"]
        if not (directory / "done.json").exists():
            if partial:
                continue
            raise SystemExit(f"Not completed: {job['id']}")
        case = cases[job["case"]]
        these = [read_world(directory / f"seed-{seed}", job, case, seed)
                 for seed in range(job["first_seed"], job["first_seed"]+job["seeds"])]
        rows.extend(these)
        batches.append({"id": job["id"], "case": case["key"], "label": case["label"],
                        "group": case["group"], **summarize(these, 41000+j*10)})
    combined = []
    for j, case in enumerate(plan["cases"]):
        these = [r for r in rows if r["case"] == case["key"]]
        if these:
            combined.append({"case": case["key"], "label": case["label"], "group": case["group"],
                             **summarize(these, 51000+j*10)})
    # Common-seed comparisons include endogenous market reactions; not identical-price replay.
    reference = {r["seed"]: r for r in rows if r["case"] == "v5"}
    paired = []
    for j, case in enumerate(plan["cases"]):
        if case["key"] == "v5":
            continue
        these = [r for r in rows if r["case"] == case["key"] and r["seed"] in reference]
        if not these:
            continue
        delta_r = [r["return_pct"]-reference[r["seed"]]["return_pct"] for r in these]
        delta_p = [r["percentile"]-reference[r["seed"]]["percentile"] for r in these]
        paired.append({"case": case["key"], "worlds": len(these),
                       "return_difference_pp": stats.mean(delta_r), "return_difference_ci95": mean_ci(delta_r, 61000+j*10),
                       "percentile_difference": stats.mean(delta_p), "percentile_difference_ci95": mean_ci(delta_p, 61001+j*10)})
    # Descriptive, post-hoc strata; repeated seed IDs across scenarios are not independent worlds.
    regimes = {}
    main_rows = [r for r in rows if cases[r["case"]]["group"] == "market"]
    for tag in sorted(set(r["observed_regime"] for r in main_rows)):
        these = [r for r in main_rows if r["observed_regime"] == tag]
        regimes[tag] = {"world_runs": len(these), "mean_percentile": stats.mean(r["percentile"] for r in these),
                        "median_return_pct": stats.median(r["return_pct"] for r in these),
                        "loss_worlds": sum(r["return_pct"] < 0 for r in these)}
    return {"plan": plan, "complete": len(batches) == len(plan["jobs"]), "world_runs": len(rows),
            "batches": batches, "combined": combined, "paired_vs_v5": paired,
            "descriptive_price_regimes": regimes, "worlds": rows,
            "statistics_notes": ["95% percentile bootstrap intervals resample world seeds within a case (3,000 draws).",
                                 "Intervals are descriptive and not corrected for multiple comparisons.",
                                 "Final equity includes dividends, financing, fees and terminal settlement.",
                                 "Subtracting dividends from realized P&L is an accounting decomposition, not a no-dividend counterfactual.",
                                 "Drawdown uses observed mark equity and includes final settlement. It is not executable bid-side liquidation value."]}


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--out", type=Path, default=DEFAULT_OUT)
    p.add_argument("--partial", action="store_true")
    a = p.parse_args()
    result = build_report(a.out.resolve(), a.partial)
    name = "swift-stress-partial.json" if a.partial else "swift-stress-results.json"
    target = (a.out if a.partial else ROOT / "docs") / name
    target.write_text(json.dumps(result, ensure_ascii=False, indent=2, allow_nan=False)+"\n", encoding="utf-8")
    for row in result["combined"]:
        print(f"{row['case']:16s} n={row['worlds']:2d} rank={row['mean_percentile']:6.2f} "
              f"return={row['mean_return_pct']:7.1f}% median={row['median_return_pct']:7.1f}% "
              f"loss={row['loss_worlds']:2d} meanDD={row['mean_drawdown_pct']:5.1f}% worstDD={row['max_drawdown_pct']:5.1f}%")
    print(f"Wrote {target}; complete={result['complete']}; world-runs={result['world_runs']}")


if __name__ == "__main__":
    main()
