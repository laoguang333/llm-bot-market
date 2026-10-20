"""Compact progress from completed experiment worlds; no simulation runs."""
import json,statistics as st
from pathlib import Path
p=Path(__file__).resolve().parents[1]/'runs/v10-research'
for group in ['batch','ablation','bench']:
    f=p/f'{group}.json'
    if not f.exists():continue
    rows=json.loads(f.read_text());arms=sorted({r['arm'] for r in rows})
    seeds=set.intersection(*[{r['seed'] for r in rows if r['arm']==arm} for arm in arms])
    print(group,'completed matches',len(rows),'complete worlds',len(seeds))
    if group=='bench':continue
    for arm in arms:
        a=[next(a for a in r['summary']['accounts'] if a['account']=='researcher') for r in rows if r['arm']==arm and r['seed'] in seeds]
        print(arm,len(a),round(st.mean(x['return_ratio']-1 for x in a)*100,1),'%',round(st.mean(x['percentile'] for x in a),1),'pct',sum(x['percentile']>=75 for x in a),'topquartile')
