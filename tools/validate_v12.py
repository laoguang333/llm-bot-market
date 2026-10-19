"""Validate Horizon's promotion using reviewed release sources; serial half-core runs."""
import hashlib
import json
import os
import statistics as st
import tomllib
from pathlib import Path
from build_release import windows_build_budget
from research_v7 import run_one, read, write, sha

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT/'runs/v12-validation'
FROZEN = ROOT/'runs/horizon-v3-research/frozen'


def main():
    budget = windows_build_budget() if os.name == 'nt' else os.cpu_count()
    assert not (OUT/'bench').exists(), 'Refusing overwrite'
    manifest = read(FROZEN/'manifest.json')
    for name, expected in manifest['source'].items():
        assert sha(ROOT/name) == expected, f'Reviewed source changed: {name}'
    binary = FROZEN/'llm-market.exe'
    assert sha(binary) == manifest['binaries']['llm-market.exe']
    configs = {v: ROOT/f'scenarios/baseline-{v}.toml' for v in ['v11', 'v12']}
    a, b = [tomllib.loads(p.read_text()) for p in configs.values()]
    assert a['bots'][:-1] == b['bots'][:-1]
    assert len(a['bots']) == len(b['bots'])
    target = b['bots'][-1]
    assert target == dict(bot='horizon', owner='horizon-incumbent', count=1, cash=20_000_000, shares_each=200, news_multiplier=1.0, forecast_access=dict(horizon=1000, coverage=.5, range_width=3.), params=dict(leverage=2., cap=.45, clip=.025, research=1., staged=1., terminal_guard=1.))
    a.pop('bots'); b.pop('bots'); assert a == b
    assert (ROOT/'scenarios/baseline.toml').read_bytes() == configs['v12'].read_bytes()
    write(OUT/'manifest.json', dict(source=manifest['source'], binary_sha256=sha(binary), configs={k:sha(v) for k,v in configs.items()}, cpu_budget=budget, threads=4, seeds=[7,27006,28012], repeats=3))
    warm=[]
    for arm in configs:
        summary, wall = run_one(binary, configs[arm], OUT/'warmup'/arm, 7, 4)
        warm.append(dict(arm=arm, seed=7, wall=wall, summary=summary))
    rows=[]
    for seed in [7,27006,28012]:
        for repeat in range(3):
            pair=[]
            for arm in (['v11','v12'] if repeat%2 == 0 else ['v12','v11']):
                dest=OUT/'bench'/f'{arm}-{seed}-{repeat}'
                summary,wall=run_one(binary, configs[arm], dest, seed, 4)
                row=dict(arm=arm, seed=seed, repeat=repeat, wall=wall, summary=summary)
                rows.append(row); pair.append(row)
                print(arm, seed, repeat, round(wall,3), 'seconds', flush=True)
            dirs=[OUT/'bench'/f'{arm}-{seed}-{repeat}' for arm in ['v11','v12']]
            assert (dirs[0]/'news.csv').read_bytes() == (dirs[1]/'news.csv').read_bytes()
            x,y=pair[0]['summary'],pair[1]['summary']
            assert x['final_dividends'] == y['final_dividends'] and x['terminal_payments'] == y['terminal_payments']
            assert [(v['account'],v['initial_cash'],v['initial_shares'],v['initial_equity']) for v in x['accounts']] == [(v['account'],v['initial_cash'],v['initial_shares'],v['initial_equity']) for v in y['accounts']]
            write(OUT/'bench.json', rows)
    all_rows=rows+warm
    result=dict(runs=len(all_rows), timed_runs=len(rows), independent_worlds=3, cpu_budget=budget, threads=4, repeats=3,
                audits=sum(r['summary']['audits'] for r in all_rows),
                skips=sum(a['stats']['skipped'] for r in all_rows for a in r['summary']['accounts']),
                errors=sum(a['stats']['bot_errors'] for r in all_rows for a in r['summary']['accounts']),
                disabled=sum(bool(a['disabled']) for r in all_rows for a in r['summary']['accounts']),
                external_world_pairs_verified=9, initial_capital_pairs_verified=9, strategies={})
    for arm in configs:
        group=[r for r in rows if r['arm']==arm]
        accounts=[next(a for a in r['summary']['accounts'] if a['account']=='horizon-incumbent') for r in group]
        result['strategies'][arm]=dict(wall_median=st.median(r['wall'] for r in group), wall_min=min(r['wall'] for r in group), wall_max=max(r['wall'] for r in group),
            engine_median=st.median(r['summary']['wall_seconds'] for r in group), mean_return=st.mean(a['return_ratio']-1 for a in accounts),
            mean_percentile=st.mean(a['percentile'] for a in accounts), quality_pass=sum(r['summary']['quality']['pass'] for r in group),
            mean_decision_us=st.mean(a['stats']['decision_nanos_total']/max(1,a['stats']['decisions'])/1000 for a in accounts))
    assert not any(result[k] for k in ['skips','errors','disabled'])
    write(ROOT/'docs/baseline-v12-results.json', result)
    print(json.dumps(result,ensure_ascii=False,indent=2))


if __name__=='__main__':
    main()
