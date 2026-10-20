"""Fixed-parameter profit-effect experiment; serial release runs, half-core budget.

Noise replacement is paired by account INDEX, capital and exogenous world.
Counts are first resolved exactly as v12 would draw them, then frozen before
splitting its Noise block. Thus inserting groups cannot change population RNG.
Run through arena_gate.py. No fitting or seed selection based on profit.
"""
import argparse, hashlib, json, os, re, shutil, statistics as st, subprocess, time, tomllib, zipfile
from pathlib import Path
from build_release import windows_build_budget
from research_v7 import run_one, read, write, sha

ROOT=Path(__file__).resolve().parents[1]
OUT=ROOT/'runs/profit-effect-study'
ARMS=['noise','full','peer_off','price_off']
SEEDS=list(range(33001,33021))
MASK=(1<<64)-1

def population(seed,index,g):
    lo=g.get('count',1); hi=g.get('count_max',lo)
    if hi<=lo:return lo
    h=(((seed^0xc04e7)*0x9e3779b97f4a7c15)&MASK)^(((index+1)*0xd1b54a32d192ed03)&MASK)
    h=((h^(h>>31))*0xbf58476d1ce4e5b9)&MASK
    return lo+(h>>33)%(hi-lo+1)

def scenario(seed,arm):
    text=(ROOT/'scenarios/baseline-v12.toml').read_text(encoding='utf-8')
    cfg=tomllib.loads(text)
    chunks=text.split('[[bots]]')
    result=chunks[0]
    for index,(block,g) in enumerate(zip(chunks[1:],cfg['bots'])):
        count=population(seed,index,g)
        block=re.sub(r'(?m)^count_max\s*=.*\n','',block)
        block=re.sub(r'(?m)^count\s*=.*$',f'count = {count}',block)
        if g['bot']!='noise':
            result+='[[bots]]'+block
            continue
        replaced=count//2
        remaining=count-replaced
        result+='[[bots]]'+re.sub(r'(?m)^count\s*=.*$',f'count = {remaining}',block)
        for i in range(replaced):
            new=re.sub(r'(?m)^count\s*=.*$','count = 1',block)
            strategy='noise' if arm=='noise' else 'profit_effect'
            new=new.replace('bot = "noise"',f'bot = "{strategy}"\nowner = "effect#{i}"')
            if arm!='noise':
                new+='social_group = "profit-circle"\n'
                if arm=='peer_off':new+='params = { peer_weight = 0.0 }\n'
                if arm=='price_off':new+='params = { price_weight = 0.0 }\n'
            result+='[[bots]]'+new+'\n'
    result='# Research only: frozen v12 population for world '+str(seed)+'; half Noise replaced.\n'+result
    actual=tomllib.loads(result)
    assert sum(g.get('count',1) for g in actual['bots'])==sum(population(seed,i,g) for i,g in enumerate(cfg['bots']))
    return result

def prepare():
    configs=OUT/'configs';configs.mkdir(exist_ok=True)
    for seed in [7,27006,28012]+SEEDS:
        for arm in ARMS:
            (configs/f'{arm}-{seed}.toml').write_text(scenario(seed,arm),encoding='utf-8')
    (ROOT/'scenarios/profit-effect-v12.toml').write_text(scenario(7,'full'),encoding='utf-8')
    frozen=OUT/'frozen';frozen.mkdir(exist_ok=False)
    for name in ['llm-market.exe','debug-run.exe']:
        shutil.copy2(ROOT/'target/release'/name,frozen/name)
    paths=list((ROOT/'crates').rglob('*.rs'))+list((ROOT/'crates').rglob('Cargo.toml'))
    paths += [ROOT/'Cargo.lock',ROOT/'Cargo.toml',ROOT/'tools/research_profit_effect.py',ROOT/'scenarios/baseline-v12.toml']
    with zipfile.ZipFile(frozen/'sources.zip','x',compression=zipfile.ZIP_DEFLATED) as z:
        for p in paths:z.write(p,p.relative_to(ROOT))
    write(frozen/'manifest.json',dict(source={str(p.relative_to(ROOT)):sha(p) for p in paths},
        binaries={n:sha(frozen/n) for n in ['llm-market.exe','debug-run.exe']},
        before_binary_sha256=sha(OUT/'before.exe'),configs={p.name:sha(p) for p in configs.glob('*.toml')}))
    write(OUT/'plan.json',dict(seeds=SEEDS,arms=ARMS,ticks=10000,threads=4,cpu_budget=10,
        repeats_per_world_arm=1,parameters_frozen=True,replaced='floor(noise_count/2)',
        purpose='Behavior and market effect, not beating a ranking gate',
        performance_seeds=[7,27006,28012],performance_repeats=3))

