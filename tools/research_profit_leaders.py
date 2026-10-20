"""Fixed minority market-leader followers; paired v12 populations and capital.

Run with arena_gate.py. No fitting to outcomes; preserve the previous study.
"""
import argparse
import os
from pathlib import Path
import re
import shutil
import subprocess
import time
import tomllib
import zipfile

from build_release import windows_build_budget
from research_profit_effect import scenario as previous_scenario, verify_pair
from research_v7 import run_one, read, write, sha

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / 'runs/profit-effect-leaders'
SEEDS = list(range(34001, 34021))
ARMS = ['group', 'group_leaders', 'full', 'full_leaders']


def scenario(seed, arm):
    base = previous_scenario(seed, 'price_off' if arm.startswith('group') else 'full')
    chunks = base.split('[[bots]]')
    result = chunks[0]
    for block in chunks[1:]:
        cfg = tomllib.loads(block)
        if cfg['bot'] == 'profit_effect':
            index = int(cfg['owner'].split('#')[1])
            params = dict(cfg.get('params', {}))
            # Membership/weights fixed before outcomes; not derived from news.
            if arm.endswith('_leaders') and index % 3 == 0:
                params['leader_weight'] = [0.5, 0.8, 1.1][(index // 3) % 3]
            block = re.sub(r'(?m)^params\s*=.*\n', '', block)
            if params:
                block += 'params = { ' + ', '.join(f'{k} = {v}' for k, v in params.items()) + ' }\n'
        result += '[[bots]]' + block
    tomllib.loads(result)
    return result


def check():
    subprocess.run(['rustfmt', '--edition', '2021', 'crates/bots/src/profit_effect.rs',
                    'crates/bots/src/profit_effect_tests.rs'], cwd=ROOT, check=True)
    subprocess.run(['cargo', 'test', '--release', '--workspace', '--jobs', '10', '--',
                    '--test-threads=10'], cwd=ROOT, check=True)
    subprocess.run(['cargo', 'build', '--release', '--jobs', '10', '--bin', 'llm-market',
                    '--bin', 'debug-run'], cwd=ROOT, check=True)


def prepare():
    OUT.mkdir(parents=True, exist_ok=True)
    configs = OUT / 'configs'; configs.mkdir(exist_ok=False)
    for seed in SEEDS + [7, 27006]:
        for arm in ARMS:
            (configs / f'{arm}-{seed}.toml').write_text(scenario(seed, arm), encoding='utf-8')
    original = ROOT / 'scenarios/profit-effect-v12-original.toml'
    if not original.exists():
        shutil.copy2(ROOT / 'scenarios/profit-effect-v12.toml', original)
    (ROOT / 'scenarios/profit-effect-v12.toml').write_text(scenario(7, 'full_leaders'), encoding='utf-8')
    frozen = OUT / 'frozen'; frozen.mkdir(exist_ok=False)
    for name in ['llm-market.exe', 'debug-run.exe']:
        shutil.copy2(ROOT / 'target/release' / name, frozen / name)
    paths = list((ROOT / 'crates').rglob('*.rs')) + list((ROOT / 'crates').rglob('Cargo.toml'))
    paths += [ROOT / n for n in ['Cargo.toml', 'Cargo.lock', 'tools/research_profit_leaders.py',
              'tools/research_profit_effect.py', 'tools/research_v7.py', 'scenarios/baseline-v12.toml']]
    with zipfile.ZipFile(frozen / 'sources.zip', 'x', compression=zipfile.ZIP_DEFLATED) as z:
        for p in paths: z.write(p, p.relative_to(ROOT))
    write(frozen / 'manifest.json', dict(source={str(p.relative_to(ROOT)): sha(p) for p in paths},
        binaries={n: sha(frozen / n) for n in ['llm-market.exe', 'debug-run.exe']},
        configs={p.name: sha(p) for p in configs.glob('*.toml')}))
    write(OUT / 'plan.json', dict(seeds=SEEDS, arms=ARMS, ticks=10000, threads=4,
        cpu_budget=10, repeats_per_world_arm=1, parameters_frozen=True,
        leader_members='index % 3 == 0', leader_weights=[0.5, 0.8, 1.1],
        purpose='Restart trading and spread actual experience; not optimize profit'))


def compatibility():
    rows = []
    for seed in [7, 27006]:
        pair = []
        for label, binary in [('old', ROOT / 'runs/profit-effect-study/frozen/llm-market.exe'),
                              ('new', OUT / 'frozen/llm-market.exe')]:
            dest = OUT / 'compatibility' / f'{label}-{seed}'
            if dest.exists(): raise RuntimeError(f'refusing to overwrite {dest}')
            s, wall = run_one(binary, OUT / 'configs' / f'full-{seed}.toml', dest, seed, 1)
            pair.append(dict(summary=s, directory=str(dest)))
        verify_pair(pair)
        assert sha(Path(pair[0]['directory']) / 'trades.csv') == sha(Path(pair[1]['directory']) / 'trades.csv')
        for a, b in zip(pair[0]['summary']['accounts'], pair[1]['summary']['accounts']):
            assert all(a[k] == b[k] for k in ['cash', 'shares', 'equity', 'rank'])
        rows.append(dict(seed=seed, identical_trades_news_cash_shares_ranks=True))
    write(OUT / 'compatibility.json', rows)


def batch():
    rows = []
    for seed in SEEDS:
        pair = []
        for arm in (ARMS if seed % 2 else ARMS[::-1]):
            dest = OUT / 'batch' / f'{arm}-{seed}'
            if dest.exists(): raise RuntimeError(f'refusing to overwrite {dest}')
            s, wall = run_one(OUT / 'frozen/llm-market.exe', OUT / 'configs' / f'{arm}-{seed}.toml', dest, seed, 4)
            row = dict(arm=arm, seed=seed, wall=wall, summary=s, directory=str(dest))
            rows.append(row); pair.append(row)
            write(OUT / 'batch.json', rows)
        verify_pair(pair)
        print('Paired world', seed, 'completed', flush=True)


def trace():
    seed = SEEDS[0]
    if (OUT / 'trace').exists(): raise RuntimeError('refusing to overwrite trace')
    cmd = [str(OUT / 'frozen/debug-run.exe'), str(OUT / 'configs' / f'full_leaders-{seed}.toml'),
           '--out', str(OUT / 'trace'), '--seed', str(seed), '--threads', '4', '--record', 'effect#0,effect#1']
    started = time.perf_counter(); subprocess.run(cmd, cwd=ROOT, check=True)
    write(OUT / 'trace-command.json', dict(command=cmd, wall=time.perf_counter() - started))


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('phase', choices=['check', 'prepare', 'compatibility', 'batch', 'trace', 'all'])
    phase = ap.parse_args().phase
    if os.name == 'nt': windows_build_budget()
    for name in ['check', 'prepare', 'compatibility', 'batch', 'trace']:
        if phase in [name, 'all']: globals()[name]()


if __name__ == '__main__': main()
