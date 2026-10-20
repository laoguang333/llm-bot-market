"""Paired earlier-growth and Osprey specialist experiments. Run via arena_gate.py."""
import argparse
import csv
import json
import os
from pathlib import Path
import statistics as st
import subprocess
import sys
import time
import zipfile

from build_release import windows_build_budget
from research_v7 import sha, write

ROOT=Path(__file__).resolve().parents[1]
ARMS={'late_old':('v14','osprey_v1',1),'early_old':('v15','osprey_v1',1),
      'early_new':('v15','osprey',1),'early_control':('v15','osprey',0)}


def metrics(folder, cfg):
    s=json.loads((folder/'summary.json').read_text(encoding='utf-8'))
    record=json.loads((folder/'stock-record.json').read_text(encoding='utf-8'))
    a=next(a for a in s['accounts'] if a['owner']=='osprey-research')
    assert a['initial_cash']==2000000 and a['initial_shares']==[20]*10
    assert a['news_multiplier']==1 and a['forecasts_received']==0
    assert all(not a['disabled'] and a['stats']['bot_errors']==0 and a['stats']['skipped']==0 for a in s['accounts'])
    out=dict(return_pct=(a['return_ratio']-1)*100,percentile=a['percentile'],
        audits=s['audits'],seconds=s['wall_seconds'],stats=a['stats'],
        quality_pass=s['quality']['pass'],quality_failures=[c['name'] for c in s['quality']['checks'] if not c['pass']],dragon=s['dragon']['selected_stock'],
        cutoff=s['dragon']['divergence_tick'],terminal_payments=s['terminal_payments'])
    chosen=out['dragon']
    if chosen is not None:
        l=record['ledger'][chosen]
        terminal=(a['shares'][chosen]*s['final_dividends'][chosen]*s['terminal_payments']*(s['terminal_payment_ticks']//s['dividend_period'])+s['dividend_scale']//2)//s['dividend_scale']
        initial=a['initial_shares'][chosen]*record['opening_prices'][chosen]
        pnl=l['sold']-l['bought']-l['fees']+l['dividends']+terminal-initial
        late=[p for p in record['samples'] if p['tick']>=out['cutoff']]
        exposure=[p['shares'][chosen]*p['marks'][chosen]/max(p['equity'],1) for p in late]
        out['dragon_contribution_pct']=pnl/a['initial_equity']*100
        out['dragon_dividend_pct']=l['dividends']/a['initial_equity']*100
        out['dragon_late_mean_exposure']=st.mean(exposure)
        out['dragon_late_exposure_over_10pct']=sum(x>=.1 for x in exposure)/len(exposure)
        name=cfg['stocks'][chosen]['name']
        with (folder/'bars.csv').open(encoding='utf-8',newline='') as f:
            bars=[r for r in csv.DictReader(f) if r['stock']==name and r['high']]
        out['dragon_price_peak_multiple']=max(int(r['high']) for r in bars)/record['opening_prices'][chosen]
        out['dragon_dividend_peak_multiple']=s['world_diagnostics']['stocks'][chosen]['dividend_peak_initial_multiple']
    return out


def summary(rows):
    result={}
    for label in ['all','dragon','no_dragon']:
        chosen=[r for r in rows if label=='all' or (r['early_new']['dragon'] is not None)==(label=='dragon')]
        if not chosen: continue
        result[label]={'worlds':len(chosen),'arms':{}}
        for arm in ARMS:
            rr=[r[arm] for r in chosen]
            result[label]['arms'][arm]={k:st.mean(r[k] for r in rr) for k in [
                'return_pct','percentile','seconds']+(['dragon_contribution_pct','dragon_late_mean_exposure',
                'dragon_late_exposure_over_10pct','dragon_price_peak_multiple','dragon_dividend_peak_multiple'] if label=='dragon' else [])}
        for base in ['early_control','early_old']:
            delta=[r['early_new']['return_pct']-r[base]['return_pct'] for r in chosen]
            result[label][f'new_vs_{base}']=dict(wins=sum(x>0 for x in delta),mean_delta=st.mean(delta),median_delta=st.median(delta))
            if label=='dragon':
                dd=[r['early_new']['dragon_contribution_pct']-r[base]['dragon_contribution_pct'] for r in chosen]
                result[label][f'dragon_contribution_vs_{base}']=dict(wins=sum(x>0 for x in dd),mean_delta=st.mean(dd),median_delta=st.median(dd))
    return result


def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('name'); p.add_argument('--first-seed',type=int,required=True);p.add_argument('--seeds',type=int,required=True)
    args=p.parse_args()
    budget=windows_build_budget() if os.name=='nt' else os.cpu_count()
    out=ROOT/'runs'/args.name;out.mkdir(parents=True,exist_ok=False)
    import tomllib
    configs={}
    for arm,(version,bot,strength) in ARMS.items():
        text=(ROOT/f'scenarios/baseline-{version}.toml').read_text(encoding='utf-8')
        text+=f'\n[[bots]]\nbot = "{bot}"\nowner = "osprey-research"\ncount = 1\ncash = 2000000\nshares_each = 20\nnews_multiplier = 1.0\nparams = {{ strength = {strength}.0 }}\n'
        (out/f'{arm}.toml').write_text(text,encoding='utf-8');configs[arm]=tomllib.loads(text)
    binary=ROOT/'target/release/osprey-eval.exe'
    manifest=dict(seeds=list(range(args.first_seed,args.first_seed+args.seeds)),repeats=1,threads=4,cpu_budget=budget,
        source={str(p.relative_to(ROOT)):sha(p) for p in list((ROOT/'crates').rglob('*.rs'))+[Path(__file__).resolve()]},
        binary_sha256=sha(binary),configs={a:sha(out/f'{a}.toml') for a in ARMS},
        review='Main agent reviewed both Osprey versions, Kestrel execution, config/world changes and read-only observer. No hidden labels enter strategies. Review/tests precede runs.',
        contribution_definition='Per-stock sale proceeds minus purchase costs and trade fees, plus ordinary dividends and ordinary terminal value, minus starting inventory value. Excludes shared idle fees, interest and combo income; not account net profit.')
    write(out/'manifest.json',manifest)
    with zipfile.ZipFile(out/'source.zip','x') as z:
        for path in manifest['source']: z.write(ROOT/path,path)
    rows=[]
    started=time.perf_counter()
    for seed in manifest['seeds']:
        row={'seed':seed}
        order=list(ARMS)
        if seed%2: order.reverse()
        for arm in order:
            dest=out/f'{arm}-{seed}'
            command=[str(binary),str(out/f'{arm}.toml'),str(seed),str(dest)]
            launched=time.perf_counter()
            with (out/f'{arm}-{seed}.log').open('w',encoding='utf-8') as log:
                subprocess.run(command,cwd=ROOT,stdout=log,stderr=subprocess.STDOUT,check=True)
            process_seconds=time.perf_counter()-launched
            write(dest/'command.json',command)
            row[arm]=metrics(dest,configs[arm])
            row[arm]['process_seconds']=process_seconds
        assert len({row[a]['dragon'] for a in ARMS})==1
        assert len({row[a]['terminal_payments'] for a in ARMS})==1
        assert len({sha(out/f'{a}-{seed}'/'news.csv') for a in ['early_new','early_old','early_control']})==1
        rows.append(row)
        write(out/'results.json',dict(manifest=manifest,rows=rows,summary=summary(rows),elapsed_seconds=time.perf_counter()-started))
        print(seed,'dragon',row['early_new']['dragon'],*[f'{a}={row[a]["return_pct"]:.1f}%' for a in ARMS],flush=True)
    assert sha(binary)==manifest['binary_sha256']
    assert all(sha(ROOT/path)==digest for path,digest in manifest['source'].items())
    assert all(sha(out/f'{arm}.toml')==digest for arm,digest in manifest['configs'].items())
    print(json.dumps(summary(rows),ensure_ascii=False,indent=2),flush=True)


if __name__=='__main__':main()
