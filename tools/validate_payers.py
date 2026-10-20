"""Frozen v19 crowd experiments; run serially through arena_gate.py."""
import argparse
from collections import Counter, defaultdict
import csv
import json
import os
from pathlib import Path
import statistics
import subprocess
import time
import tomllib

from validate_v18 import opening, save, sha, summarize

ROOT = Path(__file__).resolve().parents[1]
PAYERS = ('profit_effect', 'news_reactor', 'noise', 'latecomer', 'grid')
NEW = ('news_reactor', 'latecomer')
REPLACED = {'momentum': 'latecomer', 'hold': 'news_reactor', 'grid': 'grid_v2'}
SOURCES = ['crates/bots/src/crowd_execution.rs', 'crates/bots/src/news_reactor.rs', 'crates/bots/src/grid_v2.rs',
           'crates/bots/src/latecomer.rs', 'crates/bots/src/lib.rs',
           'crates/cli/tests/payer_crowds.rs', 'crates/cli/tests/payer_scenarios.rs',
           'tools/validate_payers.py', 'tools/validate_v18.py']


def ledger(accounts):
    initial = sum(a['initial_equity'] for a in accounts)
    if not initial:
        return {}
    result = {k: sum(a['stats'][k] for a in accounts) / initial for k in
              ['trade_fees', 'idle_fees', 'idle_cash_fees', 'idle_stock_fees',
               'interest', 'dividends', 'combo_dividends', 'bad_debt']}
    result['net_return'] = sum(a['equity'] - a['initial_equity'] for a in accounts) / initial
    result['trading_inventory_terminal_residual'] = (
        result['net_return'] - result['dividends'] - result['combo_dividends']
        + result['trade_fees'] + result['idle_fees'] + result['interest'] - result['bad_debt'])
    return result


