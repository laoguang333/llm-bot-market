"""Two-factor, predeclared risk-constraint test after v22 foundation diagnosis."""
import argparse
from copy import deepcopy
import json
import os
from pathlib import Path
import shutil
import subprocess
import time
import tomllib

from validate_v18 import sha,save
from market_health import analyze,aggregate,foundation_checks
from research_ecology_v22 import compare,toml_value,validate_generated

ROOT=Path(__file__).resolve().parents[1]
ARMS=('control','noise','coral','both')
PROTOCOL=ROOT/'docs/ecology-v22-repair-protocol.md'


def prepare():
    source=(ROOT/'scenarios/ecology-v22-base-none.toml').read_text(encoding='utf-8')
    cfg=tomllib.loads(source)
    prefix=source.split('[[bots]]')[0]
    original=tomllib.loads((ROOT/'scenarios/ecology-v21-control.toml').read_text(encoding='utf-8'))
    originals={b['owner']:b for b in original['bots'] if 'owner' in b}
    for arm in ARMS:
        base=deepcopy(cfg['bots'])
        for b in base:
            if b['bot']=='noise' and arm in ('noise','both'): b.setdefault('params',{})['max_qty']=4.0
            if b['bot']=='coral' and arm in ('coral','both'): b.setdefault('params',{})['leverage']=1.0
        for kind,owners in {'base':[],'swift':['swift-standard'],'tide':['tide-incumbent'],
                            'almanac':['almanac-incumbent'],'all':['swift-standard','tide-incumbent','almanac-incumbent']}.items():
            groups=deepcopy(base)
            for b in groups:
                owner=b.get('owner','').removeprefix('slot-')
                if owner not in owners: continue
                b['bot']=originals[owner]['bot']
                b.pop('params',None)
                if 'params' in originals[owner]: b['params']=originals[owner]['params']
            text=prefix+'\n'+''.join('[[bots]]\n'+'\n'.join(f'{k} = {toml_value(v)}' for k,v in b.items())+'\n\n' for b in groups)
            text=text.rstrip()+'\n'
            validate_generated(text,cfg,groups,'none')
            (ROOT/f'scenarios/foundation-repair-v22-{kind}-{arm}.toml').write_text(text,encoding='utf-8')
    print('Prepared 20 equal-capital repair and conditional competition scenarios.')


def frozen():
    paths=[*sorted((ROOT/'crates').rglob('*.rs')),*sorted((ROOT/'scenarios').glob('foundation-repair-v22-*.toml')),
           PROTOCOL,ROOT/'docs/ecology-v22-protocol.md',ROOT/'tools/research_foundation_repair.py',
           ROOT/'tools/research_ecology_v22.py',ROOT/'tools/market_health.py',ROOT/'tools/test_market_health.py',
           ROOT/'tools/validate_v18.py',ROOT/'target/release/llm-market.exe',ROOT/'scenarios/baseline.toml']
    return {str(p.relative_to(ROOT)):sha(p) for p in paths}


def plan(stage):
    if stage=='dev': return {a:ROOT/f'scenarios/foundation-repair-v22-base-{a}.toml' for a in ARMS},91401,6
    dev=json.loads((ROOT/'docs/ecology-v22-repair-dev-results.json').read_text())
    assert frozen()==dev['frozen'],'No tuning between development and independent evaluation'
    selected=dev['selected'] or 'both'
    if stage=='holdout':
        return {a:ROOT/f'scenarios/foundation-repair-v22-base-{a}.toml' for a in dict.fromkeys(['control',selected])},91501,10
    holdout=json.loads((ROOT/'docs/ecology-v22-repair-holdout-results.json').read_text())
    if not dev['selected'] or not holdout['aggregate'][selected]['accepted']:
        save(ROOT/'docs/ecology-v22-repair-competition-status.json',dict(ran=False,reason='foundation_not_qualified',selected=dev['selected']))
        return None,0,0
    return {a:ROOT/f'scenarios/foundation-repair-v22-{a}-{selected}.toml' for a in ('base','swift','tide','almanac','all')},91601,6


def run(stage):
    from build_release import windows_build_budget
    budget=windows_build_budget() if os.name=='nt' else os.cpu_count()
    configs,start,n=plan(stage)
    if configs is None:
        print('Competition not run: repaired foundation failed qualification.');return
    lock=frozen()
    out=ROOT/f'runs/ecology-v22-repair-{stage}'
    out.mkdir(exist_ok=False)
    save(out/'manifest.json',dict(stage=stage,cpu_budget=budget,threads=4,first_seed=start,worlds=n,repeats=2,frozen=lock))
    snapshot=out/'frozen'
    for relative in lock:
        dest=snapshot/relative;dest.parent.mkdir(parents=True,exist_ok=True)
        shutil.copy2(ROOT/relative,dest)
    binary=snapshot/'target/release/llm-market.exe'
    rows=[];arms=list(configs)
    for i,seed in enumerate(range(start,start+n)):
        for repeat in range(2):
            offset=(i+repeat)%len(arms)
            previous,previous_news=None,None
            for arm in arms[offset:]+arms[:offset]:
                folder=out/f'{arm}-{seed}-r{repeat}'
                began=time.perf_counter()
                with folder.with_suffix('.log').open('w',encoding='utf-8') as handle:
                    subprocess.run([str(binary),'run',str(configs[arm]),'--out',str(folder),'--seed',str(seed),'--threads','4'],
                        cwd=ROOT,stdout=handle,stderr=subprocess.STDOUT,check=True)
                cli=time.perf_counter()-began
                summary=json.loads((folder/'summary.json').read_text(encoding='utf-8'));news=sha(folder/'news.csv')
                if previous is not None: compare(previous,summary,previous_news,news)
                previous,previous_news=summary,news
                m=analyze(folder,configs[arm]);m['cli_seconds']=cli
                rows.append(dict(arm=arm,seed=seed,repeat=repeat,folder=str(folder),scenario=str(configs[arm].relative_to(ROOT)),
                    summary_sha256=sha(folder/'summary.json'),news_sha256=news,metrics=m))
                save(out/'progress.json',rows)
                print(f"{len(rows)}/{n*2*len(arms)} repair-{stage} {arm} {seed} r{repeat} "
                      f"Coral={m['groups']['coral']['net_return']:+.3f} Noise ruin={m['groups']['noise']['ruined']:.3f} "
                      f"both={m['health']['normal']['both']:.3f}",flush=True)
    stats=aggregate(rows);base=stats['base' if stage=='competition' else 'control']
    for a,v in stats.items():
        v['checks']=foundation_checks(v,base);v['accepted']=all(v['checks'].values())
    selected=next((a for a in ARMS if stats[a]['accepted']),None) if stage=='dev' else None
    assert frozen()==lock,'Frozen files changed during simulation'
    save(ROOT/f'docs/ecology-v22-repair-{stage}-results.json',dict(frozen=lock,selected=selected,aggregate=stats,rows=rows))
    print(json.dumps({a:{'accepted':v['accepted'],'failed':[k for k,p in v['checks'].items() if not p]} for a,v in stats.items()}),flush=True)


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('stage',choices=['prepare','dev','holdout','competition']);args=parser.parse_args()
    prepare() if args.stage=='prepare' else run(args.stage)
