"""Serial paired Osprey research in frozen v14; world labels are post-run only.

Run through arena_gate.py. Each output name must be new. No parameter tuning here.
"""
import argparse
import os
from pathlib import Path
import statistics as st
import zipfile

from build_release import windows_build_budget
from research_v7 import run_one, sha, write

ROOT = Path(__file__).resolve().parents[1]


def summarize(rows):
    result = {}
    for group in ['all', 'dragon', 'no_dragon']:
        chosen = [r for r in rows if group == 'all' or (r['dragon'] is not None) == (group == 'dragon')]
        if not chosen:
            continue
        item = {'worlds': len(chosen)}
        for arm in ['control', 'osprey']:
            aa = [r[arm] for r in chosen]
            item[arm] = dict(mean_return_pct=st.mean(a['return_pct'] for a in aa),
                            median_return_pct=st.median(a['return_pct'] for a in aa),
                            worst_return_pct=min(a['return_pct'] for a in aa),
                            mean_percentile=st.mean(a['percentile'] for a in aa),
                            top25=sum(a['percentile'] >= 75 for a in aa),
                            liquidations=sum(a['liquidations'] for a in aa),
                            beat_incumbent_kestrel=sum(a['return_pct'] > a['kestrel_return_pct'] for a in aa))
        delta = [r['osprey']['return_pct'] - r['control']['return_pct'] for r in chosen]
        item['overlay_wins'] = sum(d > 0 for d in delta)
        item['mean_delta_pp'] = st.mean(delta)
        item['median_delta_pp'] = st.median(delta)
        result[group] = item
    return result


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('name')
    p.add_argument('--first-seed', type=int, required=True)
    p.add_argument('--seeds', type=int, required=True)
    args = p.parse_args()
    budget = windows_build_budget() if os.name == 'nt' else os.cpu_count()
    out = ROOT / 'runs' / args.name
    out.mkdir(parents=True, exist_ok=False)
    baseline = ROOT / 'scenarios/baseline-v14.toml'
    configs = {}
    for arm, strength in [('control',0),('osprey',1)]:
        configs[arm] = out / f'{arm}.toml'
        configs[arm].write_text(baseline.read_text(encoding='utf-8') + f'''
[[bots]]
bot = "osprey_v1"
owner = "osprey-research"
count = 1
cash = 2_000_000
shares_each = 20
news_multiplier = 1.0
params = {{ strength = {strength}.0 }}
''', encoding='utf-8')
    binary = ROOT / 'target/release/llm-market.exe'
    manifest = dict(seeds=list(range(args.first_seed,args.first_seed+args.seeds)),
                    repeats=1, threads=4, cpu_budget=budget, binary_sha256=sha(binary),
                    configs={a:sha(p) for a,p in configs.items()},
                    source={str(p.relative_to(ROOT)):sha(p) for p in (ROOT/'crates').rglob('*.rs')},
                    review='Main agent read Osprey, Kestrel and common helpers before running. Inputs restricted to Rules and own observations. No seed/identity/dragon metadata/external IO. Bot release tests passed before running; see checks log. Control strength=0 matches Kestrel in deterministic observation tests.')
    write(out/'manifest.json',manifest)
    with zipfile.ZipFile(out/'source.zip','x') as archive:
        for path in manifest['source']:
            archive.write(ROOT/path,path)
        archive.write(Path(__file__), 'tools/research_osprey.py')
    rows=[]
    for seed in manifest['seeds']:
        row={'seed':seed}
        summaries=[]
        for arm in (['control','osprey'] if seed%2 else ['osprey','control']):
            dest=out/f'{arm}-{seed}'
            summary,wall=run_one(binary,configs[arm],dest,seed,4)
            summaries.append((summary,dest))
            a=next(a for a in summary['accounts'] if a['owner']=='osprey-research')
            k=next(a for a in summary['accounts'] if a['owner']=='kestrel-incumbent')
            t=next(a for a in summary['accounts'] if a['owner']=='tide-incumbent')
            assert a['initial_cash']==2000000 and a['initial_shares']==[20]*10
            assert a['forecasts_received']==0 and a['forecast_access']['horizon']==0
            assert not any(a['stats']['skipped'] for a in summary['accounts'])
            row['dragon']=summary['dragon']['selected_stock']
            row[arm]=dict(return_pct=(a['return_ratio']-1)*100,percentile=a['percentile'],
                          liquidations=a['stats']['liquidations'],stats=a['stats'],
                          kestrel_return_pct=(k['return_ratio']-1)*100,
                          tide_return_pct=(t['return_ratio']-1)*100,
                          simulation_seconds=summary['wall_seconds'],process_seconds=wall,
                          audits=summary['audits'],quality=summary['quality']['pass'])
        a,b=summaries
        assert sha(a[1]/'news.csv')==sha(b[1]/'news.csv')
        assert a[0]['dragon']['selected_stock']==b[0]['dragon']['selected_stock']
        assert a[0]['final_dividends']==b[0]['final_dividends']
        rows.append(row)
        write(out/'results.json',dict(manifest=manifest,summary=summarize(rows),rows=rows))
        print(seed,'dragon',row['dragon'],*[f'{arm}={row[arm]["return_pct"]:.1f}%' for arm in configs],flush=True)
    assert sha(binary)==manifest['binary_sha256']
    print(summarize(rows),flush=True)


if __name__=='__main__':
    main()