def verify_pair(rows):
    first=rows[0]['summary'];left=Path(rows[0]['directory'])
    capital=lambda s:sorted((a['account'],a['initial_cash'],a['initial_shares'],a['initial_equity']) for a in s['accounts'])
    for r in rows[1:]:
        s=r['summary'];right=Path(r['directory'])
        assert capital(s)==capital(first)
        assert s['final_dividends']==first['final_dividends'] and s['terminal_payments']==first['terminal_payments']
        assert sha(left/'news.csv')==sha(right/'news.csv')

def performance():
    binary=OUT/'frozen/llm-market.exe';config=ROOT/'scenarios/baseline-v12.toml'
    rows=[]
    for label,exe in [('before',OUT/'before.exe'),('after',binary)]:
        run_one(exe,config,OUT/'performance'/f'warm-{label}',7,4)
    for seed in [7,27006,28012]:
        for repeat in range(3):
            for label in (['before','after'] if repeat%2==0 else ['after','before']):
                exe=OUT/'before.exe' if label=='before' else binary
                dest=OUT/'performance'/f'{label}-{seed}-{repeat}'
                s,wall=run_one(exe,config,dest,seed,4)
                rows.append(dict(arm=label,seed=seed,repeat=repeat,wall=wall,summary=s,directory=str(dest)))
                write(OUT/'performance.json',rows)
        print('Performance world',seed,'completed',flush=True)
    # Single worker verifies optional channel does not change old matching,
    # settlement, information delivery, or final account results at all.
    for seed in [7,27006]:
        pair=[]
        for label,exe in [('before',OUT/'before.exe'),('after',binary)]:
            dest=OUT/'performance'/f'compat-{label}-{seed}'
            s,wall=run_one(exe,config,dest,seed,1)
            pair.append(dict(summary=s,directory=str(dest)))
        verify_pair(pair)
        assert sha(Path(pair[0]['directory'])/'trades.csv')==sha(Path(pair[1]['directory'])/'trades.csv')
        for a,b in zip(pair[0]['summary']['accounts'],pair[1]['summary']['accounts']):
            for key in ['cash','shares','equity','return_ratio','rank']:
                assert a[key]==b[key],(seed,key)
    write(OUT/'compatibility.json',dict(seeds=[7,27006],threads=1,identical_trades_news_and_final_state=True))

def batch():
    rows=[]
    for seed in SEEDS:
        pair=[]
        for arm in (ARMS if seed%2 else ARMS[::-1]):
            dest=OUT/'batch'/f'{arm}-{seed}'
            s,wall=run_one(OUT/'frozen/llm-market.exe',OUT/'configs'/f'{arm}-{seed}.toml',dest,seed,4)
            row=dict(arm=arm,seed=seed,wall=wall,summary=s,directory=str(dest));rows.append(row);pair.append(row)
            write(OUT/'batch.json',rows)
        verify_pair(pair)
        print('Paired world',seed,'completed',flush=True)

def trace():
    seed=SEEDS[0];dest=OUT/'trace'
    cmd=[str(OUT/'frozen/debug-run.exe'),str(OUT/'configs'/f'full-{seed}.toml'),'--out',str(dest),
         '--seed',str(seed),'--threads','4','--record','effect#0,effect#1']
    started=time.perf_counter();subprocess.run(cmd,cwd=ROOT,check=True)
    write(OUT/'trace-command.json',dict(command=cmd,wall=time.perf_counter()-started))

def main():
    ap=argparse.ArgumentParser();ap.add_argument('phase',choices=['prepare','performance','batch','trace','all']);args=ap.parse_args()
    if os.name=='nt':windows_build_budget()
    for phase in ['prepare','performance','batch','trace']:
        if args.phase in [phase,'all']:globals()[phase]()

if __name__=='__main__':main()
