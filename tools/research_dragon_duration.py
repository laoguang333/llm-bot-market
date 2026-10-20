"""Serial v15 duration and selected-growth ablation; no bot/engine changes.

Hidden seed reconstruction here is OFFLINE analysis only, never a strategy input.
Run through arena_gate.py. All worlds/lengths are fixed before the first match.
"""
import argparse
from collections import defaultdict
import csv
import json
import math
import os
from pathlib import Path
import shutil
import statistics as st
import time
import tomllib
import zipfile

from build_release import windows_build_budget
from research_v7 import run_one, sha, write

ROOT = Path(__file__).resolve().parents[1]
LENGTHS = [10000, 20000, 40000]
MASK = (1 << 64) - 1


def mix(x):
    x ^= x >> 30
    x = x * 0xbf58476d1ce4e5b9 & MASK
    x ^= x >> 27
    x = x * 0x94d049bb133111eb & MASK
    return x ^ (x >> 31)


def unit(seed, a, b):
    return (mix(seed ^ mix(((a * 0x9e3779b97f4a7c15) & MASK) ^ mix(b))) >> 11) / (1 << 53)


def opening_prices(cfg, seed):
    t = cfg['ticks']
    duration = cfg['scoring']['terminal_payment_ticks']
    lo, hi = [math.floor(x * t / duration + .5) for x in cfg['scoring']['terminal_ratio_range']]
    period = cfg['world']['dividend_period']
    payments = (t - 1) // period + (lo + hi) / 2 * (duration // period)
    tick_size = cfg['market']['tick_size']
    return [max(tick_size, math.floor(s['dividend'] / cfg['world']['dividend_scale'] * payments *
            (1 + cfg['market']['opening_spread'] * (2 * unit(seed ^ 0x09e7, i, 1) - 1)) / tick_size + .5) * tick_size)
            for i, s in enumerate(cfg['stocks'])]


def traces(folder, cfg):
    names = {s['name']: i for i, s in enumerate(cfg['stocks'])}
    result = [dict(rates=[], closes=[], highs=[], volume=0, notional=0, late_volume=0, late_notional=0,
                   two_sided=0, book_samples=0, spreads=[]) for _ in names]
    cutoff = math.floor(cfg['ticks'] * cfg['world']['dragon']['start_fraction'])
    with (folder / 'bars.csv').open(encoding='utf-8', newline='') as f:
        for b in csv.DictReader(f):
            s = result[names[b['stock']]]
            s['rates'].append(int(b['dividend']))
            s['closes'].append(int(b['close']) if b['close'] else None)
            s['highs'].append(int(b['high']) if b['high'] else None)
            s['volume'] += int(b['volume'])
            s['notional'] += int(b['notional'])
            if int(b['tick']) >= cutoff:
                s['late_volume'] += int(b['volume'])
                s['late_notional'] += int(b['notional'])
    with (folder / 'books.csv').open(encoding='utf-8', newline='') as f:
        for b in csv.DictReader(f):
            s = result[names[b['stock']]]
            s['book_samples'] += 1
            if b['bid'] and b['ask']:
                s['two_sided'] += 1
                bid, ask = int(b['bid']), int(b['ask'])
                s['spreads'].append((ask - bid) / ((ask + bid) / 2))
    assert all(len(s['rates']) == cfg['ticks'] for s in result)
    return result


