"""Fixed-incumbent Horizon research. Serial release runs, Windows half-core affinity."""
import argparse, json, os, shutil, subprocess, sys, tomllib, zipfile
from pathlib import Path
from build_release import windows_build_budget
import research_v7 as common

ROOT=Path(__file__).resolve().parents[1]
OUT=ROOT/'runs/horizon-v3-research'
PRIMARY=['v2_long','v3_control','v3_short','v3_long']
ABLATIONS=['v3_unstaged','v3_unguarded']

def setup():
    folder=OUT/'configs';folder.mkdir(exist_ok=False)
    base=(ROOT/'scenarios/baseline-v11.toml').read_text()
    for arm in PRIMARY+ABLATIONS:
        bot='horizon_v2' if arm=='v2_long' else 'horizon'
        horizon=300 if arm=='v3_short' else 1000
        extra=f'\n\n[[bots]]\nbot = "{bot}"\nowner = "researcher"\ncount = 1\ncash = 20_000_000\nshares_each = 200\nnews_multiplier = 1.0\nforecast_access = {{ horizon = {horizon}, coverage = 0.5, range_width = 3.0 }}\n'
        if arm=='v3_control':extra+='params = { research = 0.0 }\n'
        if arm=='v3_unstaged':extra+='params = { staged = 0.0 }\n'
        if arm=='v3_unguarded':extra+='params = { terminal_guard = 0.0 }\n'
        (folder/f'{arm}.toml').write_text(base+extra)
    for ver in ['v10','v11']:shutil.copy2(ROOT/f'scenarios/baseline-{ver}.toml',folder/f'{ver}.toml')
    common.write(OUT/'plan.json',dict(pilot_seeds=[7,11,23],heldout_seeds=list(range(32001,32041)),
        ablation_seeds=list(range(32001,32021)),arms=PRIMARY,ablations=ABLATIONS,
        threads=4,cpu_budget=10,performance_seeds=[7,27006,28012],performance_repeats=3,
        capital='same cash, shares and initial equity as Titan and incumbent Horizon',
        incumbent='frozen horizon_v2 with horizon=300',
        restrictions='No dragon identities, schedules, features, seed inversion or hidden world. No retuning after holdout starts.'))

def freeze(folder):
    folder.mkdir(exist_ok=False)
    for name in ['llm-market','debug-run']:
        shutil.copy2(ROOT/f'target/release/{name}.exe',folder/f'{name}.exe')
    files=sorted((ROOT/'crates').rglob('*.rs'))+sorted((ROOT/'crates').rglob('Cargo.toml'))
    files += [ROOT/'Cargo.toml',ROOT/'Cargo.lock',Path(__file__)]
    common.write(folder/'manifest.json',dict(source={str(p.relative_to(ROOT)):common.sha(p) for p in files},
        configs={p.name:common.sha(p) for p in (OUT/'configs').glob('*.toml')},
        binaries={p.name:common.sha(p) for p in folder.glob('*.exe')}))
    with zipfile.ZipFile(folder/'sources.zip','x') as z:
        for p in files:z.write(p,str(p.relative_to(ROOT)))

def one(arm,seed,group,binary,repeat=0):
    dest=OUT/group/f'{arm}-{seed}-{repeat}'
    if dest.exists():raise RuntimeError(f'Refusing overwrite: {dest}')
    s,wall=common.run_one(binary,OUT/'configs'/f'{arm}.toml',dest,seed,4)
    row=dict(arm=arm,seed=seed,repeat=repeat,wall=wall,summary=s)
    if arm.startswith('v3_') or arm=='v2_long':
        a=next(a for a in s['accounts'] if a['account']=='researcher')
        for name in ['titan-value','horizon-incumbent']:
            b=next(b for b in s['accounts'] if b['account']==name)
            assert (a['initial_cash'],a['initial_shares'],a['initial_equity'])==(b['initial_cash'],b['initial_shares'],b['initial_equity'])
        print(group,arm,seed,round(wall,2),'s',round((a['return_ratio']-1)*100,1),'%',round(a['percentile'],1),'pct',flush=True)
    else:print(group,arm,seed,repeat,round(wall,2),'s',flush=True)
    common.write(dest/'research-row.json',row)
    return row

def run(group,arms,seeds,binary):
    rows=[]
    for seed in seeds:
        for arm in arms if seed%2 else arms[::-1]:rows.append(one(arm,seed,group,binary))
        common.write(OUT/f'{group}.json',rows)

if __name__=='__main__':
    p=argparse.ArgumentParser();p.add_argument('mode',choices=['setup','pilot','study','test']);p.add_argument('--tag',default='pilot1');args=p.parse_args()
    budget=windows_build_budget() if os.name=='nt' else os.cpu_count()
    if args.mode=='setup':setup();sys.exit()
    env=os.environ.copy();env['RUSTC_WRAPPER']='';env['CARGO_BUILD_JOBS']=str(budget)
    command=['cargo','test' if args.mode=='test' else 'build','--release','--workspace']
    if args.mode=='test':command+=['--','--test-threads=1']
    subprocess.run(command,cwd=ROOT,env=env,check=True)
    if args.mode=='test':sys.exit()
    folder=OUT/(args.tag+'-frozen' if args.mode=='pilot' else 'frozen');freeze(folder)
    binary=folder/'llm-market.exe'
    if args.mode=='pilot':run(args.tag,PRIMARY,[7,11,23],binary)
    else:
        run('batch',PRIMARY,range(32001,32041),binary)
        run('ablation',ABLATIONS,range(32001,32021),binary)
        rows=[]
        for seed in [7,27006,28012]:
            for repeat in range(3):
                for arm in ['v10','v11']:rows.append(one(arm,seed,'bench',binary,repeat))
        common.write(OUT/'bench.json',rows)
