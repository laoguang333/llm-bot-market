"""World-level statistics and offline diagnostics for the fixed News/Swift study."""
import bisect
from collections import defaultdict
import csv
import hashlib
import json
import os
from pathlib import Path
import random
import statistics as stats
import tomllib

from build_release import windows_build_budget

ROOT=Path(__file__).resolve().parents[1]
OUT=ROOT/"runs/news-swift-20261019"


def read(p): return json.loads(p.read_text(encoding="utf-8"))
def write(p,x): p.write_text(json.dumps(x,ensure_ascii=False,indent=2)+"\n",encoding="utf-8")
def mean(x): return stats.mean(x) if x else None


def interval(values):
    rng=random.Random(423871)
    xs=sorted(sum(rng.choices(values,k=len(values)))/len(values) for _ in range(2000))
    return [xs[49],xs[1949]]


def summarize(rows):
    returns=[r["return_pct"] for r in rows]
    ranks=[r["account"]["percentile"] for r in rows]
    return {"worlds":len(rows),"mean_return_pct":mean(returns),"median_return_pct":stats.median(returns),
            "mean_return_ci95":interval(returns),"mean_percentile":mean(ranks),"top25":sum(x>=75 for x in ranks),
            "pass":mean(ranks)>=90 and sum(x>=75 for x in ranks)/len(ranks)>=0.8,
            "losses":sum(x<0 for x in returns),"quality_passes":sum(r["quality"]["pass"] for r in rows),
            "first20_return_pct":mean(returns[:20]),"second20_return_pct":mean(returns[20:]),
            "mean_drawdown_pct":mean(r["probe"]["max_drawdown"]*100 for r in rows),
            "mean_market_volume_yuan":mean(r["market_volume_yuan"] for r in rows),
            "mean_market_trades":mean(r["market_trades"] for r in rows),
            "mean_price_inventory_pnl_yuan":mean(r["price_inventory_pnl_yuan"] for r in rows),
            "mean_pnl_yuan":mean(r["pnl_yuan"] for r in rows),
            "mean_income_cost_yuan":{k:mean(r["account"]["stats"][k]/100 for r in rows) for k in ["dividends","combo_dividends","trade_fees","idle_fees","interest"]},
            "account_totals":{k:sum(r["account"]["stats"][k] for r in rows) for k in ["rejects","liquidations","skipped","bot_errors","decisions"]},
            "all_accounts_totals":{k:sum(r["all_stats"][k] for r in rows) for k in ["rejects","liquidations","skipped","bot_errors","decisions"]},
            "filtered_buy_actions":sum(r["probe"]["filtered_buys"] for r in rows),
            "quality_checks":{c["name"]:{"passes":sum(next(x["pass"] for x in r["quality"]["checks"] if x["name"]==c["name"]) for r in rows),
                               "median":stats.median(next(x["value"] for x in r["quality"]["checks"] if x["name"]==c["name"]) for r in rows)} for c in rows[0]["quality"]["checks"]}}


def raw_rows(path):
    with path.open(encoding="utf-8",newline="") as f:
        return list(csv.DictReader(f))


