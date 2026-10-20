"""World-equal paired results, with activity split into leader and other members."""
from collections import Counter, defaultdict
import json
import os
from pathlib import Path
import statistics as st

from build_release import windows_build_budget
from research_profit_leaders import ROOT, OUT, ARMS, SEEDS, verify_pair
from research_v7 import read, write, sha


def members(row, subset='all'):
    accounts = [a for a in row['summary']['accounts'] if a['account'].startswith('effect#')]
    if subset == 'all': return accounts
    return [a for a in accounts if (int(a['account'].split('#')[1]) % 3 == 0) == (subset == 'leaders')]


def group_return(row, subset='all'):
    return st.mean(a['return_ratio'] - 1 for a in members(row, subset))


def activity(row):
    names = {a['account'] for a in members(row)}
    last = {name: -1 for name in names}
    bins = {name: [0] * 10 for name in names}
    for line in (Path(row['directory']) / 'trades.csv').open(encoding='utf-8'):
        if 'effect#' not in line: continue
        tick, _, _, _, _, taker, maker = line.strip().split(',')
        tick = int(tick); slot = min(9, tick // 1000)
        for name in [taker, maker]:
            if name in names:
                last[name] = max(last[name], tick); bins[name][slot] += 1
    result = {}
    for subset in ['all', 'leaders', 'others']:
        accounts = members(row, subset)
        result[subset] = dict(
            no_trade_last_5000=st.mean(last[a['account']] < 5000 for a in accounts),
            final_flat=st.mean(not any(a['shares']) for a in accounts),
            fills_by_1000=[st.mean(bins[a['account']][i] for a in accounts) for i in range(10)],
            active_by_1000=[st.mean(bins[a['account']][i] > 0 for a in accounts) for i in range(10)])
    return result


def trace():
    manifest = read(OUT / 'trace/trace-manifest.json')
    results = {}
    for entry in manifest['starts']:
        start = entry['start']; idx = start['account_index']; counts = Counter(); examples = []
        for line in (OUT / 'trace/trace' / f'account-{idx}.jsonl').open(encoding='utf-8'):
            row = json.loads(line)
            if not row['decided']: continue
            assert row['profit_effect_verified'] is True
            counts['verified_decisions'] += 1
            for action in row['actions']:
                order = action.get('Limit', action.get('Market'))
                if not order or order['side'] != 'Buy': continue
                s = order['stock']; sig = row['profit_effect'][s]
                counts['buy_requests'] += 1
                counts['buy_with_positive_peer'] += sig['peer_profit'] > 0
                external = sig['leader_contribution'] > 0 and sig['peer_profit'] <= 0 and sig['own_profit'] <= 0
                counts['leader_buy_without_positive_group_or_own_experience'] += external
                if external and len(examples) < 3:
                    examples.append(dict(tick=row['tick'], stock=start['rules']['stocks'][s]['name'],
                        shares=row['pre']['shares'][s], order=order, signal=sig))
        results[start['account']] = dict(leader_weight=start['params'].get('leader_weight', 0),
            counters=dict(counts), first_external_examples=examples)
    assert all(v['profit_effect_mismatches'] == 0 for v in manifest['verification'])
    return results


def main():
    if os.name == 'nt': windows_build_budget()
    rows = read(OUT / 'batch.json')
    assert len(rows) == len(SEEDS) * len(ARMS)
    manifest = read(OUT / 'frozen/manifest.json')
    for path, h in manifest['source'].items(): assert sha(ROOT / path) == h, path
    for name, h in manifest['binaries'].items(): assert sha(OUT / 'frozen' / name) == h
    for name, h in manifest['configs'].items(): assert sha(OUT / 'configs' / name) == h
    for seed in SEEDS:
        pair = [r for r in rows if r['seed'] == seed]
        assert len(pair) == 4; verify_pair(pair)
    activities = []
    for r in rows:
        activities.append(dict(seed=r['seed'], arm=r['arm'], **activity(r)))
        if len(activities) % 8 == 0: print('Activity audited', len(activities), '/', len(rows), flush=True)
    write(OUT / 'activity.json', activities)
    report = dict(worlds=len(SEEDS), seeds=SEEDS, runs=len(rows), ticks=10000, threads=4,
        cpu_budget=10, repeats_per_world_arm=1,
        members_range=[min(len(members(r)) for r in rows), max(len(members(r)) for r in rows)],
        leader_member_fraction=st.mean(len(members(r, 'leaders')) / len(members(r)) for r in rows),
        initial_equity_share=st.mean(sum(a['initial_equity'] for a in members(r)) /
            sum(a['initial_equity'] for a in r['summary']['accounts']) for r in rows),
        audits=sum(r['summary']['audits'] for r in rows),
        skips=sum(a['stats']['skipped'] for r in rows for a in r['summary']['accounts']),
        errors=sum(a['stats']['bot_errors'] for r in rows for a in r['summary']['accounts']),
        disabled=sum(bool(a['disabled']) for r in rows for a in r['summary']['accounts']), arms={}, paired={})
    for arm in ARMS:
        chosen = [r for r in rows if r['arm'] == arm]
        active = [r for r in activities if r['arm'] == arm]
        quality = defaultdict(list)
        for r in chosen:
            for c in r['summary']['quality']['checks']: quality[c['name']].append(c)
        subsets = {}
        for subset in ['all', 'leaders', 'others']:
            subsets[subset] = dict(
                mean_return=st.mean(group_return(r, subset) for r in chosen),
                positive_worlds=sum(group_return(r, subset) > 0 for r in chosen),
                mean_trades=st.mean(st.mean(a['stats']['trades'] for a in members(r, subset)) for r in chosen),
                decision_us=st.mean(st.mean(a['stats']['decision_nanos_total'] / max(1, a['stats']['decisions']) / 1000
                    for a in members(r, subset)) for r in chosen),
                no_trade_last_5000=st.mean(r[subset]['no_trade_last_5000'] for r in active),
                final_flat=st.mean(r[subset]['final_flat'] for r in active),
                fills_by_1000=[st.mean(r[subset]['fills_by_1000'][i] for r in active) for i in range(10)],
                active_by_1000=[st.mean(r[subset]['active_by_1000'][i] for r in active) for i in range(10)])
        report['arms'][arm] = dict(subsets=subsets,
            market_trades=st.mean(r['summary']['trades'] for r in chosen),
            quality_pass=sum(r['summary']['quality']['pass'] for r in chosen),
            quality={n: dict(failures=sum(not c['pass'] for c in checks), median=st.median(c['value'] for c in checks))
                for n, checks in quality.items()},
            wall=dict(median=st.median(r['wall'] for r in chosen), min=min(r['wall'] for r in chosen),
                max=max(r['wall'] for r in chosen)),
            liquidations=sum(a['stats']['liquidations'] for r in chosen for a in members(r)))
    for control in ['group', 'full']:
        effect = control + '_leaders'
        deltas = []
        for seed in SEEDS:
            chosen = {r['arm']: r for r in rows if r['seed'] == seed}
            deltas.append(group_return(chosen[effect]) - group_return(chosen[control]))
        report['paired'][effect + '_minus_' + control] = dict(mean_return_delta=st.mean(deltas),
            positive_worlds=sum(d > 0 for d in deltas))
    report['compatibility'] = read(OUT / 'compatibility.json')
    report['trace'] = trace()
    write(ROOT / 'docs/profit-effect-leaders-results.json', report)
    for arm, r in report['arms'].items():
        print(arm, json.dumps({k: v for k, v in r.items() if k != 'quality'}, ensure_ascii=False), flush=True)


if __name__ == '__main__': main()
