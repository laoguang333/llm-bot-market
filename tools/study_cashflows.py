"""Validate the v6 fee/combo migration with sequential release matches.

Requires before.exe, baseline.toml and before.json frozen in --out before editing.
Use arena_gate.py to serialize the entire run with other market experiments.
"""
import argparse
import copy
import hashlib
import json
import os
from pathlib import Path
import shutil
import statistics as stats
import subprocess
import time
import tomllib

from build_release import windows_build_budget
from study_v6_population import measure
from swift_stress import toml_text

ROOT=Path(__file__).resolve().parents[1]


def read(path):
    return json.loads(path.read_text(encoding='utf-8'))


def save(path,value):
    path.write_text(json.dumps(value,ensure_ascii=False,indent=2),encoding='utf-8')


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def run_one(binary,scenario,dest,seed,threads):
    marker=dest.with_suffix('.done.json')
    if marker.exists():
        data=read(marker)
        assert data['exit_code']==0,marker
        return data
    if dest.exists():raise SystemExit(f'Incomplete run preserved: {dest}')
    dest.parent.mkdir(exist_ok=True,parents=True)
    command=[str(binary),'run',str(scenario),'--out',str(dest),'--seed',str(seed),'--threads',str(threads)]
    start=time.perf_counter()
    with dest.with_suffix('.log').open('w',encoding='utf-8') as f:
        process=subprocess.run(command,cwd=ROOT,stdout=f,stderr=subprocess.STDOUT)
    row=dict(command=command,wall_seconds=time.perf_counter()-start,exit_code=process.returncode)
    save(marker,row)
    if process.returncode:raise SystemExit(dest.with_suffix('.log').read_text(encoding='utf-8'))
    return row


def prepare(out):
    if (out/'plan.json').exists():raise SystemExit('Plan already frozen; use run/report to resume')
    before=read(out/'before.json')
    assert sha(out/'before.exe')==before['binary_sha256']
    assert sha(out/'baseline.toml')==before['scenario_sha256']
    shutil.copy2(ROOT/'target/release/llm-market.exe',out/'after.exe')
    old=tomllib.loads((out/'baseline.toml').read_text(encoding='utf-8'))
    configs={key:copy.deepcopy(old) for key in ['before','fee','combo','after']}
    for key in ['fee','after']:configs[key]['fees']['idle_rate']=0.006
    for key in ['combo','after']:
        configs[key]['world'].update(combo_period=1,combo_scale=50)
    # This frozen migration predates the separate adoption of NewsV2 in v6.
    assert configs['after']==tomllib.loads((ROOT/'scenarios/baseline-v6-legacy-news.toml').read_text(encoding='utf-8'))
    for key,cfg in configs.items():(out/f'{key}.toml').write_text(toml_text(cfg),encoding='utf-8')
    plan=dict(before,conditions=list(configs),binaries={k:sha(out/f'{k}.exe') for k in ['before','after']},
        scenarios={k:sha(out/f'{k}.toml') for k in configs},
        bot_sources={str(p.relative_to(ROOT)):sha(p) for p in (ROOT/'crates/bots/src').glob('*.rs')},
        note='Fixed user-selected 0.6% fee and per-tick rounded combos; no strategy fitting. Single-thread compatibility, interleaved timing, then paired market effects.')
    save(out/'plan.json',plan)


def run(out):
    plan=read(out/'plan.json')
    for name,digest in plan['binaries'].items():assert sha(out/f'{name}.exe')==digest
    for name,digest in plan['scenarios'].items():assert sha(out/f'{name}.toml')==digest
    for name,digest in plan['bot_sources'].items():assert sha(ROOT/name)==digest,name
    # Identical old rules on one thread must preserve actual trades and accounting.
    for seed in plan['perf_seeds']:
        runs=[]
        for name in ['before','after']:
            dest=out/'compat'/f'{name}-{seed}'
            run_one(out/f'{name}.exe',out/'before.toml',dest,seed,1)
            runs.append(dest)
        assert sha(runs[0]/'trades.csv')==sha(runs[1]/'trades.csv'),seed
        a,b=[read(p/'summary.json') for p in runs]
        for aa,bb in zip(a['accounts'],b['accounts']):
            for key in ['cash','shares','terminal_value','equity','initial_equity','return_ratio']:
                assert aa[key]==bb[key],(seed,aa['owner'],key)
        print(f'Compatibility seed {seed}: trades and account settlement identical',flush=True)
    variants=[('old_engine_old_rules','before','before'),('new_engine_old_rules','after','before'),('new_engine_new_rules','after','after')]
    cases=[(True,0,plan['perf_seeds'][0])]+[(False,rep,seed) for rep in range(plan['perf_repeats']) for seed in plan['perf_seeds']]
    for number,(warmup,rep,seed) in enumerate(cases):
        for label,binary,scenario in variants[::1 if number%2==0 else -1]:
            dest=out/'perf'/f'{label}-{seed}-{rep}-warmup-{int(warmup)}'
            row=run_one(out/f'{binary}.exe',out/f'{scenario}.toml',dest,seed,plan['threads'])
            print(f'{dest.name}: {row["wall_seconds"]:.3f}s',flush=True)
    for seed in plan['impact_seeds']:
        keys=plan['conditions'];offset=seed%len(keys)
        for key in keys[offset:]+keys[:offset]:
            dest=out/key/f'seed-{seed}-repeat-0'
            row=run_one(out/'after.exe',out/f'{key}.toml',dest,seed,plan['threads'])
            print(f'{key} seed {seed}: {row["wall_seconds"]:.3f}s',flush=True)