def inspect_world(job,seed,names):
    p=OUT/job/f"seed-{seed}"
    summary=read(p/"summary.json")
    probe=read(p/"probe.json")
    account=next(a for a in summary["accounts"] if a["owner"]=="challenger")
    missing=account["stats"]["trades"]-probe["observed_fills"]
    final=[]
    if missing:
        final=[line.split(",") for line in (p/"trades.csv").read_text().splitlines() if line.startswith("2999,")]
        final=[r for r in final if r[5]=="challenger"]
        assert len(final)==missing and all(r[4]=="sell" for r in final), (job,seed,"unexplained missing fills")
    signals=defaultdict(list)
    for r in raw_rows(p/"signals.csv"):
        v={k:float(x) for k,x in r.items()}
        signals[int(v["stock"])].append(v)
    times={s:[r["tick"] for r in rs] for s,rs in signals.items()}
    typical={s:stats.median(r["volume10"] for r in rs if r["tick"]>=30) for s,rs in signals.items()}
    events=defaultdict(list)
    for e in raw_rows(p/"news.csv"):
        for change in e["changes"].split():
            stock,d=change.split(":")
            events[names.index(stock)].append((int(e["first_reveal"]),int(d),int(e["public_tick"])))
    for es in events.values(): es.sort()
    event_times={s:[e[0] for e in es] for s,es in events.items()}
    def label(s,t):
        i=bisect.bisect_right(event_times.get(s,[]),t)-1
        if i<0 or t-events[s][i][0]>180: return "far_from_news"
        first,delta,public=events[s][i]
        direction="positive" if delta>0 else "negative" if delta<0 else "zero"
        return direction+("_private" if t<public else "_public")
    def forward(s,t,price,dividend):
        i=bisect.bisect_left(times[s],t+30)
        if i==len(times[s]) or price<=0: return None
        later=signals[s][i]
        if later["bid"]<=0: return None
        return ((later["bid"]*0.9995+later["cumulative_dividend"]-dividend)/(price*1.0005)-1)*100
    groups=defaultdict(list)
    events_observed=defaultdict(list)
    unavailable=0
    sample_counts={"green":0,"all":0}
    for s,rs in signals.items():
        for r in rs:
            if r["tick"]<30 or r["tick"]>2950: continue
            markout=forward(s,r["tick"],r["ask"],r["cumulative_dividend"])
            if markout is None:
                unavailable+=1
                continue
            activity="high" if r["volume10"]>=2*typical[s] else "ordinary"
            direction="buy" if r["flow10"]>=.3 else "sell" if r["flow10"]<=-.3 else "weak"
            groups[f"{activity}_{direction}"].append(markout)
            green=r["return30"]>=.01
            groups["green" if green else "not_green"].append(markout)
            sample_counts["green"]+=green
            sample_counts["all"]+=1
            events_observed[label(s,r["tick"])].append((r["volume10"],r["flow10"],r["return30"]*100,markout))
    fills=defaultdict(lambda:{"count":0,"qty":0,"notional":0,"markout_sum":0,"markout_notional":0})
    for f in raw_rows(p/"challenger-fills.csv"):
        if f["side"]!="Buy": continue
        t,s,price,q,div=int(f["tick"]),int(f["stock"]),int(f["price"]),int(f["qty"]),int(f["cumulative_dividend"])
        green=float(f["return30"])>=.01
        m=forward(s,t,price,div)
        for key in ["all","green" if green else "not_green",label(s,t)]:
            a=fills[key];a["count"]+=1;a["qty"]+=q;a["notional"]+=q*price
            if m is not None: a["markout_sum"]+=m*q*price;a["markout_notional"]+=q*price
    return {"seed":seed,"typical_volume10":mean(typical.values()),"unavailable_forward_quotes":unavailable,
            "fill_delivery":{"recorded":probe["observed_fills"],"total":account["stats"]["trades"],"unobserved_final_tick_sells":missing,"unobserved_buys":0},
            "samples":sample_counts,"groups":{k:{"count":len(v),"mean_quote_return_pct":mean(v)} for k,v in groups.items()},
            "events":{k:{"count":len(v),"mean_volume10":mean(x[0] for x in v),"mean_flow10":mean(x[1] for x in v),
                         "mean_past_return30_pct":mean(x[2] for x in v),"mean_quote_return_pct":mean(x[3] for x in v)} for k,v in events_observed.items()},
            "fills":dict(fills)}


def summarize_trace(rows):
    groups={}
    for key in sorted({k for r in rows for k in r["groups"]}):
        xs=[r["groups"][key]["mean_quote_return_pct"] for r in rows if key in r["groups"]]
        groups[key]={"worlds":len(xs),"observations":sum(r["groups"].get(key,{}).get("count",0) for r in rows),
                     "mean_quote_return_pct":mean(xs),"ci95":interval(xs)}
    fills={}
    for key in sorted({k for r in rows for k in r["fills"]}):
        shares=[r["fills"].get(key,{}).get("notional",0)/r["fills"]["all"]["notional"] for r in rows if r["fills"].get("all",{}).get("notional",0)]
        returns=[r["fills"][key]["markout_sum"]/r["fills"][key]["markout_notional"] for r in rows if r["fills"].get(key,{}).get("markout_notional",0)]
        fills[key]={"worlds_with_forward_quotes":len(returns),"mean_notional_share":mean(shares),"mean_quote_return_pct":mean(returns),"ci95":interval(returns) if returns else None,
                    "fill_count":sum(r["fills"].get(key,{}).get("count",0) for r in rows)}
    events={}
    for key in sorted({k for r in rows for k in r["events"]}):
        rs=[r["events"][key] for r in rows if key in r["events"]]
        events[key]={"worlds":len(rs),**{k:mean(x[k] for x in rs) for k in ["mean_volume10","mean_flow10","mean_past_return30_pct","mean_quote_return_pct"]}}
    return {"groups":groups,"fills":fills,"events":events,"worlds":rows,
            "fill_delivery":{k:sum(r["fill_delivery"][k] for r in rows) for k in rows[0]["fill_delivery"]},
            "mean_typical_volume10":mean(r["typical_volume10"] for r in rows),
            "mean_green_observation_share":mean(r["samples"]["green"]/r["samples"]["all"] for r in rows)}


