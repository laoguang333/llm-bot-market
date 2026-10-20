"""Audit the fixed v11 incumbent and Horizon v3 experiments; no strategy tuning."""
import hashlib
import json
import os
import statistics as st
import tomllib
from pathlib import Path
from build_release import windows_build_budget

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / 'runs/horizon-v3-research'


def read(path):
    return json.loads(path.read_text(encoding='utf-8'))


def account(row, name='researcher'):
    return next(a for a in row['summary']['accounts'] if a['account'] == name)


def describe(rows, name='researcher'):
    accounts = [account(r, name) for r in rows]
    returns = [a['return_ratio'] - 1 for a in accounts]
    result = dict(runs=len(rows), mean_return=st.mean(returns),
                  median_return=st.median(returns), min_return=min(returns),
                  max_return=max(returns), positive=sum(v > 0 for v in returns),
                  worst_decile_mean=st.mean(sorted(returns)[:max(1, len(rows)//10)]),
                  mean_percentile=st.mean(a['percentile'] for a in accounts),
                  top_quartile=sum(a['percentile'] >= 75 for a in accounts),
                  negative_final_equity=sum(a['equity'] < 0 for a in accounts),
                  mean_decision_us=st.mean(a['stats']['decision_nanos_total'] / max(1, a['stats']['decisions']) / 1000 for a in accounts),
                  forecasts=st.mean(a['forecasts_received'] for a in accounts),
                  mean_trades=st.mean(a['stats']['trades'] for a in accounts),
                  mean_profit=st.mean((a['equity']-a['initial_equity'])/100 for a in accounts),
                  mean_trade_and_inventory=st.mean((a['equity']-a['initial_equity']-a['stats']['dividends']-a['stats']['combo_dividends']+a['stats']['idle_fees']+a['stats']['trade_fees']+a['stats']['interest']-a['stats']['bad_debt'])/100 for a in accounts),
                  mean_fees={k: st.mean(a['stats'][k]/100 for a in accounts) for k in ['idle_fees', 'trade_fees', 'interest', 'dividends', 'combo_dividends']},
                  liquidations=sum(a['stats']['liquidations'] for a in accounts),
                  bad_debt=sum(a['stats']['bad_debt']/100 for a in accounts))
    result['rank_gate_pass'] = result['mean_percentile'] >= 90 and result['top_quartile']/len(rows) >= .8
    for other in ['titan-value', 'horizon-incumbent']:
        result['beat_'+other] = sum(account(r, name)['return_ratio'] > account(r, other)['return_ratio'] for r in rows)
    return result


def paired(a, b):
    left = {r['seed']: account(r)['return_ratio'] for r in a}
    right = {r['seed']: account(r)['return_ratio'] for r in b}
    assert left.keys() == right.keys()
    deltas = [left[s] - right[s] for s in sorted(left)]
    return dict(worlds=len(deltas), improved=sum(v > 0 for v in deltas),
                mean_delta=st.mean(deltas), median_delta=st.median(deltas),
                min_delta=min(deltas), max_delta=max(deltas))


def main():
    if os.name == 'nt':
        windows_build_budget()
    groups = {g: read(OUT/f'{g}.json') for g in ['pilot1', 'batch', 'ablation', 'bench']}
    for group, count in [('pilot1', 12), ('batch', 160), ('ablation', 40), ('bench', 18)]:
        assert len(groups[group]) == count, f'{group} incomplete'
    manifest = read(OUT/'frozen/manifest.json')
    for path, expected in manifest['source'].items():
        assert hashlib.sha256((ROOT/path).read_bytes()).hexdigest() == expected, path
    for path, expected in manifest['configs'].items():
        assert hashlib.sha256((OUT/'configs'/path).read_bytes()).hexdigest() == expected, path
    for path, expected in manifest['binaries'].items():
        assert hashlib.sha256((OUT/'frozen'/path).read_bytes()).hexdigest() == expected, path
    assert (ROOT/'scenarios/baseline-v11.toml').read_bytes() == (OUT/'configs/v11.toml').read_bytes()
    assert (ROOT/'scenarios/horizon-v11.toml').read_bytes() == (OUT/'configs/v3_long.toml').read_bytes()
    old, new = [tomllib.loads((OUT/'configs'/f'{arm}.toml').read_text()) for arm in ['v10', 'v11']]
    assert new['bots'][:-1] == old['bots']
    assert new['bots'][-1] == dict(bot='horizon_v2', owner='horizon-incumbent', count=1, cash=20_000_000, shares_each=200, news_multiplier=1., forecast_access=dict(horizon=300, coverage=.5, range_width=3.))
    old.pop('bots'); new.pop('bots'); assert old == new
    assert (ROOT/'crates/bots/src/horizon_v2.rs').read_bytes() == (OUT/'crates_bots_src_horizon.rs.before').read_bytes()
    for seed in [7, 27006, 28012]:
        for repeat in range(3):
            dirs = [OUT/'bench'/f'{arm}-{seed}-{repeat}' for arm in ['v10', 'v11']]
            assert (dirs[0]/'news.csv').read_bytes() == (dirs[1]/'news.csv').read_bytes()
            a, b = [read(d/'summary.json') for d in dirs]
            assert a['final_dividends'] == b['final_dividends']
            assert a['terminal_payments'] == b['terminal_payments']
    # Matching worlds have the same external news, despite endogenous trade differences.
    for seed in range(32001, 32041):
        files = [OUT/'batch'/f'{arm}-{seed}-0/news.csv' for arm in ['v2_long', 'v3_control', 'v3_short', 'v3_long']]
        assert len({hashlib.sha256(p.read_bytes()).hexdigest() for p in files}) == 1
    rows = [r for g in groups.values() for r in g]
    risks = 0
    for row in rows:
        for a in row['summary']['accounts']:
            events = a['liquidation_events']
            assert len(events) == a['stats']['liquidations']
            assert sum(e['bad_debt'] for e in events) == a['stats']['bad_debt']
            for e in events:
                assert e['sale_notional'] == sum(f['qty']*f['price'] for f in e['fills'])
                assert e['trade_fees'] == sum(f['fee'] for f in e['fills'])
                assert e['cash_before']+e['sale_notional']-e['trade_fees']+e['bad_debt'] == e['cash_after']
                assert all(x-y == sum(f['qty'] for f in e['fills'] if f['stock'] == s) for s, (x, y) in enumerate(zip(e['shares_before'], e['shares_after'])))
                risks += 1
    result = dict(runs=len(rows), audits=sum(r['summary']['audits'] for r in rows),
                  skips=sum(a['stats']['skipped'] for r in rows for a in r['summary']['accounts']),
                  errors=sum(a['stats']['bot_errors'] for r in rows for a in r['summary']['accounts']),
                  disabled=sum(bool(a['disabled']) for r in rows for a in r['summary']['accounts']),
                  liquidation_events=risks, risk_reconciliation_errors=0,
                  frozen_source_verified=True, paired_bench_news_identical=9,
                  paired_primary_news_identical=40, arms={}, paired={}, ablation={}, performance={}, background={})
    batch = groups['batch']
    arm_rows = {arm: [r for r in batch if r['arm'] == arm] for arm in ['v2_long', 'v3_control', 'v3_short', 'v3_long']}
    for arm, chosen in arm_rows.items():
        assert len(chosen) == 40 and {r['seed'] for r in chosen} == set(range(32001, 32041))
        stats = describe(chosen)
        stats['quality_pass'] = sum(r['summary']['quality']['pass'] for r in chosen)
        names = [c['name'] for c in chosen[0]['summary']['quality']['checks']]
        stats['quality_failures'] = {name: sum(not c['pass'] for r in chosen for c in r['summary']['quality']['checks'] if c['name'] == name) for name in names}
        stats['halves'] = {name: describe([r for r in chosen if (r['seed'] <= 32020) == first]) for name, first in [('first20', True), ('second20', False)]}
        stats['dragon_strata'] = {name: describe([r for r in chosen if (r['summary']['dragon']['selected_stock'] is not None) == present]) for name, present in [('absent', False), ('present', True)]}
        result['arms'][arm] = stats
        result['background'][arm] = {name: describe(chosen, name) for name in ['titan-value', 'horizon-incumbent', 'swift-standard', 'swift-early-single']}
    for left, right in [('v3_long', 'v2_long'), ('v3_long', 'v3_control'), ('v3_long', 'v3_short'), ('v3_short', 'v3_control')]:
        result['paired'][left+'_minus_'+right] = paired(arm_rows[left], arm_rows[right])
    first20 = [r for r in arm_rows['v3_long'] if r['seed'] <= 32020]
    result['ablation']['v3_long_same20'] = describe(first20)
    for arm in ['v3_unstaged', 'v3_unguarded']:
        selected = [r for r in groups['ablation'] if r['arm'] == arm]
        assert len(selected) == 20 and {r['seed'] for r in selected} == set(range(32001, 32021))
        result['ablation'][arm] = describe(selected)
        result['ablation'][arm]['difference_from_full'] = paired(selected, first20)
    for arm in ['v10', 'v11']:
        chosen = [r for r in groups['bench'] if r['arm'] == arm]
        result['performance'][arm] = dict(runs=len(chosen), median=st.median(r['wall'] for r in chosen), min=min(r['wall'] for r in chosen), max=max(r['wall'] for r in chosen), engine_median=st.median(r['summary']['wall_seconds'] for r in chosen))
    result['pilot1_note'] = read(OUT/'plan.json')['pilot_note']
    (ROOT/'docs/horizon-v3-results.json').write_text(json.dumps(result, ensure_ascii=False, indent=2)+'\n', encoding='utf-8')
    print(json.dumps({k: result[k] for k in ['runs', 'audits', 'skips', 'errors', 'liquidation_events', 'paired', 'performance']}, ensure_ascii=False, indent=2))
    for arm, stats in result['arms'].items():
        print(arm, {k: stats[k] for k in ['mean_return', 'median_return', 'mean_percentile', 'top_quartile', 'beat_titan-value', 'beat_horizon-incumbent', 'negative_final_equity']})


if __name__ == '__main__':
    main()
