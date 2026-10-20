"""Describe actual selected-stock price opportunities in existing Osprey traces.

Post-run diagnostics only. No labels or future prices are supplied to bots.
"""
import csv
import json
import os
from pathlib import Path
import statistics as st
import tomllib

from build_release import windows_build_budget

ROOT = Path(__file__).resolve().parents[1]


def main():
    if os.name == 'nt':
        windows_build_budget()
    cfg=tomllib.loads((ROOT/'scenarios/baseline-v14.toml').read_text(encoding='utf-8'))
    rows=[]
    for seed in range(62001,62041):
        for arm in ['osprey','control']:
            p=ROOT/'runs/osprey-holdout'/f'{arm}-{seed}'
            summary=json.loads((p/'summary.json').read_text(encoding='utf-8'))
            dragon=summary['dragon']['selected_stock']
            if dragon is None:
                continue
            name=cfg['stocks'][dragon]['name']
            with (p/'bars.csv').open(encoding='utf-8',newline='') as f:
                bars=[r for r in csv.DictReader(f) if r['stock']==name
                      and all(r[k] for k in ['open','high','low','close'])]
            opening=int(bars[0]['open'])
            peak=max(int(r['high']) for r in bars)
            cutoff=summary['dragon']['divergence_tick']
            late=[r for r in bars if int(r['tick'])>=cutoff]
            low=None
            excursion=1.0
            for r in late:
                if low is not None and low>0:
                    excursion=max(excursion,int(r['high'])/low)
                if int(r['low'])>0:
                    low=min(low or int(r['low']),int(r['low']))
            peak_so_far=int(late[0]['open'])
            drawdown=0.0
            for r in late:
                # Only earlier-bar highs, so no assumed high/low order within a bar.
                drawdown=max(drawdown,1-int(r['low'])/peak_so_far)
                peak_so_far=max(peak_so_far,int(r['high']))
            diag=summary['world_diagnostics']['stocks'][dragon]
            rows.append(dict(seed=seed,arm=arm,stock=name,
                peak_dividend_over_initial=diag['dividend_peak_initial_multiple'],
                peak_price_over_first_bar_open=peak/opening,
                final_close_over_first_bar_open=int(bars[-1]['close'])/opening,
                late_peak_over_late_first_open=max(int(r['high']) for r in late)/int(late[0]['open']),
                late_previous_trough_to_future_peak=excursion,
                late_price_drawdown=drawdown))
    result=dict(source='runs/osprey-holdout; selected stock labels used offline only',
        definition='Price multiple uses first recorded bar open, not initial theoretical value. Late starts at public divergence boundary. Trough/peak and drawdown use strictly earlier bars; retrospective opportunity, not achievable bot return.',
        rows=rows,summary={})
    for arm in ['osprey','control']:
        chosen=[r for r in rows if r['arm']==arm]
        result['summary'][arm]={'worlds':len(chosen)}
        for key in [k for k in chosen[0] if k not in ['seed','arm','stock']]:
            values=[r[key] for r in chosen]
            result['summary'][arm][key]=dict(min=min(values),median=st.median(values),max=max(values),
                at_least_10=sum(v>=10 for v in values))
    (ROOT/'docs/dragon-opportunities.json').write_text(json.dumps(result,ensure_ascii=False,indent=2),encoding='utf-8')
    print(json.dumps(result['summary'],ensure_ascii=False,indent=2))


if __name__=='__main__':
    main()
