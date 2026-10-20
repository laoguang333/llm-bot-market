"""Read frozen per-tick study outputs; no simulation or parameter selection."""
from collections import Counter
import hashlib
import json
from pathlib import Path
import statistics as stats

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "runs/per-tick-20261019"


def read(path):
    return json.loads(path.read_text(encoding="utf-8"))


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def mean(xs):
    return stats.mean(xs)


def account_rows(accounts):
    returns = [(a["return_ratio"]-1)*100 for a in accounts]
    profit = [a["equity"]-a["initial_equity"] for a in accounts]
    return dict(n=len(accounts), return_mean=mean(returns), return_median=stats.median(returns),
                return_sd=stats.stdev(returns) if len(returns)>1 else 0,
                losses=sum(x<0 for x in returns), percentile_mean=mean(a["percentile"] for a in accounts),
                top25=sum(a["percentile"]>=75 for a in accounts),
                profit_yuan=mean(profit)/100,
                dividends_yuan=mean(a["stats"]["dividends"] for a in accounts)/100,
                combo_yuan=mean(a["stats"]["combo_dividends"] for a in accounts)/100,
                costs_yuan=mean(sum(a["stats"][k] for k in ["trade_fees","idle_fees","interest"]) for a in accounts)/100,
                trading_inventory_residual_yuan=mean(
                    p-a["stats"]["dividends"]-a["stats"]["combo_dividends"]
                    +sum(a["stats"][k] for k in ["trade_fees","idle_fees","interest"])
                    for p,a in zip(profit,accounts))/100,
                rejects=sum(a["stats"]["rejects"] for a in accounts),
                liquidations=sum(a["stats"]["liquidations"] for a in accounts))


