"""Post-game execution diagnostics. Identities and future quotes are never bot inputs.

For each target fill, compare its price with the first available two-sided mid
at or after tick+20, allowing at most 10 more ticks. This is a descriptive markout,
not a realized-trade P&L, and can include dividend-related price changes.
Clock-phase bins use trade timestamps, not the opponent's private local clock;
the simulator permits tick skew, so phase alignment is approximate.
"""
import bisect
import csv
import json
from pathlib import Path
import statistics as stats
from swift_hunt import ROOT, BASE


def world(row):
    path=BASE/row["stage"]/f"{row['case']}-{row['order']}"/f"seed-{row['seed']}"
    books={}
    with (path/"books.csv").open() as f:
        for b in csv.DictReader(f):
            if b["bid"] and b["ask"]:
                books.setdefault(b["stock"],[]).append((int(b["tick"]),(int(b["bid"])+int(b["ask"]))/2))
    times={s:[p[0] for p in xs] for s,xs in books.items()}
    total=marked=markout=buy=late_buy=sell=late_sell=0
    name=row["target"]["account"]
    with (path/"trades.csv").open() as f:
        for t in csv.DictReader(f):
            if t["taker"]!=name and t["maker"]!=name:
                continue
            is_buy=(t["aggressor"]=="buy")== (t["taker"]==name)
            tick,price,qty=int(t["tick"]),int(t["price"]),int(t["qty"])
            value=price*qty
            total+=value
            if is_buy:
                buy+=value
                if (tick//20)%2: late_buy+=value
            else:
                sell+=value
                if (tick//20)%2==0: late_sell+=value
            xs=books.get(t["stock"],[])
            idx=bisect.bisect_left(times.get(t["stock"],[]),tick+20)
            if idx<len(xs) and xs[idx][0]<=tick+30:
                marked+=value
                markout+=(xs[idx][1]-price)*qty*(1 if is_buy else -1)
    return {"case":row["case"],"seed":row["seed"],"order":row["order"],
            "target_trade_notional":total,"coverage":marked/max(1,total),
            "signed_mid_markout20_bps":markout/max(1,marked)*10000,
            "buy_notional_during_clock_sell_phase_share":late_buy/max(1,buy),
            "sell_notional_during_clock_buy_phase_share":late_sell/max(1,sell)}


def main():
    rpt=json.loads((ROOT/"docs/swift-hunt-results.json").read_text(encoding="utf-8"))
    rows=[world(r) for r in rpt["worlds"] if r["case"] in ["alone","relay","clock"]]
    combined=[]
    for case in ["alone","relay","clock"]:
        these=[r for r in rows if r["case"]==case]
        combined.append({"case":case,"runs":len(these),**{k:stats.mean(r[k] for r in these) for k in
                         ["coverage","signed_mid_markout20_bps","buy_notional_during_clock_sell_phase_share","sell_notional_during_clock_buy_phase_share"]}})
    out={"notes":__doc__,"combined":combined,"worlds":rows}
    (ROOT/"docs/swift-hunt-markout.json").write_text(json.dumps(out,ensure_ascii=False,indent=2)+"\n",encoding="utf-8")
    print(json.dumps(combined,ensure_ascii=False,indent=2))


if __name__=="__main__":
    main()
