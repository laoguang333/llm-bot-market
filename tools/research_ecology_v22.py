"""Frozen, serial v22 fee/foundation/competition research; see the protocol."""
import argparse
from copy import deepcopy
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import time
import tomllib

from validate_v18 import sha,save,opening
from market_health import analyze,aggregate,foundation_checks

ROOT = Path(__file__).resolve().parents[1]
FEES = {'current':(.006,.01),'light':(.001,.01),'flat':(.001,0.),'none':(0.,0.)}
CORE = {'grid_v2','value','coral','latecomer','news_reactor','noise','profit_effect'}
ROLE = {'news_v2':'value','hunter':'coral','shark':'coral','tracker':'value',
        'swift':'value','titan':'value','horizon':'coral','tide':'value','kestrel':'coral',
        'lattice':'value','almanac':'value','osprey':'coral'}


def toml_value(value):
    if isinstance(value,dict):
        return '{ '+', '.join(f'{k} = {toml_value(v)}' for k,v in value.items())+' }'
    return json.dumps(value,ensure_ascii=False)


def fee_text(text, arm):
    rate,credit=FEES[arm]
    text=re.sub(r'^idle_rate = .*$',f'idle_rate = {rate}',text,flags=re.M)
    return re.sub(r'^credit_ratio = .*$',f'credit_ratio = {credit}',text,flags=re.M)


def foundation_groups(groups):
    result=deepcopy(groups)
    for group in result:
        old=group['bot']
        if old in CORE: continue
        group['bot']=ROLE[old]
        if 'owner' in group: group['owner']='slot-'+group['owner']
        group.pop('forecast_access',None)
        group.pop('params',None)
        group['news_multiplier']=1.0
        if group['bot']=='coral': group['params']={'jitter':1.0}
    return result


def assemble(prefix, groups):
    return prefix+'\n'+''.join('[[bots]]\n'+'\n'.join(f'{k} = {toml_value(v)}' for k,v in b.items())+'\n\n' for b in groups)


def prepare():
    control=(ROOT/'scenarios/ecology-v21-control.toml').read_text(encoding='utf-8')
    source=(ROOT/'scenarios/dividend-v20-balanced.toml').read_text(encoding='utf-8')
    control=control[control.index('ticks ='):]
    source=source[source.index('ticks ='):]
    moderate=re.search(r'\[world.dividend_risk\].*?(?=\n\[world.dragon\])',control,re.S)[0]
    source=re.sub(r'\[world.dividend_risk\].*?(?=\n\[world.dragon\])',moderate,source,flags=re.S)
    prefix=source.split('[[bots]]')[0]
    cfg=tomllib.loads(source)
    groups=foundation_groups(cfg['bots'])
    original={b['owner']:b for b in cfg['bots'] if 'owner' in b}
    for arm in FEES:
        (ROOT/f'scenarios/ecology-v22-fees-{arm}.toml').write_text(
            '# v22 frozen fee ablation, original roster.\n'+fee_text(control,arm),encoding='utf-8')
        (ROOT/f'scenarios/ecology-v22-base-{arm}.toml').write_text(
            '# v22 experimental foundation, not the default baseline.\n'+fee_text(assemble(prefix,groups),arm),encoding='utf-8')
        for challenge,owners in {'swift':['swift-standard'],'tide':['tide-incumbent'],
                'almanac':['almanac-incumbent'],'all':['swift-standard','tide-incumbent','almanac-incumbent']}.items():
            changed=deepcopy(groups)
            for group in changed:
                owner=group.get('owner','').removeprefix('slot-')
                if owner not in owners: continue
                group['bot']=original[owner]['bot']
                group.pop('params',None)
                if 'params' in original[owner]: group['params']=original[owner]['params']
            (ROOT/f'scenarios/ecology-v22-{challenge}-{arm}.toml').write_text(
                '# v22 ordinary-information equal-capital slot replacement.\n'+fee_text(assemble(prefix,changed),arm),encoding='utf-8')
    print('Prepared 24 scenarios; default untouched.',flush=True)


def frozen():
    files=[*sorted((ROOT/'crates').rglob('*.rs')),*sorted((ROOT/'scenarios').glob('ecology-v22-*.toml')),
           ROOT/'tools/market_health.py',ROOT/'tools/research_ecology_v22.py',ROOT/'tools/test_market_health.py',
           ROOT/'tools/validate_v18.py',ROOT/'docs/ecology-v22-protocol.md',ROOT/'docs/ecology-v22-review.md',
           ROOT/'target/release/llm-market.exe',ROOT/'scenarios/baseline.toml']
    return {str(p.relative_to(ROOT)):sha(p) for p in files}


def compare(before,after,news_before,news_after):
    assert opening(before)==opening(after), 'Initial assets changed'
    assert len(before['accounts'])==len(after['accounts'])
    for a,b in zip(before['accounts'],after['accounts']):
        for k in ('account','owner','initial_equity','initial_cash','initial_shares','news_multiplier',
                  'forecast_access','news_received_by_stage','forecasts_received'):
            assert a[k]==b[k],(k,a['account'])
    for k in ('final_dividends','terminal_payments'):
        assert before[k]==after[k],k
    assert news_before==news_after,'World news changed'


