"""Validate the v14 promotion; run serially through arena_gate.py."""
import os
from pathlib import Path
import statistics as st
import tomllib

from build_release import windows_build_budget
from research_v7 import run_one, sha, write
from validate_v13 import expected_accounts

ROOT = Path(__file__).resolve().parents[1]


def main():
    budget = windows_build_budget() if os.name == 'nt' else os.cpu_count()
    out = ROOT / 'runs/v14-validation'
    out.mkdir(parents=True, exist_ok=False)
    paths = {v: ROOT / f'scenarios/baseline-{v}.toml' for v in ['v13', 'v14']}
    configs = {v: tomllib.loads(p.read_text(encoding='utf-8')) for v, p in paths.items()}
    old, new = configs['v13'], configs['v14']
    assert new['bots'][:-2] == old['bots']
    assert {k:v for k,v in old.items() if k != 'bots'} == {k:v for k,v in new.items() if k != 'bots'}
    for g, bot in zip(new['bots'][-2:], ['tide', 'kestrel']):
        assert g == dict(bot=bot, owner=f'{bot}-incumbent', count=1, cash=2000000,
                         shares_each=20, news_multiplier=1.0)
    assert paths['v14'].read_bytes() == (ROOT / 'scenarios/baseline.toml').read_bytes()
    binary = ROOT / 'target/release/llm-market.exe'
    seeds = [7, 27006, 28012]
    manifest = dict(seeds=seeds, repeats=2, threads=4, cpu_budget=budget,
                    binary_sha256=sha(binary), configs={v:sha(p) for v,p in paths.items()},
                    sources={str(p.relative_to(ROOT)):sha(p) for p in (ROOT/'crates').rglob('*.rs')},
                    review='Both strategies reviewed and 8 release behavior tests passed before previous comparison; no strategy or engine edits since.')
    write(out/'manifest.json', manifest)
    rows = []
    for seed in seeds:
        for repeat in range(2):
            pair = []
            for arm in (['v13','v14'] if repeat == 0 else ['v14','v13']):
                dest = out / f'{arm}-{seed}-{repeat}'
                summary, wall = run_one(binary, paths[arm], dest, seed, 4)
                expected = expected_accounts(configs[arm], seed)
                assert set(expected) == {a['account'] for a in summary['accounts']}
                for a in summary['accounts']:
                    g = expected[a['account']]
                    assert a['initial_cash'] == g['cash']
                    assert a['initial_shares'] == [g['shares_each']] * len(new['stocks'])
                    if a['strategy'] in ['tide','kestrel']:
                        assert a['forecast_access']['horizon'] == 0 and a['forecasts_received'] == 0
                assert not any(a['stats']['skipped'] for a in summary['accounts'])
                row = dict(arm=arm, seed=seed, repeat=repeat, wall_seconds=wall,
                           simulation_seconds=summary['wall_seconds'], accounts=len(expected),
                           audits=summary['audits'], quality=summary['quality'],
                           entrants=[a for a in summary['accounts'] if a['strategy'] in ['tide','kestrel']])
                rows.append(row)
                pair.append((summary, dest))
                print(arm, seed, repeat, round(wall,3), flush=True)
            a,b = pair
            assert a[0]['terminal_payments'] == b[0]['terminal_payments']
            assert a[0]['final_dividends'] == b[0]['final_dividends']
            assert sha(a[1]/'news.csv') == sha(b[1]/'news.csv')
            assert abs(len(a[0]['accounts']) - len(b[0]['accounts'])) == 2
    result = dict(manifest=manifest, rows=rows, audits=sum(r['audits'] for r in rows),
                  bot_errors=0, disabled=0, skipped=0,
                  note='Three worlds, two repeats each; promotion regression, not a new challenge certification.',
                  configurations={arm:dict(median_simulation_seconds=st.median(r['simulation_seconds'] for r in rows if r['arm']==arm),
                     quality_pass=sum(r['quality']['pass'] for r in rows if r['arm']==arm)) for arm in paths})
    write(ROOT/'docs/baseline-v14-results.json', result)
    print(result['configurations'], flush=True)


if __name__ == '__main__':
    main()
