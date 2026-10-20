"""Frozen, serial release experiments for v9; no return-based parameter search."""
import argparse, json, os, shutil, statistics, subprocess, sys, time, zipfile
from pathlib import Path
from build_release import windows_build_budget
import research_v7 as common
ROOT=Path(__file__).resolve().parents[1]
OUT=ROOT/'runs/v9-research'

def setup():
    text=(ROOT/'scenarios/baseline-v8.toml').read_text(encoding='utf-8')
    text=text.replace('# v8:', '# v9:').replace('model = "trend"','model = "growth"').replace('growth_log_bias = [1.0, 1.5]','growth_log_bias = [2.3, 2.65]').replace('max_step = 0.04','max_step = 0.16').replace('events_per_1000_ticks = 5.0','events_per_1000_ticks = 3.5')
    (ROOT/'scenarios/baseline-v9.toml').write_text(text,encoding='utf-8')

def prepare():
    OUT.mkdir(exist_ok=True,parents=True)
    folder=OUT/'configs';folder.mkdir(exist_ok=False)
    source=(ROOT/'scenarios/baseline-v9.toml').read_text(encoding='utf-8')
    for horizon in [0,300,600,1000]:
        text=source+'\n[[bots]]\nbot = "horizon"\nowner = "researcher"\ncount = 1\ncash = 20000000\nshares_each = 200\n'
        text+=f'forecast_access = {{ horizon = {horizon}, coverage = 0.5, range_width = 3.0 }}\n'
        (folder/f'h{horizon}.toml').write_text(text,encoding='utf-8')
    shutil.copy2(ROOT/'scenarios/baseline-v8.toml',folder/'v8.toml')
    shutil.copy2(ROOT/'scenarios/baseline-v9.toml',folder/'v9.toml')
    for name in ['llm-market','world-study','debug-run']:
        shutil.copy2(ROOT/f'target/release/{name}.exe',OUT/f'{name}.exe')
    files=sorted((ROOT/'crates').rglob('*.rs'))+sorted((ROOT/'crates').rglob('Cargo.toml'))+[ROOT/'Cargo.toml',ROOT/'Cargo.lock',Path(__file__)]
    common.write(OUT/'manifest.json',dict(source={str(p.relative_to(ROOT)):common.sha(p) for p in files},
        binary=common.sha(OUT/'llm-market.exe'),configs={p.name:common.sha(p) for p in folder.iterdir()},
        seeds=list(range(29001,29041)),performance_seeds=[7,27006,28012],repeats=3,threads=4,cpu_budget=10))
    with zipfile.ZipFile(OUT/'sources.zip','x') as archive:
        for p in files:archive.write(p,str(p.relative_to(ROOT)))

def one(arm,seed,group,repeat=0):
    dest=OUT/group/f'{arm}-{seed}-{repeat}'
    if dest.exists():raise RuntimeError(f'Refusing overwrite {dest}')
    summary,wall=common.run_one(OUT/'llm-market.exe',OUT/'configs'/f'{arm}.toml',dest,seed,4)
    row=dict(arm=arm,seed=seed,repeat=repeat,wall=wall,summary=summary)
    print(group,arm,seed,repeat,round(wall,3),'seconds',flush=True)
    return row

def run(mode):
    rows=[]
    if mode=='bench':
        for seed in [7,27006,28012]:
            for rep in range(3):
                for arm in ['v8','v9']:rows.append(one(arm,seed,mode,rep))
    elif mode=='batch':
        for seed in range(29001,29041):
            for horizon in [0,300,600,1000]:rows.append(one(f'h{horizon}',seed,mode))
    elif mode=='pilot':
        for seed in [7,11]:
            for arm in ['v9','h0','h1000']:rows.append(one(arm,seed,mode))
    common.write(OUT/f'{mode}.json',rows)

if __name__=='__main__':
    p=argparse.ArgumentParser();p.add_argument('mode',choices=['setup','build','test','prepare','pilot','bench','batch','world']);a=p.parse_args()
    budget=windows_build_budget() if os.name=='nt' else (os.cpu_count() or 1)
    if a.mode=='setup':setup()
    elif a.mode in ['build','test']:
        env=os.environ.copy();env['RUSTC_WRAPPER']='';env['CARGO_BUILD_JOBS']=str(budget)
        command=['cargo',a.mode,'--release','--workspace']
        if a.mode=='test':command+=['--','--test-threads=1']
        subprocess.run(command,cwd=ROOT,env=env,check=True)
    elif a.mode=='prepare':prepare()
    elif a.mode=='world':
        OUT.mkdir(exist_ok=True,parents=True)
        subprocess.run([str(ROOT/'target/release/world-study.exe'),str(ROOT/'scenarios/baseline-v9.toml'),'400','30001',str(OUT/'world-shapes.json')],check=True)
        rows=json.loads((OUT/'world-shapes.json').read_text())
        selected=[r['world']['stocks'][r['world']['selected_dragon']] for r in rows if r['world']['selected_dragon'] is not None]
        print(dict(worlds=len(rows),dragons=len(selected),interval=statistics.mean(r['world']['mean_stock_news_interval'] for r in rows),
            peak_median=statistics.median(s['dividend_peak_initial_multiple'] for s in selected),
            peak_min=min(s['dividend_peak_initial_multiple'] for s in selected),peak_max=max(s['dividend_peak_initial_multiple'] for s in selected),
            at_least_8=sum(s['dividend_peak_initial_multiple']>=8 for s in selected),
            at_least_10=sum(s['dividend_peak_initial_multiple']>=10 for s in selected),
            pullbacks=sum(s['max_dividend_drawdown']>=.1 for s in selected)))
    else:run(a.mode)
