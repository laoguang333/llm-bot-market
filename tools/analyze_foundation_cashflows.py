"""Realized fixed-inventory contract benchmark, only in zero-idle-fee worlds.

This is an ex-post account payoff calculation, not a tradable value forecast or
a reconstruction of how the other agents would trade if Grid stopped trading.
"""
import json
from pathlib import Path
import statistics as st
import tomllib

from market_health import rates_and_events,csv_rows
from validate_v18 import sha,save

ROOT=Path(__file__).resolve().parents[1]


def rounded(n,d):
    return (1 if n>=0 else -1)*((abs(n)+d//2)//d)


def passive(cfg,summary,account,rates):
    assert cfg['fees']['idle_rate']==0 and cfg['world']['dividend_period']==1 and cfg['world']['combo_period']==1
    names=[s['name'] for s in cfg['stocks']]
    holdings=dict(zip(names,account['initial_shares']))
    end=cfg['ticks']-1
    ds,cs=cfg['world']['dividend_scale'],cfg['world']['combo_scale']
    cash=minimum=account['initial_cash']
    ordinary,combo=0,0
    for tick in range(1,cfg['ticks']):
        d=sum(rounded(holdings[s]*rates[s][tick],ds) for s in names)
        c=sum(rounded(min(holdings[r['a']]//r.get('a_units',1),holdings[r['b']]//r.get('b_units',1))*r['bonus'],cs)
              for r in cfg['combos'] if r.get('start_tick',0)<=tick<=r.get('end_tick',end))
        ordinary+=d;combo+=c;cash+=d+c;minimum=min(minimum,cash)
    assert minimum>=0,'Passive contract requires financing; simple benchmark is not applicable'
    duration=summary['terminal_payments']*cfg['scoring']['terminal_payment_ticks']
    terminal=sum(rounded(holdings[s]*rates[s][-1]*duration,ds) for s in names)
    terminal+=sum(rounded(min(holdings[r['a']]//r.get('a_units',1),holdings[r['b']]//r.get('b_units',1))*r['bonus']*duration,cs)
                  for r in cfg['combos'] if r.get('start_tick',0)<=end<=r.get('end_tick',end))
    return dict(net_return=(cash+terminal)/account['initial_equity']-1,ordinary=ordinary,
                combo=combo,terminal=terminal,final_cash=cash,minimum_cash=minimum)


def main():
    cache={};results={}
    for stage in ('foundation','repair-dev','repair-holdout'):
        path=ROOT/f'docs/ecology-v22-{stage}-results.json'
        if not path.exists():continue
        experiment=json.loads(path.read_text(encoding='utf-8'));rows=[]
        for row in experiment['rows']:
            cfg=tomllib.loads((ROOT/row['scenario']).read_text(encoding='utf-8'))
            if cfg['fees']['idle_rate']!=0:continue
            summary=json.loads((Path(row['folder'])/'summary.json').read_text(encoding='utf-8'))
            assert sha(Path(row['folder'])/'summary.json')==row['summary_sha256']
            grid=[a for a in summary['accounts'] if a['strategy']=='grid_v2']
            a=grid[0]
            assert all((b['initial_cash'],b['initial_shares'],b['initial_equity'])==
                       (a['initial_cash'],a['initial_shares'],a['initial_equity']) for b in grid)
            key=(row['seed'],row['news_sha256'],a['initial_cash'],tuple(a['initial_shares']),a['initial_equity'])
            if key not in cache:
                rates,_,_=rates_and_events(cfg,csv_rows(Path(row['folder'])/'news.csv'))
                cache[key]=passive(cfg,summary,a,rates)
            actual=row['metrics']['groups']['grid_v2']['net_return']
            rows.append(dict(seed=row['seed'],repeat=row['repeat'],arm=row['arm'],actual=actual,
                             passive=cache[key]['net_return'],gap=actual-cache[key]['net_return'],
                             minimum_cash=cache[key]['minimum_cash']))
        results[stage]=dict(source_sha256=sha(path),rows=rows,
            aggregate={arm:{k:st.mean(r[k] for r in rows if r['arm']==arm) for k in ('actual','passive','gap')}
                       for arm in sorted({r['arm'] for r in rows})})
        print(stage,results[stage]['aggregate'])
    save(ROOT/'docs/ecology-v22-passive-analysis.json',dict(script_sha256=sha(Path(__file__)),stages=results,
        interpretation='Ex-post static initial-inventory contract payoff; all assessed passive accounts stay cash-positive; not a price-path replay or pure counterparty profit.'))


if __name__=='__main__':main()
