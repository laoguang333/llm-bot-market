"""Frozen equal-capital Horizon experiments. Serial release, Windows half-core budget."""
import argparse,json,os,shutil,subprocess,sys,time,tomllib,zipfile
from pathlib import Path
from build_release import windows_build_budget
import research_v7 as common
ROOT=Path(__file__).resolve().parents[1]
OUT=ROOT/'runs/v10-research'
PRIMARY=['old1000','new0','new300','new600','new1000','titan1000']
ABLATION=['new1000_l12','new1000_full']

def setup():
    folder=OUT/'configs';folder.mkdir(exist_ok=False)
    source=(ROOT/'scenarios/baseline-v10.toml').read_text(encoding='utf-8')
    titan=next(g for g in tomllib.loads(source)['bots'] if g.get('owner')=='titan-value')
    assert (titan['cash'],titan['shares_each'])==(20_000_000,200)
    for arm in PRIMARY+ABLATION:
        bot='horizon_v1' if arm=='old1000' else 'titan' if arm=='titan1000' else 'horizon'
        horizon=0 if arm=='new0' else 300 if arm=='new300' else 600 if arm=='new600' else 1000
        text=source+f'\n[[bots]]\nbot = "{bot}"\nowner = "researcher"\ncount = 1\ncash = {titan["cash"]}\nshares_each = {titan["shares_each"]}\nnews_multiplier = 1.0\nforecast_access = {{ horizon = {horizon}, coverage = 0.5, range_width = 3.0 }}\n'
        if arm=='new1000_l12':text+='params = { leverage = 1.2 }\n'
        if arm=='new1000_full':text+='params = { research_window = 100000 }\n'
        (folder/f'{arm}.toml').write_text(text,encoding='utf-8')
    for version in ['v9','v10']:shutil.copy2(ROOT/f'scenarios/baseline-{version}.toml',folder/f'{version}.toml')
    common.write(OUT/'plan.json',dict(primary=PRIMARY,seeds=list(range(31001,31041)),
        ablation=ABLATION,ablation_seeds=list(range(31001,31021)),pilot_seeds=[7,11,23],
        performance_seeds=[7,27006,28012],repeats=3,threads=4,cpu_budget=10,
        capital='exactly matches existing Titan cash and initial inventory',
        information='same ordinary multiplier; optional 50% research coverage and width 3',
        notes='No retuning on held-out worlds. One added account per match. Benchmark baseline roster unchanged.'))

def freeze():
    dest=OUT/'frozen';dest.mkdir(exist_ok=False)
    for name in ['llm-market','debug-run','world-study']:
        shutil.copy2(ROOT/f'target/release/{name}.exe',dest/f'{name}.exe')
    files=sorted((ROOT/'crates').rglob('*.rs'))+sorted((ROOT/'crates').rglob('Cargo.toml'))
    files += [ROOT/'Cargo.toml',ROOT/'Cargo.lock',Path(__file__)]
    manifest=dict(source={str(p.relative_to(ROOT)):common.sha(p) for p in files},
        configs={p.name:common.sha(p) for p in (OUT/'configs').glob('*.toml')},
        binaries={p.name:common.sha(p) for p in dest.glob('*.exe')})
    common.write(dest/'manifest.json',manifest)
    with zipfile.ZipFile(dest/'sources.zip','x') as z:
        for p in files:z.write(p,str(p.relative_to(ROOT)))

def one(arm,seed,group,repeat=0):
    dest=OUT/group/f'{arm}-{seed}-{repeat}'
    if dest.exists():raise RuntimeError(f'Refusing overwrite {dest}')
    binary=ROOT/'target/release/llm-market.exe' if group.startswith('pilot') else OUT/'frozen/llm-market.exe'
    s,wall=common.run_one(binary,OUT/'configs'/f'{arm}.toml',dest,seed,4)
    row=dict(arm=arm,seed=seed,repeat=repeat,wall=wall,summary=s)
    if arm not in ['v9','v10']:
        a=next(a for a in s['accounts'] if a['account']=='researcher')
        t=next(a for a in s['accounts'] if a['account']=='titan-value')
        assert (a['initial_cash'],a['initial_shares'],a['initial_equity'])==(t['initial_cash'],t['initial_shares'],t['initial_equity'])
        print(group,arm,seed,round(wall,2),'s',round((a['return_ratio']-1)*100,1),'%',round(a['percentile'],1),'pct',flush=True)
    else: print(group,arm,seed,repeat,round(wall,2),'s',flush=True)
    common.write(dest/'research-row.json',row)
    return row

def run(mode):
    rows=[]
    if mode.startswith('pilot'):
        for seed in [7,11,23]:
            arms=['new0','new1000'] if mode=='pilot-execution' else ['new1000_full','new1000'] if mode=='pilot-path' else ['old1000','new0','new1000','titan1000']
            for arm in arms:rows.append(one(arm,seed,mode))
    elif mode in ['batch','ablation']:
        arms=PRIMARY if mode=='batch' else ABLATION
        seeds=range(31001,31041) if mode=='batch' else range(31001,31021)
        for seed in seeds:
            for arm in (arms if seed%2 else arms[::-1]):rows.append(one(arm,seed,mode))
            common.write(OUT/f'{mode}.json',rows)
    elif mode=='bench':
        for seed in [7,27006,28012]:
            for repeat in range(3):
                for arm in ['v9','v10']:rows.append(one(arm,seed,mode,repeat))
    common.write(OUT/f'{mode}.json',rows)

if __name__=='__main__':
    p=argparse.ArgumentParser();p.add_argument('mode',choices=['setup','freeze','build','test','pilot','pilot-execution','pilot-path','batch','ablation','bench','complete-study']);args=p.parse_args()
    budget=windows_build_budget() if os.name=='nt' else (os.cpu_count() or 1)
    if args.mode in ['build','test','complete-study']:
        env=os.environ.copy();env['RUSTC_WRAPPER']='';env['CARGO_BUILD_JOBS']=str(budget)
        cmd=['cargo','build' if args.mode=='complete-study' else args.mode,'--release','--workspace']
        if args.mode=='test':cmd+=['--','--test-threads=1']
        subprocess.run(cmd,cwd=ROOT,env=env,check=True)
        if args.mode=='complete-study':
            freeze()
            for mode in ['batch','ablation','bench']:run(mode)
    elif args.mode=='setup':setup()
    elif args.mode=='freeze':freeze()
    else:run(args.mode)
