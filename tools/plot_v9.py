"""Standalone figures from real v9 research output (no strategy inputs changed)."""
import csv,json,os
from pathlib import Path
from build_release import windows_build_budget
ROOT=Path(__file__).resolve().parents[1]
if os.name=='nt':windows_build_budget()
import matplotlib
matplotlib.use('Agg')
import matplotlib.pyplot as plt

def main():
    dest=ROOT/'docs/assets';dest.mkdir(exist_ok=True)
    rows=json.loads((ROOT/'runs/v9-research/world-shapes.json').read_text())
    selected=[r['world']['stocks'][r['world']['selected_dragon']]['dividend_peak_initial_multiple'] for r in rows if r['world']['selected_dragon'] is not None]
    fig,axs=plt.subplots(1,2,figsize=(11,4),layout='constrained')
    axs[0].hist([r['world']['mean_stock_news_interval'] for r in rows],bins=22,color='#4c8d9a');axs[0].axvline(100,color='#dc8265',ls='--');axs[0].set(xlabel='Ticks per affected-stock news event',ylabel='Worlds',title='400 generated worlds: news cadence')
    axs[1].hist(selected,bins=22,color='#4c8d9a');axs[1].axvline(10,color='#dc8265',ls='--');axs[1].set(xlabel='Peak dividend / initial dividend',ylabel='Designated dragons',title='Cash-flow growth, not guaranteed price growth')
    fig.savefig(dest/'v9-world-shapes.png',dpi=150);plt.close(fig)
    run=ROOT/'runs/v9-research/bench/v8-27006-2'
    summary=json.loads((run/'summary.json').read_text())
    a=next(a for a in summary['accounts'] if a['account']=='tracker#3')
    bars=[r for r in csv.DictReader((run/'bars.csv').open()) if r['stock']=='T1' and r['close']]
    fig,axs=plt.subplots(2,1,figsize=(11,7),layout='constrained')
    axs[0].plot([int(r['tick']) for r in bars],[int(r['close'])/100 for r in bars],lw=.8,color='#4c8d9a');axs[0].set(yscale='log',xlabel='Tick',ylabel='T1 price (yuan, log)',title='v8 seed 27006 repeat 2: executed T1 prices')
    ev=a['liquidation_events'];axs[1].plot([e['tick'] for e in ev],[e['gross_before']/100 for e in ev],'o-',label='Marked inventory before liquidation');axs[1].plot([e['tick'] for e in ev],[-e['cash_before']/100 for e in ev],'o-',label='Debt before liquidation');axs[1].bar([e['tick'] for e in ev],[e['sale_notional']/100 for e in ev],width=.4,label='Actual sale proceeds')
    axs[1].set(xlabel='Account tick',ylabel='Yuan',title='tracker#3: marked collateral and realizable proceeds');axs[1].legend();fig.savefig(dest/'v9-liquidation-case.png',dpi=150);plt.close(fig)
    batch=ROOT/'runs/v9-research/batch/h300-29001-0'
    if (batch/'summary.json').exists():
        s=json.loads((batch/'summary.json').read_text());stock=s['dragon']['selected_stock']
        if stock is not None:
            import tomllib
            cfg=tomllib.loads((ROOT/'scenarios/baseline-v9.toml').read_text(encoding='utf-8'));name=cfg['stocks'][stock]['name'];initial=cfg['stocks'][stock]['dividend']
            bars=[r for r in csv.DictReader((batch/'bars.csv').open()) if r['stock']==name]
            fig,axs=plt.subplots(2,1,figsize=(11,6),sharex=True,layout='constrained')
            axs[0].plot([int(r['tick']) for r in bars],[int(r['dividend'])/initial for r in bars],color='#4c8d9a');axs[0].axhline(10,color='#dc8265',ls='--');axs[0].set(ylabel='Dividend / initial',title=f'v9 {name}, seed 29001: cash-flow path and actual price')
            traded=[r for r in bars if r['close']];axs[1].plot([int(r['tick']) for r in traded],[int(r['close'])/100 for r in traded],color='#dc8265');axs[1].set(xlabel='Tick',ylabel='Executed price (yuan)')
            for ax in axs:ax.axvline(5500,color='#777',ls=':');ax.grid(alpha=.15)
            fig.savefig(dest/'v9-growth-case.png',dpi=150);plt.close(fig)
if __name__=='__main__':main()
