"""Promote reviewed ProfitEffect to v13; serial release regression/performance.

Population ranges are preserved, individual seed populations are not identical
to v12. This is a promotion regression, not a capital-matched strategy study.
"""
from collections import defaultdict
import os
from pathlib import Path
import statistics as st
import subprocess
import tomllib

from build_release import windows_build_budget
from research_profit_effect import population
from research_v7 import read, write, sha, run_one

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / 'runs/v13-validation'
FROZEN = ROOT / 'runs/profit-effect-leaders/frozen'
SEEDS = [7, 27006, 28012]


def expected_accounts(cfg, seed):
    offsets = defaultdict(int)
    result = {}
    for i, g in enumerate(cfg['bots']):
        for j in range(population(seed, i, g)):
            name = g.get('owner', f"{g['bot']}#{offsets[g['bot']] + j}")
            assert name not in result
            result[name] = g
        if 'owner' not in g: offsets[g['bot']] += g.get('count_max', g.get('count', 1))
    return result


def check_config(configs):
    old, new = configs['v12'], configs['v13']
    assert {k: v for k, v in old.items() if k != 'bots'} == {k: v for k, v in new.items() if k != 'bots'}
    assert new['bots'][:-4][:4] == old['bots'][:4]
    assert new['bots'][5:-4] == old['bots'][5:]
    noise = dict(old['bots'][4], count=9, count_max=21)
    assert new['bots'][4] == noise and noise['bot'] == 'noise'
    crowd = new['bots'][-4:]
    assert [(g['count'], g['count_max'], g['params']['leader_weight']) for g in crowd] == [
        (6, 14, 0.0), (1, 3, 0.5), (1, 2, 0.8), (1, 2, 1.1)]
    assert all(g['bot'] == 'profit_effect' and g['cash'] == 2_000_000 and g['shares_each'] == 20
        and g['news_multiplier'] == 1.0 and not g.get('forecast_access') and g['social_group'] == 'profit-circle' for g in crowd)
    assert all(not g.get('social_group') for g in new['bots'][:-4])
    for cfg in configs.values():
        assert sum(g['count'] for g in cfg['bots']) == 100
        assert sum(g.get('count_max', g['count']) for g in cfg['bots']) == 224
    assert (ROOT/'scenarios/baseline.toml').read_bytes() == (ROOT/'scenarios/baseline-v13.toml').read_bytes()


def main():
    budget = windows_build_budget() if os.name == 'nt' else os.cpu_count()
    OUT.mkdir(parents=True, exist_ok=False)
    manifest = read(FROZEN/'manifest.json')
    for path, expected in manifest['source'].items():
        assert sha(ROOT/path) == expected, f'Reviewed source changed: {path}'
    binary = FROZEN/'llm-market.exe'
    assert sha(binary) == manifest['binaries']['llm-market.exe']
    paths = {v: ROOT/f'scenarios/baseline-{v}.toml' for v in ['v12','v13']}
    configs = {v: tomllib.loads(p.read_text(encoding='utf-8')) for v, p in paths.items()}
    check_config(configs)
    write(OUT/'manifest.json', dict(source=manifest['source'], binary_sha256=sha(binary),
        configs={v: sha(p) for v, p in paths.items()}, seeds=SEEDS, repeats=3, threads=4, cpu_budget=budget))
    subprocess.run(['cargo','test','--release','--workspace','--jobs',str(budget),'--','--test-threads=1'], cwd=ROOT, check=True)
    subprocess.run(['node','--check','tools/debugger/app.js'], cwd=ROOT, check=True)
    subprocess.run(['node','tools/test_research_links.js'], cwd=ROOT, check=True)
    rows=[]; warm=[]
    def run(arm, seed, destination):
        summary, wall = run_one(binary, paths[arm], destination, seed, 4)
        expected = expected_accounts(configs[arm], seed)
        assert set(expected) == {a['account'] for a in summary['accounts']}
        for a in summary['accounts']:
            g = expected[a['account']]
            assert a['strategy'] == g['bot'] and a['initial_cash'] == g['cash']
            assert a['initial_shares'] == [g['shares_each']] * len(configs[arm]['stocks'])
        return dict(arm=arm, seed=seed, wall=wall, summary=summary, directory=str(destination))
    for arm in paths: warm.append(run(arm,7,OUT/'warmup'/arm))
    for seed in SEEDS:
        for repeat in range(3):
            pair=[]
            for arm in (['v12','v13'] if repeat%2==0 else ['v13','v12']):
                row=run(arm,seed,OUT/'bench'/f'{arm}-{seed}-{repeat}')
                row['repeat']=repeat; rows.append(row); pair.append(row)
                print(arm, seed, repeat, round(row['wall'],3), 'seconds', flush=True)
            a,b=pair
            assert sha(Path(a['directory'])/'news.csv') == sha(Path(b['directory'])/'news.csv')
            assert a['summary']['terminal_payments'] == b['summary']['terminal_payments']
            assert a['summary']['final_dividends'] == b['summary']['final_dividends']
            write(OUT/'bench.json',rows)
    all_rows=rows+warm
    result=dict(runs=len(all_rows), timed_runs=len(rows), seeds=SEEDS, repeats=3, threads=4, cpu_budget=budget,
        audits=sum(r['summary']['audits'] for r in all_rows),
        skips=sum(a['stats']['skipped'] for r in all_rows for a in r['summary']['accounts']),
        errors=sum(a['stats']['bot_errors'] for r in all_rows for a in r['summary']['accounts']),
        disabled=sum(bool(a['disabled']) for r in all_rows for a in r['summary']['accounts']),
        external_world_pairs_verified=9, per_run_population_and_endowment_verified=True,
        equal_population_or_capital_between_versions=False, configurations={})
    for arm in paths:
        chosen=[r for r in rows if r['arm']==arm]
        result['configurations'][arm]=dict(wall_median=st.median(r['wall'] for r in chosen),
            wall_min=min(r['wall'] for r in chosen),wall_max=max(r['wall'] for r in chosen),
            quality_pass=sum(r['summary']['quality']['pass'] for r in chosen),
            populations={str(seed):len(expected_accounts(configs[arm],seed)) for seed in SEEDS})
    result['v13_crowd']={str(seed):dict(total=sum(g['bot']=='profit_effect' for g in expected_accounts(configs['v13'],seed).values()),
        leaders=sum(g['bot']=='profit_effect' and g['params']['leader_weight']>0 for g in expected_accounts(configs['v13'],seed).values())) for seed in SEEDS}
    assert not any(result[k] for k in ['skips','errors','disabled'])
    write(ROOT/'docs/baseline-v13-results.json',result)
    print(result,flush=True)


if __name__=='__main__': main()