def main():
    plan = read(OUT / "plan.json")
    performance, raw_perf = {}, []
    paths = {}
    for i,j in enumerate(plan["performance"]):
        d = OUT / "performance" / f"{i:03}-{j['condition']}-t{j['threads']}-s{j['seed']}-r{j['repeat']}"
        result = read(d / "summary.json")
        paths[j["condition"],j["threads"],j["seed"],j["repeat"]] = d
        if j["warmup"]:
            continue
        raw_perf.append(dict(**j, engine_seconds=result["wall_seconds"],
                             process_seconds=read(d.with_suffix(".done.json"))["wall_seconds"],
                             trades=result["trades"]))
    for threads in [1,4]:
        for condition in plan["conditions"]:
            rows = [r for r in raw_perf if r["threads"]==threads and r["condition"]==condition]
            performance[f"t{threads}-{condition}"] = dict(n=len(rows),
                engine_mean=mean(r["engine_seconds"] for r in rows),
                engine_median=stats.median(r["engine_seconds"] for r in rows),
                process_mean=mean(r["process_seconds"] for r in rows),
                trades_mean=mean(r["trades"] for r in rows))
    compatibility=[]
    for seed in range(22001,22006):
        for repeat in range(1,4):
            a=paths["old-periodic",1,seed,repeat]
            b=paths["new-periodic",1,seed,repeat]
            ra,rb=read(a/"summary.json"),read(b/"summary.json")
            compatibility.append(dict(seed=seed,repeat=repeat,
                same_trades=digest(a/"trades.csv")==digest(b/"trades.csv"),
                same_account_equity=[x["equity"] for x in ra["accounts"]]==[x["equity"] for x in rb["accounts"]]))
    markets={}
    accounting=[]
    for job in plan["markets"]:
        d=OUT/"market"/job["id"]
        read(d.with_suffix(".done.json"))
        worlds=[read(d/f"seed-{s}"/"summary.json") for s in range(plan["market_first_seed"],plan["market_first_seed"]+plan["market_seeds"])]
        quality={}
        for check in worlds[0]["quality"]["checks"]:
            rows=[next(c for c in w["quality"]["checks"] if c["name"]==check["name"]) for w in worlds]
            quality[check["name"]]=dict(median=stats.median(c["value"] for c in rows),
                                      passes=sum(c["pass"] for c in rows),threshold=check["threshold"])
        totals=Counter()
        for w in worlds:
            cash=sum(a["cash"] for a in w["accounts"])+w["treasury"]-sum(a["initial_cash"] for a in w["accounts"])
            share=[sum(a["shares"][s]-a["initial_shares"][s] for a in w["accounts"])+w["shares_burned"][s] for s in range(len(w["shares_burned"]))]
            accounting.append(dict(group=job["id"],seed=w["seed"],cash_error=cash,share_errors=share,audits=w["audits"]))
            totals["disabled"]+=sum(bool(a["disabled"]) for a in w["accounts"])
            for a in w["accounts"]:
                for k in ["skipped","bot_errors","liquidations","bad_debt","rejects","decisions","dividends","combo_dividends","volume_notional"]:
                    totals[k]+=a["stats"][k]
        by_strategy={}
        by_profile={}
        # Average per world first, so randomized population sizes do not reweight worlds.
        for strategy in sorted({a["strategy"] for w in worlds for a in w["accounts"]}):
            groups=[[a for a in w["accounts"] if a["strategy"]==strategy and a["owner"]!="challenger"] for w in worlds]
            groups=[g for g in groups if g]
            if groups:
                by_strategy[strategy]=dict(return_mean=mean(mean((a["return_ratio"]-1)*100 for a in g) for g in groups),
                                           percentile_mean=mean(mean(a["percentile"] for a in g) for g in groups))
        for owner in ["swift-standard","swift-early","swift-selective","swift-flow","swift-depth","challenger"]:
            accounts=[a for w in worlds for a in w["accounts"] if a["owner"]==owner]
            if accounts:
                by_profile[owner]=account_rows(accounts)
        markets[job["id"]]=dict(n=len(worlds),quality_pass=sum(w["quality"]["pass"] for w in worlds),quality=quality,
            trades_mean=mean(w["trades"] for w in worlds),engine_seconds=sum(w["wall_seconds"] for w in worlds),
            command_seconds=read(d.with_suffix(".done.json"))["wall_seconds"],totals=dict(totals),
            by_strategy=by_strategy,by_profile=by_profile,
            failed_quality=[dict(seed=w["seed"],checks=[c["name"] for c in w["quality"]["checks"] if not c["pass"]]) for w in worlds if not w["quality"]["pass"]])
    world_checks=[]
    for job in plan["markets"]:
        if not job["id"].endswith("periodic"):
            continue
        other=job["id"].removesuffix("periodic")+"per-tick"
        for seed in range(plan["market_first_seed"],plan["market_first_seed"]+plan["market_seeds"]):
            a=OUT/"market"/job["id"]/f"seed-{seed}"
            b=OUT/"market"/other/f"seed-{seed}"
            ra,rb=read(a/"summary.json"),read(b/"summary.json")
            world_checks.append(dict(group=job["id"],seed=seed,
                same_news=digest(a/"news.csv")==digest(b/"news.csv"),
                same_terminal_units=ra["terminal_payments"]==rb["terminal_payments"],
                same_final_rate_units=ra["final_dividends"]==rb["final_dividends"]))
    paired_returns={}
    for family in ["v5","v6"]:
        for bot in ["swift","news_v2"]:
            changes=[]
            for seed in range(plan["market_first_seed"],plan["market_first_seed"]+plan["market_seeds"]):
                pair=[next(a for a in read(OUT/"market"/f"{family}-{bot}-{regime}"/f"seed-{seed}"/"summary.json")["accounts"]
                           if a["owner"]=="challenger") for regime in ["periodic","per-tick"]]
                changes.append((pair[1]["return_ratio"]-pair[0]["return_ratio"])*100)
            delta=mean(changes)
            # Two-sided t interval for 20 paired worlds (19 degrees of freedom).
            assert len(changes)==20
            error=2.093*stats.stdev(changes)/len(changes)**0.5
            paired_returns[f"{family}-{bot}"]=dict(mean_change_pp=delta,t95_interval=[delta-error,delta+error],
                note="Across-world paired uncertainty; one concurrent run per condition, not a separate scheduling variance estimate.")
    report=dict(plan="per-tick-plan.json",performance=performance,performance_runs=raw_perf,
                compatibility=compatibility,markets=markets,accounting=accounting,world_checks=world_checks,
                paired_returns=paired_returns)
    path=ROOT/"docs/per-tick-results.json"
    path.write_text(json.dumps(report,ensure_ascii=False,indent=2)+"\n",encoding="utf-8")
    print(json.dumps(dict(performance=performance,
        compatibility_all=all(c["same_trades"] and c["same_account_equity"] for c in compatibility),
        world_checks_all=all(c["same_news"] and c["same_terminal_units"] and c["same_final_rate_units"] for c in world_checks),
        cash_errors=sum(bool(c["cash_error"]) for c in accounting),
        share_errors=sum(any(c["share_errors"]) for c in accounting),
        markets={k:dict(quality_pass=v["quality_pass"],trades=v["trades_mean"],totals=v["totals"],
                       challenger=v["by_profile"].get("challenger")) for k,v in markets.items()}),ensure_ascii=False,indent=2))


if __name__=="__main__":
    main()