def main():
    if os.name=="nt": windows_build_budget()
    plan=read(OUT/"plan.json")
    batches={}; worlds=[]
    for job in plan["jobs"]:
        dest=OUT/job["id"]
        assert (dest/"done.json").exists(),job["id"]
        rows=[]
        for seed in range(job["first_seed"],job["first_seed"]+job["seeds"]):
            d=dest/f"seed-{seed}"; w=read(d/"summary.json"); probe=read(d/"probe.json")
            assert w["ticks"]==3000 and w["threads"]==4 and w["audits"]==6
            assert probe["calls"]==3000 and probe["unmatched_fills"]==0
            assert all(a["stats"]["decisions"]==3000 and a["stats"]["skipped"]==a["stats"]["bot_errors"]==0 and not a["disabled"] for a in w["accounts"])
            a=next(a for a in w["accounts"] if a["owner"]=="challenger")
            a={k:v for k,v in a.items() if k!="notional_by_counterparty"}
            pnl=(a["equity"]-a["initial_equity"])/100
            st=a["stats"]
            residual=pnl-(st["dividends"]+st["combo_dividends"]-st["trade_fees"]-st["idle_fees"]-st["interest"])/100
            r={"job":job["id"],"seed":seed,"account":a,"return_pct":(a["return_ratio"]-1)*100,
               "pnl_yuan":pnl,"price_inventory_pnl_yuan":residual,"probe":probe,"quality":w["quality"],
               "wall_seconds":w["wall_seconds"],"account_count":len(w["accounts"]),
               "market_volume_yuan":sum(x["stats"]["volume_notional"] for x in w["accounts"])/200,
               "market_trades":w["trades"],"audits":w["audits"],
               "all_stats":{k:sum(x["stats"][k] for x in w["accounts"]) for k in ["rejects","liquidations","skipped","bot_errors","decisions"]}}
            rows.append(r);worlds.append(r)
        batches[job["id"]]={"job":job,**summarize(rows),"engine_wall_seconds":sum(r["wall_seconds"] for r in rows),
                            "command_wall_seconds":read(dest/"done.json")["wall_seconds"]}
    pairs=[]
    def compare(a,b):
        ra={w["seed"]:w["return_pct"] for w in worlds if w["job"]==a}
        rb={w["seed"]:w["return_pct"] for w in worlds if w["job"]==b}
        assert ra.keys()==rb.keys()
        diff=[ra[s]-rb[s] for s in sorted(ra)]
        pairs.append({"a":a,"b":b,"mean_difference_pp":mean(diff),"ci95":interval(diff),"a_wins":sum(d>0 for d in diff),
                      "first20_difference_pp":mean(diff[:20]),"second20_difference_pp":mean(diff[20:])})
    for market in ["v5","v6"]:
        compare(f"{market}-news_fixed-plain",f"{market}-news-plain")
        compare(f"{market}-news_v2-plain",f"{market}-swift-plain")
        compare(f"{market}-swift-random",f"{market}-swift-flow")
        compare(f"{market}-swift-exclude_green",f"{market}-swift-plain")
    for mode in ["reverse","only_green","green"]:
        compare(f"v5-swift-{mode}","v5-swift-flow" if mode=="reverse" else "v5-swift-plain")
    compare("no_news-swift-plain","v5-swift-plain")
    compare("active_no_news-swift-plain","no_news-swift-plain")
    for market in ["no_news","active_no_news"]:
        compare(f"{market}-swift-random",f"{market}-swift-flow")
    names=[s["name"] for s in tomllib.loads((OUT/"v5.toml").read_text())["stocks"]]
    traces={}
    for job in plan["jobs"]:
        if not job["trace"]: continue
        print("ANALYZE",job["id"],flush=True)
        traces[job["id"]]=summarize_trace([inspect_world(job["id"],s,names) for s in range(job["first_seed"],job["first_seed"]+job["seeds"])])
    result={"plan":plan,"world_runs":len(worlds),"audits":sum(w["audits"] for w in worlds),"batches":batches,"pairs":pairs,
            "traces":traces,"worlds":worlds,"reporter_sha256":hashlib.sha256(Path(__file__).read_bytes()).hexdigest()}
    write(ROOT/"docs/news-swift-results.json",result)
    for name,b in batches.items(): print(name,"rank",round(b["mean_percentile"],2),"return",round(b["mean_return_pct"],2),"median",round(b["median_return_pct"],2),"losses",b["losses"],"quality",b["quality_passes"])
    for p in pairs: print("PAIR",p["a"],"minus",p["b"],round(p["mean_difference_pp"],2),[round(x,2) for x in p["ci95"]])


if __name__=="__main__": main()
