"""Read executed trades, not inferred intentions, for all full/price-off worlds."""
import os,statistics as st
from pathlib import Path
from build_release import windows_build_budget
from research_v7 import read,write
from research_profit_effect import ROOT,OUT
if os.name=='nt':windows_build_budget()
rows=read(OUT/'batch.json');result={}
for arm in ['full','price_off']:
    worlds=[]
    for r in rows:
        if r['arm']!=arm:continue
        members={a['account']:a for a in r['summary']['accounts'] if a['account'].startswith('effect#')}
        bins=[0]*10;last={name:-1 for name in members};active=[set() for _ in range(10)]
        for line in (Path(r['directory'])/'trades.csv').open(encoding='utf-8'):
            if 'effect#' not in line:continue
            tick,stock,price,qty,side,taker,maker=line.strip().split(',');tick=int(tick);slot=min(9,tick//1000)
            for name in [taker,maker]:
                if name in members:
                    bins[slot]+=1;last[name]=max(last[name],tick);active[slot].add(name)
        worlds.append(dict(seed=r['seed'],members=len(members),
            fills_per_account_by_1000=[x/len(members) for x in bins],
            active_fraction_by_1000=[len(x)/len(members) for x in active],
            final_flat_fraction=sum(not any(a['shares']) for a in members.values())/len(members),
            no_trade_last_5000_fraction=sum(t<5000 for t in last.values())/len(members)))
    result[arm]=dict(worlds=worlds,mean_fills_by_1000=[st.mean(w['fills_per_account_by_1000'][i] for w in worlds) for i in range(10)],
        mean_active_by_1000=[st.mean(w['active_fraction_by_1000'][i] for w in worlds) for i in range(10)],
        final_flat_fraction=st.mean(w['final_flat_fraction'] for w in worlds),
        no_trade_last_5000_fraction=st.mean(w['no_trade_last_5000_fraction'] for w in worlds))
    print(arm,{k:v for k,v in result[arm].items() if k!='worlds'},flush=True)
write(ROOT/'docs/profit-effect-activity.json',result)