def stage_plan(stage):
    if stage=='fees':
        return {a:ROOT/f'scenarios/ecology-v22-fees-{a}.toml' for a in FEES},91001,6
    if stage=='foundation':
        return {a:ROOT/f'scenarios/ecology-v22-base-{a}.toml' for a in FEES},91101,6
    development=json.loads((ROOT/'docs/ecology-v22-foundation-results.json').read_text())
    selected=development['selected'] or 'light'
    assert frozen()==development['frozen'],'Sources changed after development'
    if stage=='holdout':
        return {a:ROOT/f'scenarios/ecology-v22-base-{a}.toml' for a in dict.fromkeys(['current',selected])},91201,10
    validation=json.loads((ROOT/'docs/ecology-v22-holdout-results.json').read_text())
    if not development['selected'] or not validation['aggregate'][selected]['accepted']:
        save(ROOT/'docs/ecology-v22-competition-status.json',dict(ran=False,reason='foundation_did_not_pass',selected=development['selected']))
        return None,0,0
    return {a:ROOT/f"scenarios/ecology-v22-{('base' if a=='base' else a)}-{selected}.toml"
            for a in ('base','swift','tide','almanac','all')},91301,6


def run(stage):
    from build_release import windows_build_budget
    budget=windows_build_budget() if os.name=='nt' else os.cpu_count()
    arms,start,count=stage_plan(stage)
    if arms is None:
        print('Strong-bot reintroduction withheld: foundation failed independent qualification.')
        return
    out=ROOT/f'runs/ecology-v22-{stage}'
    out.mkdir(exist_ok=False)
    lock=frozen()
    manifest=dict(stage=stage,cpu_budget=budget,threads=4,first_seed=start,worlds=count,repeats=2,frozen=lock)
    save(out/'manifest.json',manifest)
    # Preserve executable, scripts, configs and source independently of later working-tree edits.
    snapshot=out/'frozen'; snapshot.mkdir()
    for relative in lock:
        dest=snapshot/relative;dest.parent.mkdir(parents=True,exist_ok=True)
        shutil.copy2(ROOT/relative,dest)
    binary=snapshot/'target/release/llm-market.exe'
    rows=[]
    names=list(arms)
    total=count*2*len(names)
    for i,seed in enumerate(range(start,start+count)):
        for repeat in range(2):
            offset=(i+repeat)%len(names)
            previous,previous_news=None,None
            for arm in names[offset:]+names[:offset]:
                folder=out/f'{arm}-{seed}-r{repeat}'
                cfg=arms[arm]
                command=[str(binary),'run',str(cfg),'--out',str(folder),'--seed',str(seed),'--threads','4']
                began=time.perf_counter()
                with folder.with_suffix('.log').open('w',encoding='utf-8') as handle:
                    subprocess.run(command,cwd=ROOT,stdout=handle,stderr=subprocess.STDOUT,check=True)
                cli_seconds=time.perf_counter()-began
                summary=json.loads((folder/'summary.json').read_text(encoding='utf-8'))
                news=sha(folder/'news.csv')
                if previous is not None: compare(previous,summary,previous_news,news)
                previous,previous_news=summary,news
                metrics=analyze(folder,cfg)
                metrics['cli_seconds']=cli_seconds
                row=dict(arm=arm,seed=seed,repeat=repeat,folder=str(folder),scenario=str(cfg.relative_to(ROOT)),
                         summary_sha256=sha(folder/'summary.json'),news_sha256=news,metrics=metrics)
                rows.append(row)
                save(out/'progress.json',rows)
                h=metrics['health']['normal']; d=metrics['discovery']
                print(f"{len(rows)}/{total} {stage} {arm} {seed} r{repeat}: net={metrics['market_return']:+.3f} "
                      f"both={h['both']:.3f} active={h['active']:.3f} events={d['used']} hit={d['hit']}",flush=True)
    result=aggregate(rows)
    selected=None
    if stage in ('foundation','holdout'):
        for arm,stats in result.items():
            stats['checks']=foundation_checks(stats,result['current'])
            stats['accepted']=all(stats['checks'].values())
        if stage=='foundation':
            selected=next((a for a in ('none','flat','light','current') if result[a]['accepted']),None)
    if stage=='competition':
        for arm,stats in result.items():
            stats['checks']=foundation_checks(stats,result['base'])
            stats['accepted']=all(stats['checks'].values())
    assert frozen()==lock,'Source or protocol changed while running'
    save(ROOT/f'docs/ecology-v22-{stage}-results.json',dict(frozen=lock,selected=selected,aggregate=result,rows=rows))
    print(json.dumps({a:{'accepted':v.get('accepted'),'failed':[k for k,p in v.get('checks',{}).items() if not p]}
                      for a,v in result.items()},ensure_ascii=False),flush=True)


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('stage',choices=['prepare','fees','foundation','holdout','competition'])
    args=parser.parse_args()
    (prepare() if args.stage=='prepare' else run(args.stage))


if __name__=='__main__': main()