def metrics(folder, cfg, selected, ts):
    raw = json.loads((folder / 'summary.json').read_text(encoding='utf-8'))
    t = cfg['ticks']
    assert raw['ticks'] == t and raw['threads'] == 4 and raw['audits'] == t // 500
    accounts = raw['accounts']
    assert all(not a['disabled'] and a['stats']['bot_errors'] == 0 and a['stats']['skipped'] == 0 for a in accounts)
    opening = opening_prices(cfg, raw['seed'])
    assert all(a['initial_equity'] == a['initial_cash'] + sum(q * p for q, p in zip(a['initial_shares'], opening)) for a in accounts)
    assert [s['rates'][-1] for s in ts] == raw['final_dividends']
    equity0 = sum(a['initial_equity'] for a in accounts)
    total_volume = sum(s['volume'] for s in ts)
    total_notional = sum(s['notional'] for s in ts)
    late_volume = sum(s['late_volume'] for s in ts)
    late_notional = sum(s['late_notional'] for s in ts)
    stock_metrics = []
    end_ticks = raw['terminal_payments'] * raw['terminal_payment_ticks']
    for i, s in enumerate(ts):
        initial_rate = cfg['stocks'][i]['dividend']
        p0 = opening[i]
        high = max(p for p in s['highs'] if p is not None)
        high_tick = s['highs'].index(high)
        dividend_peak = max(s['rates'])
        rate_peak_tick = s['rates'].index(dividend_peak)
        rate_peak = s['rates'][high_tick]
        duration_factor = (t - 1 - high_tick + end_ticks) / (1.5 * t - 1)
        # Same flat-current-dividend yardstick used by the engine's reference value.
        flat_at_high = rate_peak / raw['dividend_scale'] * (t - 1 - high_tick + end_ticks)
        cumulative = sum(s['rates'][1:high_tick + 1]) / raw['dividend_scale']
        final_cashflows = (sum(s['rates'][1:]) + s['rates'][-1] * end_ticks) / raw['dividend_scale']
        last = next(p for p in reversed(s['closes']) if p is not None)
        positive = [p for p in s['closes'] if p is not None]
        running_high, dd = positive[0], 0
        for price in positive:
            dd = max(dd, 1 - price / running_high)
            running_high = max(running_high, price)
        stock_metrics.append(dict(name=cfg['stocks'][i]['name'], opening=p0,
            peak_multiple=high / p0, price_peak_fraction=high_tick / t,
            dividend_peak_multiple=dividend_peak / initial_rate, dividend_peak_fraction=rate_peak_tick / t,
            dividend_at_price_peak_multiple=rate_peak / initial_rate, duration_factor=duration_factor,
            flat_value_at_price_peak_multiple=flat_at_high / p0, price_over_flat_at_peak=high / max(flat_at_high, 1),
            cash_paid_at_price_peak_multiple=cumulative / p0,
            price_plus_paid_dividends_at_price_peak_multiple=(high + cumulative) / p0,
            gross_hold_to_end_wealth_multiple=final_cashflows / p0,
            final_price_multiple=last / p0, close_drawdown=dd,
            two_sided=s['two_sided'] / max(s['book_samples'], 1),
            spread=st.median(s['spreads']) if s['spreads'] else None,
            volume_share=s['volume'] / max(total_volume, 1), notional_share=s['notional'] / max(total_notional, 1),
            late_volume_share=s['late_volume'] / max(late_volume, 1), late_notional_share=s['late_notional'] / max(late_notional, 1),
            notional_per_tick=s['notional'] / t))
    other = [s for i, s in enumerate(stock_metrics) if i != selected]
    groups = defaultdict(list)
    for a in accounts:
        groups[a['strategy']].append(a)
    strategies = {g: dict(return_pct=st.mean((a['return_ratio'] - 1) * 100 for a in aa),
                          percentile=st.mean(a['percentile'] for a in aa)) for g, aa in groups.items()}
    return dict(seed=raw['seed'], ticks=t, selected=selected, accounts=len(accounts), initial_market_equity=equity0,
        terminal_fraction=end_ticks / t, seconds=raw['wall_seconds'], audits=raw['audits'], quality=raw['quality'],
        target=stock_metrics[selected], other_mean={k:st.mean(s[k] for s in other if s[k] is not None) for k in
            ['peak_multiple', 'final_price_multiple', 'two_sided', 'spread', 'close_drawdown', 'notional_per_tick']},
        market_return_pct=(sum(a['equity'] for a in accounts) / equity0 - 1) * 100,
        median_account_return_pct=st.median((a['return_ratio'] - 1) * 100 for a in accounts),
        losing_account_share=st.mean(a['return_ratio'] < 1 for a in accounts),
        dividend_injection_pct=sum(a['stats']['dividends'] for a in accounts) / equity0 * 100,
        cost_pct=sum(a['stats'][k] for a in accounts for k in ['idle_fees','trade_fees','interest']) / equity0 * 100,
        trades_per_tick=raw['trades']/t, notional_per_tick=total_notional/t,
        liquidations=sum(a['stats']['liquidations'] for a in accounts),
        bad_debt=sum(a['stats']['bad_debt'] for a in accounts),
        source_share_burns=sum(raw['shares_burned']), strategies=strategies, stocks=stock_metrics)