def report(out):
    plan=read(out/'plan.json');results={};summaries={};fingerprints={}
    for key in plan['conditions']:
        worlds=[]
        for seed in plan['impact_seeds']:
            dest=out/key/f'seed-{seed}-repeat-0'
            assert read(dest.with_suffix('.done.json'))['exit_code']==0
            w=measure(dest);r=read(dest/'summary.json')
            if seed in fingerprints:assert w['world_fingerprint']==fingerprints[seed]
            fingerprints[seed]=w['world_fingerprint']
            w['swift']={a['owner']:{'return_pct':100*(a['return_ratio']-1),'idle_yuan':a['stats']['idle_fees']/100,
                         'combo_yuan':a['stats']['combo_dividends']/100,'dividend_yuan':a['stats']['dividends']/100}
                         for a in r['accounts'] if a['strategy']=='swift'}
            w['strategy_returns']={s:stats.mean(100*(a['return_ratio']-1) for a in r['accounts'] if a['strategy']==s) for s in r['by_strategy']}
            w['total_combo_yuan']=sum(a['stats']['combo_dividends'] for a in r['accounts'])/100
            w['total_idle_yuan']=sum(a['stats']['idle_fees'] for a in r['accounts'])/100
            assert w['audits']==6 and not (w['skipped'] or w['bot_errors'] or w['disabled'])
            worlds.append(w)
        results[key]=worlds
        summaries[key]=dict(worlds=len(worlds),quality_pass=sum(w['quality']['pass'] for w in worlds),
            median_spread=stats.median(w['median_spread'] for w in worlds),median_depth_yuan=stats.median(w['median_depth_yuan'] for w in worlds),
            mean_two_sided=stats.mean(w['two_sided'] for w in worlds),mean_total_combo_yuan=stats.mean(w['total_combo_yuan'] for w in worlds),
            mean_total_idle_yuan=stats.mean(w['total_idle_yuan'] for w in worlds),
            strategy_mean_return_pct={s:stats.mean(w['strategy_returns'][s] for w in worlds) for s in worlds[0]['strategy_returns']},
            swift={owner:{'mean_return_pct':stats.mean(w['swift'][owner]['return_pct'] for w in worlds),
                          'losses':sum(w['swift'][owner]['return_pct']<0 for w in worlds),
                          'mean_idle_yuan':stats.mean(w['swift'][owner]['idle_yuan'] for w in worlds),
                          'mean_combo_yuan':stats.mean(w['swift'][owner]['combo_yuan'] for w in worlds)} for owner in worlds[0]['swift']})
    performance={}
    for label in ['old_engine_old_rules','new_engine_old_rules','new_engine_new_rules']:
        rows=[]
        for dest in sorted((out/'perf').glob(f'{label}-*-warmup-0')):
            if not dest.is_dir():continue
            timing=read(dest.with_suffix('.done.json'));summary=read(dest/'summary.json')
            rows.append(dict(seed=summary['seed'],wall=timing['wall_seconds'],engine=summary['wall_seconds'],audits=summary['audits']))
        assert len(rows)==len(plan['perf_seeds'])*plan['perf_repeats']
        performance[label]=dict(runs=rows,median_wall=stats.median(r['wall'] for r in rows),median_engine=stats.median(r['engine'] for r in rows))
    result=dict(plan=plan,compatibility='All three seeds: one-thread trades and account settlement identical',
                performance=performance,summary=summaries,worlds=results,exogenous_worlds_matched=True)
    save(out/'results.json',result)
    print(json.dumps(dict(performance=performance,summary=summaries),ensure_ascii=False,indent=2))


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('command',choices=['prepare','run','report'])
    parser.add_argument('--out',type=Path,default=ROOT/'runs/v6-cashflows-20261019')
    args=parser.parse_args()
    if os.name=='nt':windows_build_budget()
    {'prepare':prepare,'run':run,'report':report}[args.command](args.out.resolve())
