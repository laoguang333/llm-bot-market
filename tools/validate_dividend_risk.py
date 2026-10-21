"""Frozen dividend-risk comparison; execute serially through arena_gate.py."""
import argparse
import json
import os
from pathlib import Path
import statistics
import subprocess
import time
import tomllib

from validate_v18 import sha, save, opening, summarize

ROOT = Path(__file__).resolve().parents[1]
SCENARIOS = {
    'v18': 'baseline-v18.toml', 'v20': 'dividend-v20.toml',
    'balanced': 'payer-v19-balanced.toml', 'balanced-risk': 'dividend-v20-balanced.toml',
}


def ledger(a):
    capital = a['initial_equity']
    stats = a['stats']
    cost = sum(stats[k] for k in ('trade_fees', 'idle_fees', 'interest'))
    return {
        'net_return': a['equity'] / capital - 1,
        'percentile': a['percentile'],
        'dividends': stats['dividends'] / capital,
        'combo_dividends': stats['combo_dividends'] / capital,
        'costs': cost / capital,
        'terminal_value': a['terminal_value'] / capital,
        'trading_inventory_terminal_residual': (
            a['equity'] - capital - stats['dividends'] - stats['combo_dividends'] + cost) / capital,
        'liquidations': stats['liquidations'],
        'max_leverage': stats['max_leverage'],
    }


