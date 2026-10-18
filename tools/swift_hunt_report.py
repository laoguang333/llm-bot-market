"""Aggregate frozen single-predator experiments, clustering both orders by seed."""
import argparse
import json
from pathlib import Path
import statistics as stats

from swift_hunt import BASE, ROOT, summarize
from swift_stress_report import mean_ci


def cluster_ci(rows, value, seed=18001):
    groups = {}
    for r in rows:
        groups.setdefault(r["seed"], []).append(value(r))
    means = [stats.mean(groups[k]) for k in sorted(groups)]
    return mean_ci(means, seed), len(means)


def aggregate(rows):
    result = summarize(rows)
    for owner in ["target", "predator"]:
        these = [r for r in rows if r[owner]]
        if not these:
            continue
        ci,n = cluster_ci(these, lambda r: r[owner]["return_pct"])
        result[owner]["return_ci95"] = ci
        result[owner]["independent_seeds"] = n
        for key in ["mean_gross_leverage", "turnover", "dividend_pct"]:
            result[owner][key] = stats.mean(r[owner][key] for r in these)
        other = "predator" if owner == "target" else "target"
        total = sum(r[owner]["stats"]["volume_notional"] for r in these)
        direct = sum(r[owner]["counterparty_notional"].get(other,0) for r in these)
        result[owner]["direct_counterparty_notional_share"] = direct/total if total else 0
        result[owner]["rejects"] = sum(r[owner]["stats"]["rejects"] for r in these)
        result[owner]["liquidations"] = sum(r[owner]["stats"]["liquidations"] for r in these)
        for fee in ["trade_fees","idle_fees","interest"]:
            result[owner][fee+"_pct"] = stats.mean(r[owner]["stats"][fee]/r[owner]["initial_equity"]*100 for r in these)
    paired = [r for r in rows if r["predator"]]
    if paired:
        ci,n = cluster_ci(paired, lambda r:r["predator"]["return_pct"]-r["target"]["return_pct"])
        result["advantage_ci95"] = ci
        result["independent_seeds"] = n
    return result


def build(stages, partial=False):
    plans, rows = [], []
    for stage in stages:
        plan = json.loads((BASE / stage / "plan.json").read_text(encoding="utf-8"))
        plans.append(plan)
        for job in plan["jobs"]:
            path = BASE / stage / job["id"] / "done.json"
            if not path.exists():
                if partial:
                    continue
                raise SystemExit(f"Missing {path}")
            result = json.loads(path.read_text(encoding="utf-8"))
            rows += [{"stage":stage, **r} for r in result["worlds"]]
    cases = sorted({r["case"] for r in rows})
    combined = [{"case":case, **aggregate([r for r in rows if r["case"]==case])} for case in cases]
    by_order = [{"case":case,"order":order, **aggregate([r for r in rows if r["case"]==case and r["order"]==order])}
                for case in cases for order in sorted({r["order"] for r in rows if r["case"]==case})]
    by_stage = [{"stage":stage,"case":case, **aggregate([r for r in rows if r["case"]==case and r["stage"]==stage])}
                for stage in stages for case in cases if any(r["stage"]==stage and r["case"]==case for r in rows)]
    lookup = {(r["case"],r["seed"],r["order"]):r for r in rows}
    comparisons = []
    for case in cases:
        for reference in ["cash","blind","coral"]:
            if case == reference:
                continue
            pairs = []
            for r in rows:
                if r["case"] != case:
                    continue
                baseline = lookup.get((reference,r["seed"],r["order"]))
                if baseline:
                    pairs.append({"seed":r["seed"],
                                  "target_change":r["target"]["return_pct"]-baseline["target"]["return_pct"],
                                  "predator_change":r["predator"]["return_pct"]-baseline["predator"]["return_pct"]})
            if pairs:
                ci,n = cluster_ci(pairs, lambda r:r["target_change"])
                predator_ci,_ = cluster_ci(pairs, lambda r:r["predator_change"])
                comparisons.append({"case":case,"reference":reference,"independent_seeds":n,
                                    "target_return_change_pp":stats.mean(r["target_change"] for r in pairs),
                                    "target_change_ci95":ci,
                                    "predator_return_change_pp":stats.mean(r["predator_change"] for r in pairs),
                                    "predator_change_ci95":predator_ci})
    return {"stages":stages,"plans":plans,"runs":len(rows),"combined":combined,
            "by_order":by_order,"by_stage":by_stage,"comparisons":comparisons,"worlds":rows,
            "audit_totals":{k:sum(r[k] for r in rows) for k in ["audits","all_skipped","all_errors","all_disabled"]},
            "notes":["Confidence intervals resample seed clusters, averaging both registration orders inside each seed.",
                     "Development and validation must be reported separately. Market paths are endogenous.",
                     "Post-game counterparty identities never enter either strategy."]}


if __name__ == "__main__":
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument("--stages",nargs="+",default=["dev"])
    p.add_argument("--partial",action="store_true")
    p.add_argument("--output",type=Path)
    a=p.parse_args()
    result=build(a.stages,a.partial)
    if a.output:
        a.output.write_text(json.dumps(result,ensure_ascii=False,indent=2)+"\n",encoding="utf-8")
    for r in result["combined"]:
        t=r["target"]["mean_return_pct"]
        h=r.get("predator",{}).get("mean_return_pct")
        print(r["case"],r["runs"],"Swift",round(t,2),"predator",round(h,2) if h is not None else None,
              "wins",r.get("wins"),"gap",round(r.get("mean_advantage_pp",0),2),"CI",r.get("advantage_ci95"))
    print("audits",result["audit_totals"])