def precheck():
    # Verify the offline opening-price reconstruction against recorded Start prices.
    folder = ROOT / 'runs/osprey-v2-holdout'
    cfg = tomllib.loads((folder / 'early_old.toml').read_text(encoding='utf-8'))
    for seed in range(72001, 72021):
        rec = json.loads((folder / f'early_old-{seed}/stock-record.json').read_text(encoding='utf-8'))
        assert opening_prices(cfg, seed) == rec['opening_prices']


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('name')
    p.add_argument('--first-seed', type=int, default=81001)
    p.add_argument('--seeds', type=int, default=12)
    args = p.parse_args()
    budget = windows_build_budget() if os.name == 'nt' else os.cpu_count()
    precheck()
    out = ROOT / 'runs' / args.name
    out.mkdir(parents=True, exist_ok=False)
    (out / 'configs').mkdir()
    baseline = (ROOT / 'scenarios/baseline-v15.toml').read_text(encoding='utf-8')
    cfg0 = tomllib.loads(baseline)
    configs = {}
    for length in LENGTHS:
        for on in [False, True]:
            arm = f'{length}-{"on" if on else "off"}'
            text = baseline.replace('ticks = 10000\n', f'ticks = {length}\n', 1).replace('probability = 0.5\n', f'probability = {1.0 if on else 0.0}\n', 1)
            cfg = tomllib.loads(text)
            assert cfg['bots'] == cfg0['bots']
            configs[arm] = cfg
            (out / f'configs/{arm}.toml').write_text(text, encoding='utf-8')
    seeds = list(range(args.first_seed, args.first_seed + args.seeds))
    plan = dict(seeds=seeds, lengths=LENGTHS, arms=list(configs), repeats=1, threads=4, cpu_budget=budget,
        treatment='Probability forced to 1 vs 0, with candidate growth and all other settings retained. Conditional effect of selected continuation; not removal of all growth.',
        duration='Only ticks changes between lengths. Opening prices, stage positions and terminal durations scale through existing rules; per-tick events/fees and bot lookbacks stay unchanged. Longer worlds are not identical paths stretched in time.',
        review='No new bot or engine code. Existing reviewed v15 roster and release binary. Confirmed bots do not read Rules.dragon; labels and seed reconstruction only in offline analysis.',
        metrics='Price peaks / actual opening reference price; dividend peaks; fixed-current-dividend remaining-cashflow yardstick; on/off market-wide liquidity, volume, returns and liquidation. All worlds retained; no tuning.',
        primary='Compare median selected-stock price peak multiples and >=5x/>=10x counts across lengths; compare paired on-off effects for each length separately.')
    write(out / 'plan.json', plan)
    shutil.copy2(ROOT / 'target/release/llm-market.exe', out / 'market.exe')
    sources = list((ROOT / 'crates').rglob('*.rs')) + [Path(__file__).resolve(), ROOT/'Cargo.lock', ROOT/'Cargo.toml']
    manifest = dict(source={str(f.relative_to(ROOT)):sha(f) for f in sources},
                    binary_sha256=sha(out/'market.exe'), configs={arm:sha(out/f'configs/{arm}.toml') for arm in configs},
                    plan_sha256=sha(out/'plan.json'))
    write(out/'manifest.json', manifest)
    with zipfile.ZipFile(out/'sources.zip', 'x') as z:
        for f in sources:
            z.write(f, f.relative_to(ROOT))
    started = time.perf_counter()
    rows = []
    for seed in seeds:
        for length in (LENGTHS if seed % 2 else LENGTHS[::-1]):
            summaries, elapsed = {}, {}
            for on in ([True, False] if seed % 2 else [False, True]):
                arm = f'{length}-{"on" if on else "off"}'
                folder = out/f'{arm}-{seed}'
                summaries[on], elapsed[on] = run_one(out/'market.exe', out/f'configs/{arm}.toml', folder, seed, 4)
            selected = summaries[True]['dragon']['selected_stock']
            assert selected is not None and summaries[False]['dragon']['selected_stock'] is None
            assert summaries[True]['dragon']['candidates'] == summaries[False]['dragon']['candidates']
            assert summaries[True]['terminal_payments'] == summaries[False]['terminal_payments']
            initial = lambda s: {a['account']:(a['initial_cash'],a['initial_shares'],a['initial_equity']) for a in s['accounts']}
            assert initial(summaries[True]) == initial(summaries[False])
            paths = {on:traces(out/f'{length}-{"on" if on else "off"}-{seed}', configs[f'{length}-{"on" if on else "off"}']) for on in [True, False]}
            cutoff = summaries[True]['dragon']['divergence_tick']
            for stock in range(len(cfg0['stocks'])):
                a, b = paths[True][stock]['rates'], paths[False][stock]['rates']
                assert a == b if stock != selected else a[:cutoff] == b[:cutoff]
            row = dict(seed=seed, ticks=length)
            for on in [True, False]:
                label = 'on' if on else 'off'
                row[label] = metrics(out/f'{length}-{label}-{seed}', configs[f'{length}-{label}'], selected, paths[on])
                row[label]['process_seconds'] = elapsed[on]
            rows.append(row)
            write(out/'results.json', dict(plan=plan, manifest=manifest, rows=rows, elapsed_seconds=time.perf_counter()-started))
            print(f'{seed} ticks={length} target={row["on"]["target"]["name"]} peak on/off={row["on"]["target"]["peak_multiple"]:.2f}/{row["off"]["target"]["peak_multiple"]:.2f} market return={row["on"]["market_return_pct"]:.1f}%/{row["off"]["market_return_pct"]:.1f}%', flush=True)
    assert all(sha(ROOT/path)==digest for path,digest in manifest['source'].items())
    assert sha(out/'market.exe') == manifest['binary_sha256']
    assert sha(out/'plan.json') == manifest['plan_sha256']
    print('Completed all paired worlds with frozen sources and unchanged baseline.', flush=True)


if __name__ == '__main__':
    main()
