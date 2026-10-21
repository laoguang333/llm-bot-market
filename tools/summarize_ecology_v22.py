"""Compact paired-world evidence for the frozen v22 research batches."""
import json
from pathlib import Path
import statistics as st

from validate_v18 import sha,save

ROOT=Path(__file__).resolve().parents[1]


def get(data,path):
    for part in path.split('.'):
        data=data[part]
    return data


def paired(rows,arm,control,path):
    out=[]
    for seed in sorted({r['seed'] for r in rows}):
        aa=[get(r['metrics'],path) for r in rows if r['seed']==seed and r['arm']==arm]
        bb=[get(r['metrics'],path) for r in rows if r['seed']==seed and r['arm']==control]
        if aa and bb and None not in aa+bb:
            out.append(dict(seed=seed,change=st.mean(aa)-st.mean(bb)))
    return dict(worlds=len(out),mean=st.mean(x['change'] for x in out) if out else None,
                positive=sum(x['change']>0 for x in out),negative=sum(x['change']<0 for x in out),rows=out)


def main():
    stages={}
    paths=('market_return','health.normal.both','health.normal.active','health.normal.depth10',
           'health.distressed.ask','burned_fraction','discovery.hit','discovery.excess',
           'groups.coral.net_return','groups.coral.interest','groups.noise.net_return','groups.noise.ruined')
    for stage in ('fees','foundation','holdout','competition','repair-dev','repair-holdout','repair-competition'):
        source=ROOT/f'docs/ecology-v22-{stage}-results.json'
        if not source.exists():continue
        data=json.loads(source.read_text(encoding='utf-8'));agg=data['aggregate'];rows=data['rows']
        control='base' if stage.endswith('competition') else 'control' if stage.startswith('repair') else 'current'
        for row in rows:
            assert sha(Path(row['folder'])/'summary.json')==row['summary_sha256']
        stages[stage]=dict(source_sha256=sha(source),selected=data['selected'],
            runs=len(rows),audits=sum(r['metrics']['audits'] for r in rows),
            faults=sum(r['metrics'][k] for r in rows for k in ('errors','skipped','disabled')),
            simulation_seconds=sum(r['metrics']['simulation_seconds'] for r in rows),
            cli_seconds=sum(r['metrics']['cli_seconds'] for r in rows),
            paired={arm:{path:paired(rows,arm,control,path) for path in paths} for arm in agg if arm!=control},
            failed={arm:[k for k,passed in a.get('checks',{}).items() if not passed] for arm,a in agg.items()})
        if stage=='repair-dev':
            # Two-factor average effects and interaction, always excluding legacy fees.
            factors={}
            for path in paths:
                by_world=[]
                for seed in sorted({r['seed'] for r in rows}):
                    vals={a:st.mean(get(r['metrics'],path) for r in rows if r['seed']==seed and r['arm']==a)
                          for a in ('control','noise','coral','both')}
                    by_world.append(dict(seed=seed,
                        noise=((vals['noise']-vals['control'])+(vals['both']-vals['coral']))/2,
                        coral=((vals['coral']-vals['control'])+(vals['both']-vals['noise']))/2,
                        interaction=vals['both']-vals['noise']-vals['coral']+vals['control']))
                factors[path]={k:st.mean(v[k] for v in by_world) for k in ('noise','coral','interaction')}
                factors[path]['worlds']=by_world
            stages[stage]['factorial']=factors
        print(stage,stages[stage]['runs'],'runs',stages[stage]['audits'],'audits',stages[stage]['failed'])
    save(ROOT/'docs/ecology-v22-evidence.json',dict(script_sha256=sha(Path(__file__)),stages=stages,
        totals={k:sum(v[k] for v in stages.values()) for k in ('runs','audits','faults','simulation_seconds','cli_seconds')}))


if __name__=='__main__':main()
