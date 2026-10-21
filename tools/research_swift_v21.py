"""Frozen Swift diagnostics. All market runs require arena_gate.py."""
import argparse
import json
import os
from pathlib import Path
import re
import shutil
import statistics as st
import subprocess
import time
import tomllib

from validate_v18 import sha, save, opening

ROOT=Path(__file__).resolve().parents[1]
OUT=ROOT/'runs/swift-v21-diagnosis'


def read(path):
    return json.loads(path.read_text(encoding='utf-8'))


def account_metrics(a):
    initial=a['initial_equity']
    stats=a['stats']
    metrics={k:stats[k]/initial for k in ('dividends','combo_dividends','idle_cash_fees','idle_stock_fees','trade_fees','interest','bad_debt','volume_notional')}
    metrics.update(net_return=a['equity']/initial-1, final_cash=a['cash']/initial,
                   terminal_value=a['terminal_value']/initial, trades=stats['trades'], liquidations=stats['liquidations'],
                   max_leverage=stats['max_leverage'], shares=sum(a['shares']))
    # Includes trading cash flow, initial inventory disposition and terminal.
    # Stock burn is already reflected in ending inventory; never deduct twice.
    metrics['trading_inventory_terminal']=(metrics['net_return']-metrics['dividends']-metrics['combo_dividends']
        +metrics['idle_cash_fees']+metrics['trade_fees']+metrics['interest']-metrics['bad_debt'])
    return metrics


def prepare():
    OUT.mkdir(exist_ok=False)
    (OUT/'frozen').mkdir()
    (OUT/'configs').mkdir()
    for name in ('llm-market.exe','debug-run.exe'):
        shutil.copy2(ROOT/'target/release'/name,OUT/'frozen'/name)
    for arm in ('control','both','no-depth','no-momentum','standard-single'):
        source=(ROOT/f'scenarios/ecology-v21-{"control" if arm=="control" else "both"}.toml').read_text(encoding='utf-8')
        sections=source.split('[[bots]]')
        for i,section in enumerate(sections):
            if 'bot = "swift"' not in section: continue
            if arm in ('no-depth','no-momentum'):
                key=arm[3:]
                sections[i]=re.sub(rf'\b{key}\s*=\s*[0-9.]+',f'{key} = 0.0',section)
            elif arm=='standard-single' and 'owner = "swift-standard"' in section:
                sections[i]=re.sub(r'params = \{([^}]+)\}',r'params = {\1, single_entry = 1.0 }',section)
        cfg=OUT/'configs'/f'{arm}.toml'
        cfg.write_text('[[bots]]'.join(sections),encoding='utf-8')
        parsed=tomllib.loads(cfg.read_text(encoding='utf-8'))
        if arm in ('no-depth','no-momentum','standard-single'):
            base=tomllib.loads((ROOT/'scenarios/ecology-v21-both.toml').read_text(encoding='utf-8'))
            for a,b in zip(parsed['bots'],base['bots']):
                if a['bot']=='swift':
                    if arm=='standard-single' and a['owner']=='swift-standard': assert a['params'].pop('single_entry')==1
                    elif arm!='standard-single':
                        key=arm[3:]
                        assert a['params'][key]==0
                        a['params'][key]=b['params'][key]
            assert parsed==base
    frozen=dict(sources={str(p.relative_to(ROOT)):sha(p) for p in (ROOT/'crates').rglob('*.rs')},
                binaries={n:sha(OUT/'frozen'/n) for n in ('llm-market.exe','debug-run.exe')},
                configs={p.name:sha(p) for p in (OUT/'configs').glob('*.toml')},
                protocol_sha256=sha(ROOT/'docs/swift-v21-diagnosis-protocol.md'),
                runner_sha256=sha(Path(__file__)))
    save(OUT/'manifest.json',frozen)


