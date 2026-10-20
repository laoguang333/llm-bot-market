"""Fixed first account/stock/world illustration, alongside all experimental worlds."""
import csv,json,os
from pathlib import Path
from build_release import windows_build_budget
if os.name=='nt':windows_build_budget()
import matplotlib
matplotlib.use('Agg')
import matplotlib.pyplot as plt

ROOT=Path(__file__).resolve().parents[1]
OUT=ROOT/'runs/profit-effect-study'
rows=list(csv.DictReader((OUT/'decisions-effect-0.csv').open(encoding='utf-8-sig')))
rows=[r for r in rows if r['stock']=='T1']
times=[int(r['tick']) for r in rows]
fig,axes=plt.subplots(3,1,figsize=(12,9),layout='constrained',sharex=True)
axes[0].plot(times,[float(r['last_price_yuan']) for r in rows],lw=1,color='#386b9d',label='Last traded price')
for kind,color,marker in [('buy','#cf654e','^'),('sell','#469c7f','v')]:
    fills=[r for r in rows if int(r[kind+'_filled_now'])>0]
    axes[0].scatter([int(r['tick']) for r in fills],[float(r[kind+'_fill_price_yuan']) for r in fills],s=12,marker=marker,color=color,label=kind+' (immediate fills)',alpha=.5)
axes[0].set_ylabel('CNY');axes[0].legend(fontsize=9)
for key,label,color in [('peer_profit_pct','Peers: stock profit / initial equity','#c88b2f'),('own_profit_pct','Self: stock profit / initial equity','#9657a2')]:
    axes[1].plot(times,[float(r[key]) for r in rows],lw=1,label=label,color=color)
axes[1].axhline(0,color='#888',lw=.5);axes[1].set_ylabel('%');axes[1].legend(fontsize=9)
axes[2].plot(times,[float(r['target_shares']) for r in rows],lw=1,color='#c88b2f',label='Desired shares')
axes[2].plot(times,[float(r['held_after']) for r in rows],lw=1,color='#386b9d',label='Actual shares')
axes[2].set(ylabel='Shares',xlabel='Personal tick');axes[2].legend(fontsize=9)
for ax in axes:ax.grid(alpha=.2)
fig.suptitle('Profit-effect group | seed 33001 | effect#0 | T1\nFixed first sample, not selected for profit; markers omit deferred resting fills',fontsize=12)
dest=ROOT/'docs/assets/profit-effect-trace.png';fig.savefig(dest,dpi=150);print(dest)