def trading(summary, folder):
    """Exact activity; sampled external-trade markouts, explicitly not realized P&L.

    Sample first external fill per account/stock/100-tick bucket. Future midpoint
    is the first 10-tick book sample at/after fill+h (h..h+9). Missing two-sided
    samples are omitted and counted. Gross ordinary dividends use the actual
    per-tick bars, without account rounding or combo income. Both sides are signed.
    """
    accounts = {a['account']: a for a in summary['accounts']}
    types = {a: v['strategy'] for a, v in accounts.items()}
    selected = set(PAYERS) & set(types.values())
    n = summary['ticks']
    mids = defaultdict(dict)
    with (folder / 'books.csv').open(newline='', encoding='utf-8') as f:
        for r in csv.DictReader(f):
            if r['bid'] and r['ask']:
                mids[r['stock']][int(r['tick'])] = (int(r['bid']) + int(r['ask'])) / 2
    dividends = defaultdict(lambda: [0.0] * (n + 1))
    with (folder / 'bars.csv').open(newline='', encoding='utf-8') as f:
        for r in csv.DictReader(f):
            t = int(r['tick'])
            if t < n:
                dividends[r['stock']][t + 1] = float(r['dividend_cents_per_payment'])
    for values in dividends.values():
        for t in range(n):
            values[t + 1] += values[t]
    active = defaultdict(set)
    totals = defaultdict(Counter)
    sampled = {}
    fees = 5  # Asserted against the frozen config by main().
    with (folder / 'trades.csv').open(newline='', encoding='utf-8') as f:
        reader = csv.reader(f)
        next(reader)
        for tick, stock, price, qty, aggressor, taker, maker in reader:
            t, p, q = int(tick), int(price), int(qty)
            for account, other, sign in ((taker, maker, 1 if aggressor == 'buy' else -1),
                                         (maker, taker, -1 if aggressor == 'buy' else 1)):
                group = types[account]
                if group not in selected:
                    continue
                if t >= n // 2:
                    active[group].add(account)
                if types[other] == group:
                    continue
                totals[group]['external_notional'] += p * q
                key = (account, stock)
                bucket = t // 100
                if sampled.get(key) == bucket:
                    continue
                sampled[key] = bucket
                for h in (20, 100):
                    future = ((t + h + 9) // 10) * 10
                    mid = mids[stock].get(future)
                    if mid is None or future >= n:
                        totals[group][f'missing_{h}'] += 1
                        continue
                    ordinary = dividends[stock][future + 1] - dividends[stock][t + 1]
                    gain = sign * (mid - p + ordinary) * q - (p * q * fees + 9999) // 10000
                    totals[group][f'gain_{h}'] += gain
                    totals[group][f'notional_{h}'] += p * q
                    totals[group][f'samples_{h}'] += 1
    result = {}
    for group in sorted(selected):
        aa = [a for a in accounts.values() if a['strategy'] == group]
        volume = sum(a['stats']['volume_notional'] for a in aa)
        data = dict(totals[group])
        data['late_activity_share'] = len(active[group]) / len(aa)
        data['external_share_of_own_notional'] = data.get('external_notional', 0) / volume if volume else 0
        for h in (20, 100):
            notional = data.get(f'notional_{h}', 0)
            data[f'markout_{h}'] = data.get(f'gain_{h}', 0) / notional if notional else None
        result[group] = data
    return result


def verify_plan(summary, cfg):
    named = {g['owner'] for g in cfg['bots'] if 'owner' in g}
    plan = cfg['capital_redistribution']
    anonymous = [a for a in summary['accounts'] if a['owner'] not in named]
    donors = sum(a['strategy'] == plan['from_bot'] for a in anonymous)
    weight_sum = sum(plan['recipients'].values())
    portions = {}
    cumulative = 0
    for strategy, weight in sorted(plan['recipients'].items()):
        portions[strategy] = [donors * amount * (cumulative + weight) // weight_sum
                              - donors * amount * cumulative // weight_sum
                              for amount in (plan['cash_each'], plan['shares_each'])]
        cumulative += weight
    for strategy in {a['strategy'] for a in anonymous}:
        aa = [a for a in anonymous if a['strategy'] == strategy]
        cash, stock = portions.get(strategy, [0, 0])
        for index, a in enumerate(aa):
            dc = cash * (index + 1) // len(aa) - cash * index // len(aa)
            dq = stock * (index + 1) // len(aa) - stock * index // len(aa)
            assert a['initial_cash'] == 2_000_000 + dc
            assert all(q == 20 + dq for q in a['initial_shares'])


def compare(before, after):
    assert opening(before) == opening(after)
    assert before['terminal_payments'] == after['terminal_payments']
    assert before['final_dividends'] == after['final_dividends']
    assert len(before['accounts']) == len(after['accounts'])
    for a, b in zip(before['accounts'], after['accounts']):
        assert b['strategy'] == REPLACED.get(a['strategy'], a['strategy'])
        expected_name = a['account'].replace(a['strategy'] + '#', b['strategy'] + '#', 1)
        assert b['account'] == expected_name
        assert b['owner'] == expected_name
        for key in ('news_multiplier', 'forecast_access', 'forecasts_received', 'news_received_by_stage'):
            assert a[key] == b[key], (key, a['account'])
        if '#' not in a['account']:
            assert a['initial_cash'] == b['initial_cash']
            assert a['initial_shares'] == b['initial_shares']


def aggregate(rows):
    result = {}
    for arm in sorted({r['arm'] for r in rows}):
        rr = [r for r in rows if r['arm'] == arm]
        avg = lambda key: statistics.mean(r[key] for r in rr)
        value = {
            'runs': len(rr), 'quality_pass': sum(r['quality']['pass'] for r in rr),
            'quality_failures': dict(Counter(c['name'] for r in rr for c in r['quality']['checks'] if not c['pass'])),
            'quality_means': {c['name']: statistics.mean(next(x['value'] for x in r['quality']['checks']
                                if x['name'] == c['name']) for r in rr) for c in rr[0]['quality']['checks']},
            'groups': {g: {k: statistics.mean(r['groups'][g][k] for r in rr)
                           for k in rr[0]['groups'][g]} for g in rr[0]['groups']},
            'ledger': {g: {k: statistics.mean(r['ledger'][g][k] for r in rr)
                           for k in rr[0]['ledger'][g]} for g in rr[0]['ledger']},
            'activity': {g: statistics.mean(r['trading'][g]['late_activity_share'] for r in rr)
                         for g in rr[0]['trading']},
            'external_notional_share': {g: statistics.mean(r['trading'][g]['external_share_of_own_notional']
                                          for r in rr) for g in rr[0]['trading']},
            'markouts': {g: {str(h): statistics.mean(r['trading'][g][f'markout_{h}'] for r in rr
                               if r['trading'][g][f'markout_{h}'] is not None) for h in (20, 100)}
                         for g in rr[0]['trading']},
            'max_loss_below_60_share': statistics.mean(r['max_group_loss_share'] <= 0.6 for r in rr),
            'max_capital_over_runs': max(r['max_group_capital_share'] for r in rr),
            'audits': sum(r['audits'] for r in rr),
            'cli_seconds_total': sum(r['cli_seconds'] for r in rr),
            'simulation_seconds_total': sum(r['simulation_seconds'] for r in rr),
            'new_safety_violations': sum(r['new_safety_violations'] for r in rr),
        }
        for key in ('max_group_loss_share', 'market_net_return', 'gross_loss_per_initial',
                    'liquidated_accounts', 'insolvent_accounts', 'bad_debt_per_initial',
                    'mean_thin_side_depth_cents', 'payer_loss_share', 'core_loss_share'):
            value[key] = avg(key)
        result[arm] = value
    base = result['v18']
    for arm, v in result.items():
        if arm == 'v18':
            continue
        v['acceptance'] = {
            'five_payers_loss_at_least_60': v['payer_loss_share'] >= 0.60,
            'core_three_loss_at_least_35': v['core_loss_share'] >= 0.35,
            'grid_loss_at_most_25': v['groups']['grid']['gross_loss_share'] <= 0.25,
            'no_group_mean_loss_above_40': max(g['gross_loss_share'] for g in v['groups'].values()) <= 0.40,
            '80pct_runs_no_group_loss_above_60': v['max_loss_below_60_share'] >= 0.8,
            'new_groups_mean_negative': all(v['groups'][g]['net_return'] < 0 for g in NEW),
            'new_groups_late_activity_at_least_80': all(v['activity'][g] >= 0.8 for g in NEW),
            'new_combined_trading_inventory_residual_negative': v['ledger']['new_combined']['trading_inventory_terminal_residual'] < 0,
            'quality_not_lower': v['quality_pass'] >= base['quality_pass'],
            'two_sided_drop_at_most_2pp': v['quality_means']['two_sided_share'] >= base['quality_means']['two_sided_share'] - 0.02,
            'depth_at_least_70pct': v['mean_thin_side_depth_cents'] >= 0.7 * base['mean_thin_side_depth_cents'],
            'spread_at_most_150pct': v['quality_means']['median_spread'] <= 1.5 * base['quality_means']['median_spread'],
            'capital_concentration_at_most_35': v['max_capital_over_runs'] <= 0.35,
            'new_cash_only_safety': v['new_safety_violations'] == 0,
        }
        v['accepted'] = all(v['acceptance'].values())
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('stage', choices=['dev', 'holdout'])
    parser.add_argument('--round', default='r1')
    args = parser.parse_args()
    assert args.round.isalnum()
    from build_release import windows_build_budget
    budget = windows_build_budget() if os.name == 'nt' else os.cpu_count()
    # Registry wiring and NEW modules are allowed; old strategy implementation is frozen.
    protected = subprocess.check_output(['git', 'ls-tree', '-r', '--name-only', '7263a95', '--', 'crates/bots/src', 'crates/market', 'crates/bot-api'],
                                        cwd=ROOT, text=True).splitlines()
    protected = [p for p in protected if p != 'crates/bots/src/lib.rs']
    subprocess.run(['git', 'diff', '--quiet', '7263a95', '--', *protected], cwd=ROOT, check=True)
    binary = ROOT / 'target/release' / ('llm-market.exe' if os.name == 'nt' else 'llm-market')
    freeze = ROOT / 'runs' / f'payer-v19-{args.round}-selection.json'
    if args.stage == 'dev' and freeze.exists():
        raise FileExistsError(freeze)
    selection = json.loads(freeze.read_text()) if args.stage == 'holdout' else None
    arms = ['v18', selection['candidate']] if selection else ['v18', 'balanced', 'social']
    seeds = list(range(87001, 87021)) if selection else list(range(86001, 86011))
    paths = {a: ROOT / 'scenarios' / ('baseline-v18.toml' if a == 'v18' else f'payer-v19-{a}.toml') for a in arms}
    configs = {a: tomllib.loads(p.read_text(encoding='utf-8')) for a, p in paths.items()}
    for a, cfg in configs.items():
        assert cfg['fees']['trade_fee_bps'] == 5
        assert cfg['world']['dividend_period'] == 1
        assert len(cfg['bots']) == len(configs['v18']['bots'])
        assert {k:v for k,v in cfg.items() if k not in ('bots', 'capital_redistribution')} == {
            k:v for k,v in configs['v18'].items() if k not in ('bots', 'capital_redistribution')}
        if a != 'v18':
            for old, new in zip(configs['v18']['bots'], cfg['bots']):
                expected = dict(old, bot=REPLACED.get(old['bot'], old['bot']))
                assert expected == new
    manifest = {'stage': args.stage, 'round': args.round, 'seeds': seeds, 'repeats': 2,
                'threads': 4, 'cpu_budget': budget, 'binary_sha256': sha(binary),
                'source_hashes': {p: sha(ROOT / p) for p in SOURCES},
                'protected_hashes': {p: sha(ROOT / p) for p in protected},
                'scenario_hashes': {a: sha(p) for a,p in paths.items()}}
    if selection:
        for k in ('binary_sha256', 'source_hashes', 'protected_hashes'):
            assert manifest[k] == selection[k], k
        assert manifest['scenario_hashes'][selection['candidate']] == selection['scenario_sha256']
    out = ROOT / 'runs' / f'payer-v19-{args.round}-{args.stage}'
    out.mkdir(parents=True, exist_ok=False)
    save(out / 'manifest.json', manifest)
    rows = []
    for i, seed in enumerate(seeds):
        for repeat in range(2):
            rotation = (i + repeat) % len(arms)
            pair = {}
            for arm in arms[rotation:] + arms[:rotation]:
                dest = out / f'{arm}-{seed}-r{repeat}'
                command = [str(binary), 'run', str(paths[arm]), '--out', str(dest), '--seed', str(seed), '--threads', '4']
                started = time.perf_counter()
                with dest.with_suffix('.log').open('w', encoding='utf-8') as log:
                    subprocess.run(command, cwd=ROOT, stdout=log, stderr=subprocess.STDOUT, check=True)
                elapsed = time.perf_counter() - started
                summary = json.loads((dest / 'summary.json').read_text(encoding='utf-8'))
                verify_plan(summary, configs[arm])
                # Canonical economic family; preserve original IDs and raw reports.
                canonical = json.loads(json.dumps(summary))
                for a in canonical['accounts']:
                    if a['strategy'] == 'grid_v2': a['strategy'] = 'grid'
                row = summarize(canonical, dest)
                assert row['audits'] == 20 and summary['threads'] == 4
                assert row['errors'] == row['skipped'] == row['disabled'] == 0
                row['ledger'] = {g: ledger([a for a in canonical['accounts'] if a['strategy'] == g]) for g in row['groups']}
                new = [a for a in summary['accounts'] if a['strategy'] in NEW]
                risk_accounts = [a for a in summary['accounts'] if a['strategy'] in (*NEW, 'grid_v2')]
                if new:
                    row['ledger']['new_combined'] = ledger(new)
                row['new_safety_violations'] = sum(a['cash'] < 0 or a['equity'] < 0 or a['stats']['interest'] > 0
                    or a['stats']['liquidations'] > 0 or a['stats']['max_leverage'] > 1.000001 for a in risk_accounts)
                row['trading'] = trading(canonical, dest)
                row['payer_loss_share'] = sum(row['groups'].get(g, {}).get('gross_loss_share', 0) for g in PAYERS)
                row['core_loss_share'] = sum(row['groups'].get(g, {}).get('gross_loss_share', 0) for g in ('profit_effect', *NEW))
                row.update(arm=arm, seed=seed, repeat=repeat, cli_seconds=elapsed,
                           summary_sha256=sha(dest / 'summary.json'), news_sha256=sha(dest / 'news.csv'), command=command)
                rows.append(row)
                pair[arm] = (summary, row['news_sha256'])
                save(out / 'progress.json', rows)
                print(f"{len(rows)}/{len(seeds)*2*len(arms)} {arm} seed={seed} r={repeat} "
                      f"quality={row['quality']['pass']} payers={row['payer_loss_share']:.3f} "
                      f"core={row['core_loss_share']:.3f}", flush=True)
            for arm in arms[1:]:
                compare(pair['v18'][0], pair[arm][0])
                assert pair['v18'][1] == pair[arm][1]
    assert sha(binary) == manifest['binary_sha256']
    for p, h in {**manifest['source_hashes'], **manifest['protected_hashes']}.items():
        assert sha(ROOT / p) == h
    assert all(sha(paths[a]) == manifest['scenario_hashes'][a] for a in arms)
    result = {'manifest': manifest, 'aggregate': aggregate(rows), 'rows': rows}
    save(ROOT / 'docs' / f'payer-v19-{args.round}-{args.stage}-results.json', result)
    if not selection:
        passed = [a for a in arms[1:] if result['aggregate'][a]['accepted']]
        if passed:
            chosen = min(passed, key=lambda a: (-result['aggregate'][a]['quality_pass'],
                         -sum(result['aggregate'][a]['activity'][g] for g in NEW), a != 'balanced'))
            save(freeze, {'candidate': chosen, 'scenario_sha256': sha(paths[chosen]),
                         **{k: manifest[k] for k in ('binary_sha256', 'source_hashes', 'protected_hashes')}})
            print('FROZEN SELECTION:', chosen, flush=True)
        else:
            print('NO CANDIDATE PASSED; HOLDOUT UNUSED.', flush=True)
    for arm, value in result['aggregate'].items():
        print(arm, json.dumps({k: v for k,v in value.items() if k in
              ('quality_pass', 'payer_loss_share', 'core_loss_share', 'acceptance', 'accepted')}, indent=2), flush=True)


if __name__ == '__main__':
    main()