def check_frozen():
    frozen=read(OUT/'manifest.json')
    for p,h in frozen['sources'].items(): assert sha(ROOT/p)==h,p
    for p,h in frozen['binaries'].items(): assert sha(OUT/'frozen'/p)==h,p
    for p,h in frozen['configs'].items(): assert sha(OUT/'configs'/p)==h,p
    assert sha(ROOT/'docs/swift-v21-diagnosis-protocol.md')==frozen['protocol_sha256']
    assert sha(Path(__file__))==frozen['runner_sha256']


def run(stage):
    check_frozen()
    out=OUT/stage
    out.mkdir(exist_ok=False)
    trace=stage=='traces'
    arms=['control','both'] if trace else ['control','both','no-depth','no-momentum','standard-single']
    seeds=range(90601,90605) if trace else range(90701,90707)
    repeats=1 if trace else 2
    rows=[]
    for i,seed in enumerate(seeds):
        for repeat in range(repeats):
            shift=(i+repeat)%len(arms)
            previous=None
            for arm in arms[shift:]+arms[:shift]:
                folder=out/f'{arm}-{seed}-r{repeat}'
                cfg=OUT/'configs'/f'{arm}.toml'
                if trace:
                    command=[str(OUT/'frozen/debug-run.exe'),str(cfg),'--out',str(folder),'--seed',str(seed),'--threads','4','--record','swift']
                else:
                    command=[str(OUT/'frozen/llm-market.exe'),'run',str(cfg),'--out',str(folder),'--seed',str(seed),'--threads','4']
                begin=time.perf_counter()
                with folder.with_suffix('.log').open('w',encoding='utf-8') as log:
                    subprocess.run(command,cwd=ROOT,stdout=log,stderr=subprocess.STDOUT,check=True)
                elapsed=time.perf_counter()-begin
                market=folder/'market' if trace else folder
                summary=read(market/'summary.json')
                assert summary['audits']==20
                assert all(a['stats']['bot_errors']==a['stats']['skipped']==0 and not a['disabled'] for a in summary['accounts'])
                if trace:
                    manifest=read(folder/'trace-manifest.json')
                    assert all(v['swift_mismatches']==0 and v['rows']==10001 for v in manifest['verification'])
                if previous:
                    assert opening(summary)==opening(previous)
                    assert summary['terminal_payments']==previous['terminal_payments']
                    assert sha(market/'news.csv')==sha(previous_market/'news.csv')
                    for a,b in zip(summary['accounts'],previous['accounts']):
                        for field in ('account','owner','initial_cash','initial_shares','initial_equity','news_multiplier','forecast_access'):
                            # Anonymous renamed group identities intentionally differ.
                            if field in ('account','owner') and a['strategy'] in ('value','harbor','coral','relay'): continue
                            assert a[field]==b[field],field
                previous,previous_market=summary,market
                row=dict(arm=arm,seed=seed,repeat=repeat,folder=str(folder),command=command,cli_seconds=elapsed,
                         simulation_seconds=summary['wall_seconds'],summary_sha256=sha(market/'summary.json'),
                         accounts={a['owner']:account_metrics(a) for a in summary['accounts'] if a['strategy']=='swift'})
                rows.append(row)
                save(out/'progress.json',rows)
                print(f"{len(rows)}/{len(seeds)*repeats*len(arms)} {arm} {seed} r{repeat}: "+
                      ', '.join(f'{k} {v["net_return"]:+.3f}' for k,v in row['accounts'].items()),flush=True)
    check_frozen()
    result=dict(manifest=read(OUT/'manifest.json'),rows=rows)
    save(ROOT/f'docs/swift-v21-{stage}-results.json',result)


def main():
    ap=argparse.ArgumentParser(description=__doc__)
    ap.add_argument('stage',choices=['prepare','traces','ablations'])
    stage=ap.parse_args().stage
    if os.name=='nt':
        from build_release import windows_build_budget
        windows_build_budget()
    if stage=='prepare': prepare()
    else: run(stage)


if __name__=='__main__': main()