def aggregate(rows, base, candidate):
    result = {}
    for arm in (base, candidate):
        rr = [r for r in rows if r['arm'] == arm]
        result[arm] = {
            'runs': len(rr), 'quality_pass': sum(r['quality']['pass'] for r in rr),
            'quality_means': {c['name']: statistics.mean(
                next(x['value'] for x in r['quality']['checks'] if x['name'] == c['name'])
                for r in rr) for c in rr[0]['quality']['checks']},
            'groups': {g: {k: statistics.mean(r['groups'][g][k] for r in rr)
                           for k in rr[0]['groups'][g]} for g in rr[0]['groups']},
            'owners': {o: {k: statistics.mean(r['owners'][o][k] for r in rr)
                           for k in rr[0]['owners'][o]} for o in rr[0]['owners']},
            'audits': sum(r['audits'] for r in rr),
            'errors': sum(r['errors'] for r in rr),
            'disabled': sum(r['disabled'] for r in rr),
            'skipped': sum(r['skipped'] for r in rr),
        }
        for k in ('mean_thin_side_depth_cents', 'simulation_seconds', 'cli_seconds',
                  'market_net_return', 'insolvent_accounts', 'liquidated_accounts', 'bad_debt_per_initial'):
            result[arm][k] = statistics.mean(r[k] for r in rr)
    a, b = result[base], result[candidate]
    grid = 'grid' if candidate == 'v20' else 'grid_v2'
    b['acceptance'] = {
        'no_bot_faults_or_skips': b['errors'] == b['disabled'] == b['skipped'] == 0,
        'all_audits': b['audits'] == 20 * b['runs'],
        'quality_at_least_80pct': b['quality_pass'] >= 0.8 * b['runs'],
        'quality_not_lower': b['quality_pass'] >= a['quality_pass'],
        'two_sided_drop_at_most_2pp': b['quality_means']['two_sided_share'] >= a['quality_means']['two_sided_share'] - .02,
        'depth_at_least_70pct': b['mean_thin_side_depth_cents'] >= a['mean_thin_side_depth_cents'] * .7,
        'spread_at_most_150pct': b['quality_means']['median_spread'] <= a['quality_means']['median_spread'] * 1.5,
        'grid_loss_at_most_25pct': b['groups'][grid]['gross_loss_share'] <= .25,
        'grid_loss_increase_at_most_5pp': b['groups'][grid]['gross_loss_share'] <= a['groups'][grid]['gross_loss_share'] + .05,
    }
    b['accepted'] = all(b['acceptance'].values())
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('stage', choices=['shapes', 'dev', 'holdout', 'interaction'])
    args = parser.parse_args()
    from build_release import windows_build_budget
    budget = windows_build_budget() if os.name == 'nt' else os.cpu_count()
    protected = subprocess.check_output(['git', 'ls-tree', '-r', '--name-only', '694183c', '--', 'crates/bots/src'],
                                        cwd=ROOT, text=True).splitlines()
    protected = [p for p in protected if 'test' not in Path(p).name]
    subprocess.run(['git', 'diff', '--quiet', '694183c', '--', *protected], cwd=ROOT, check=True)
    paths = {a: ROOT / 'scenarios' / p for a, p in SCENARIOS.items()}
    for base, candidate in [('v18', 'v20'), ('balanced', 'balanced-risk')]:
        a, b = [tomllib.loads(paths[x].read_text(encoding='utf-8')) for x in (base, candidate)]
        assert b['world'].pop('dividend_risk')
        assert a == b, 'Only dividend risk may differ between paired scenarios'
    suffix = '.exe' if os.name == 'nt' else ''
    binary = ROOT / 'target/release' / ('llm-market' + suffix)
    source = list((ROOT / 'crates').rglob('*.rs')) + [Path(__file__), ROOT / 'docs/dividend-v20-protocol.md']
    manifest = {
        'stage': args.stage, 'threads': 4, 'cpu_budget': budget, 'repeats': 2,
        'binary_sha256': sha(binary),
        'source_hashes': {str(p.relative_to(ROOT)): sha(p) for p in source},
        'scenario_hashes': {a: sha(p) for a, p in paths.items()},
        'git_head': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(),
        'bot_review': 'Production strategies unchanged from reviewed 694183c; only test fixtures add the public rule field.',
    }
    if args.stage == 'holdout':
        previous = json.loads((ROOT / 'docs/dividend-v20-dev-results.json').read_text(encoding='utf-8'))['manifest']
        for key in ('binary_sha256', 'source_hashes', 'scenario_hashes'):
            assert manifest[key] == previous[key], 'Do not tune parameters on the holdout'
    out = ROOT / 'runs' / f'dividend-v20-{args.stage}'
    out.mkdir(exist_ok=False)
    if args.stage == 'shapes':
        manifest.update(seeds=list(range(89001, 89101)), repeats=1)
        save(out / 'manifest.json', manifest)
        results = {}
        for arm in ('v18', 'v20'):
            destination = out / f'{arm}.json'
            command = [str(binary.with_name('world-study' + suffix)), str(paths[arm]), '100', '89001', str(destination)]
            started = time.perf_counter()
            subprocess.run(command, cwd=ROOT, check=True)
            worlds = json.loads(destination.read_text(encoding='utf-8'))
            stocks = [s for w in worlds for s in w['world']['stocks']]
            cuts = [s for s in stocks if s['early_cut_tick'] is not None]
            results[arm] = {
                'stocks': len(stocks), 'early_cuts': len(cuts),
                'recovered_after_early_cut': sum(s['recovered_after_early_cut'] for s in cuts),
                'persistent_after_early_cut': sum(not s['recovered_after_early_cut'] for s in cuts),
                'long_suspension_share': statistics.mean(s['longest_zero_rate_run'] >= 1000 for s in stocks),
                'low_final_share': statistics.mean(s['final_dividend_initial_multiple'] <= .25 for s in stocks),
                'mean_dividend_initial_multiple': statistics.mean(s['mean_dividend_initial_multiple'] for s in stocks),
                'mean_final_initial_multiple': statistics.mean(s['final_dividend_initial_multiple'] for s in stocks),
                'mean_low_rate_tick_share': statistics.mean(s['low_rate_ticks'] / 10000 for s in stocks),
                'seconds': time.perf_counter() - started,
            }
        v = results['v20']
        results['accepted'] = (v['long_suspension_share'] >= .1 and v['low_final_share'] >= .2 and
                               0 < v['recovered_after_early_cut'] < v['persistent_after_early_cut'])
        save(ROOT / 'docs/dividend-v20-shapes-results.json', {'manifest': manifest, 'aggregate': results})
        print(json.dumps(results, indent=2), flush=True)
        return
    base, candidate = ('balanced', 'balanced-risk') if args.stage == 'interaction' else ('v18', 'v20')
    start, count = {'dev': (89201, 10), 'holdout': (89301, 10), 'interaction': (89401, 3)}[args.stage]
    seeds = list(range(start, start + count))
    manifest['seeds'] = seeds
    save(out / 'manifest.json', manifest)
    owners = {g['owner'] for g in tomllib.loads(paths[base].read_text(encoding='utf-8'))['bots'] if 'owner' in g}
    rows = []
    for i, seed in enumerate(seeds):
        for repeat in range(2):
            pair = {}
            arms = [base, candidate] if (i + repeat) % 2 == 0 else [candidate, base]
            for arm in arms:
                folder = out / f'{arm}-{seed}-r{repeat}'
                command = [str(binary), 'run', str(paths[arm]), '--out', str(folder), '--seed', str(seed), '--threads', '4']
                began = time.perf_counter()
                with (out / f'{arm}-{seed}-r{repeat}.log').open('w', encoding='utf-8') as log:
                    subprocess.run(command, cwd=ROOT, stdout=log, stderr=subprocess.STDOUT, check=True)
                elapsed = time.perf_counter() - began
                summary = json.loads((folder / 'summary.json').read_text(encoding='utf-8'))
                row = summarize(summary, folder)
                assert row['errors'] == row['disabled'] == 0
                assert row['audits'] == 20
                row['owners'] = {a['owner']: ledger(a) for a in summary['accounts'] if a['owner'] in owners}
                row.update(arm=arm, seed=seed, repeat=repeat, cli_seconds=elapsed,
                           summary_sha256=sha(folder / 'summary.json'), command=command,
                           world=summary['world_diagnostics'])
                pair[arm] = summary
                rows.append(row)
                save(out / 'progress.json', rows)
                print(f"{len(rows)}/{count*4} {arm} seed={seed} r={repeat} quality={summary['quality']['pass']} "
                      f"Tide={row['owners']['tide-incumbent']['net_return']:.3f}", flush=True)
            assert opening(pair[base]) == opening(pair[candidate])
            assert pair[base]['terminal_payments'] == pair[candidate]['terminal_payments']
            for a, b in zip(pair[base]['accounts'], pair[candidate]['accounts']):
                for field in ('account', 'owner', 'strategy', 'initial_cash', 'initial_shares', 'initial_equity', 'news_multiplier', 'forecast_access'):
                    assert a[field] == b[field], field
    assert sha(binary) == manifest['binary_sha256']
    assert all(sha(ROOT / p) == h for p, h in manifest['source_hashes'].items())
    assert all(sha(paths[a]) == h for a, h in manifest['scenario_hashes'].items())
    result = {'manifest': manifest, 'aggregate': aggregate(rows, base, candidate), 'rows': rows}
    save(ROOT / f'docs/dividend-v20-{args.stage}-results.json', result)
    print(json.dumps(result['aggregate'][candidate]['acceptance'], indent=2), flush=True)


if __name__ == '__main__':
    main()
