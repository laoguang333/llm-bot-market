"""Preregistered v21 research. Run stages serially through arena_gate.py."""
import argparse
import csv
import json
import os
from pathlib import Path
import re
import statistics as st
import subprocess
import time
import tomllib

from validate_v18 import sha, save, opening, summarize
from validate_dividend_risk import ledger
from collections import defaultdict

ROOT = Path(__file__).resolve().parents[1]


def prepare():
    source = (ROOT / 'scenarios/dividend-v20.toml').read_text(encoding='utf-8')
    for name, vals in {
        'moderate': (.10, .025, .25, [.10, .45], [.55, .95]),
        'mild': (.07, .02, .30, [.15, .45], [.60, .95]),
    }.items():
        text = source.replace('# Experimental v20:', '# Experimental v21 calibration:')
        for key, value in zip(('impairment_probability', 'suspension_probability', 'recovery_probability',
                               'impaired_retention', 'recovered_retention'), vals):
            text = re.sub(rf'^{key} = .*$', f'{key} = {value}', text, flags=re.M)
        (ROOT / f'scenarios/ecology-v21-{name}.toml').write_text(text, encoding='utf-8')
    for name, params in {'full': '', 'flat': 'target = 0.0', 'no-wash': 'wash_ticks = 0.0',
                         'flat-no-wash': 'target = 0.0, wash_ticks = 0.0'}.items():
        text = source.replace('owner = "tide-incumbent"', 'owner = "tide-incumbent"' +
                              (f'\nparams = {{ {params} }}' if params else ''))
        (ROOT / f'scenarios/ecology-v21-tide-{name}.toml').write_text(text, encoding='utf-8')


def calibrate(binary, out):
    results = {}
    for arm, name in [('v18', 'baseline-v18'), ('v20', 'dividend-v20'),
                      ('moderate', 'ecology-v21-moderate'), ('mild', 'ecology-v21-mild')]:
        cfg = ROOT / f'scenarios/{name}.toml'
        dest = out / f'{arm}.json'
        start = time.perf_counter()
        subprocess.run([str(binary.with_name('world-study.exe')), str(cfg), '100', '90201', str(dest)], check=True)
        stocks = [s for w in json.loads(dest.read_text()) for s in w['world']['stocks']]
        cuts = [s for s in stocks if s['early_cut_tick'] is not None]
        r = dict(stocks=len(stocks), early_cuts=len(cuts),
                 recovered=sum(s['recovered_after_early_cut'] for s in cuts),
                 persistent=sum(not s['recovered_after_early_cut'] for s in cuts),
                 long_zero=st.mean(s['longest_zero_rate_run'] >= 1000 for s in stocks),
                 low_final=st.mean(s['final_dividend_initial_multiple'] <= .25 for s in stocks),
                 final_multiple=st.mean(s['final_dividend_initial_multiple'] for s in stocks),
                 seconds=time.perf_counter()-start, scenario_sha256=sha(cfg))
        r['accepted'] = (.05 <= r['long_zero'] <= .15 and .10 <= r['low_final'] <= .30
                         and 0 < r['recovered'] <= r['persistent'] and 1 <= r['final_multiple'] <= 2)
        results[arm] = r
    results['selected'] = next((a for a in ('moderate', 'mild') if results[a]['accepted']), None)
    # A failed calibration can still have a clearly labelled experimental comparison.
    results['experimental_world'] = results['selected'] or 'mild'
    save(ROOT / 'docs/ecology-v21-calibration-results.json', results)
    source = (ROOT / f"scenarios/ecology-v21-{results['experimental_world']}.toml").read_text(encoding='utf-8')
    for arm, swaps in {'control': {}, 'harbor': {'value': 'harbor'}, 'relay': {'coral': 'relay'},
                       'both': {'value': 'harbor', 'coral': 'relay'}}.items():
        text = source
        for old, new in swaps.items():
            text = text.replace(f'bot = "{old}"', f'bot = "{new}"')
            text = re.sub(rf'\b{old} = 20\b', f'{new} = 20', text)
        (ROOT / f'scenarios/ecology-v21-{arm}.toml').write_text(text, encoding='utf-8')
    print(json.dumps(results, indent=2), flush=True)


def run_match(binary, cfg, folder, seed):
    command = [str(binary), 'run', str(cfg), '--out', str(folder), '--seed', str(seed), '--threads', '4']
    start = time.perf_counter()
    with folder.with_suffix('.log').open('w', encoding='utf-8') as f:
        subprocess.run(command, cwd=ROOT, stdout=f, stderr=subprocess.STDOUT, check=True)
    summary = json.loads((folder / 'summary.json').read_text(encoding='utf-8'))
    row = summarize(summary, folder)
    row.update(cli_seconds=time.perf_counter()-start, folder=str(folder), summary_sha256=sha(folder/'summary.json'))
    row['owners'] = {a['owner']: ledger(a) for a in summary['accounts'] if '-incumbent' in a['owner'] or a['owner'].startswith('swift')}
    tide = next(a for a in summary['accounts'] if a['owner'] == 'tide-incumbent')
    row['tide_share_fraction'] = sum(tide['shares']) / max(1, sum(sum(a['shares']) for a in summary['accounts']))
    initial = sum(a['initial_equity'] for a in summary['accounts'])
    row['cash_ledger'] = {k: sum(a['stats'][k] for a in summary['accounts'])/initial for k in
                          ('dividends', 'combo_dividends', 'idle_cash_fees', 'idle_stock_fees', 'trade_fees', 'interest', 'bad_debt')}
    row['cash_ledger']['terminal_less_initial_shares'] = sum(a['terminal_value']-a['initial_equity']+a['initial_cash'] for a in summary['accounts'])/initial
    gains = sum(a['equity']-a['initial_equity'] for a in summary['accounts'])
    explained = sum(a['stats']['dividends']+a['stats']['combo_dividends']-a['stats']['idle_cash_fees']
                    -a['stats']['trade_fees']-a['stats']['interest']+a['stats']['bad_debt']
                    +a['terminal_value']-a['initial_equity']+a['initial_cash'] for a in summary['accounts'])
    assert gains==explained, 'Cash ledger must not double count burned shares'
    row['new_bot_risk'] = {g: {'interest': sum(a['stats']['interest'] for a in summary['accounts'] if a['strategy']==g),
                             'negative_cash': sum(a['cash'] < 0 for a in summary['accounts'] if a['strategy']==g),
                             'liquidations': sum(a['stats']['liquidations'] for a in summary['accounts'] if a['strategy']==g)}
                           for g in ('harbor', 'relay') if g in row['groups']}
    return summary, row


def ablation(binary, out):
    arms = ['full', 'flat', 'no-wash', 'flat-no-wash']
    rows = []
    for i, seed in enumerate(range(90001, 90007)):
        for repeat in range(2):
            previous = None
            for arm in arms[(i+repeat)%4:] + arms[:(i+repeat)%4]:
                cfg = ROOT / f'scenarios/ecology-v21-tide-{arm}.toml'
                folder = out / f'{arm}-{seed}-r{repeat}'
                summary, row = run_match(binary, cfg, folder, seed)
                if previous:
                    assert opening(summary) == opening(previous)
                    assert summary['terminal_payments'] == previous['terminal_payments']
                    assert summary['final_dividends'] == previous['final_dividends']
                previous = summary
                row.update(arm=arm, seed=seed, repeat=repeat)
                rows.append(row)
                save(out/'progress.json', rows)
                print(f"{len(rows)}/48 {arm} seed={seed} r={repeat} Tide={row['owners']['tide-incumbent']['net_return']:.3f}", flush=True)
    aggregate = {a: {k: st.mean(r['owners']['tide-incumbent'][k] for r in rows if r['arm']==a)
                     for k in rows[0]['owners']['tide-incumbent']} for a in arms}
    save(ROOT/'docs/ecology-v21-tide-results.json', dict(aggregate=aggregate, rows=rows))
    print(json.dumps(aggregate, indent=2), flush=True)


def health(folder, cfg):
    initial = {s['name']: s['dividend'] for s in cfg['stocks']}
    changes = defaultdict(list)
    with (folder/'news.csv').open(newline='',encoding='utf-8') as f:
        for row in csv.DictReader(f):
            for item in row['changes'].split():
                name, delta = item.split(':')
                changes[name].append((int(row['effective']),int(delta)))
    for events in changes.values():
        events.sort()
    activity = set()
    with (folder/'trades.csv').open(newline='',encoding='utf-8') as f:
        for row in csv.DictReader(f):
            activity.add((row['stock'],int(row['tick'])//20))
    with (folder/'books.csv').open(newline='',encoding='utf-8') as f:
        books = sorted(csv.DictReader(f),key=lambda x:int(x['tick']))
    rates, indices = initial.copy(), defaultdict(int)
    samples = defaultdict(list)
    for b in books:
        name,tick = b['stock'],int(b['tick'])
        events = changes[name]
        while indices[name] < len(events) and events[indices[name]][0] <= tick:
            rates[name] += events[indices[name]][1]
            indices[name] += 1
        assert rates[name] >= 0
        state = 'zero' if rates[name]==0 else ('low' if rates[name]<=initial[name]*.25 else 'normal')
        bid,ask = bool(b['bid']),bool(b['ask'])
        spread = (int(b['ask'])-int(b['bid']))/((int(b['ask'])+int(b['bid']))/2) if bid and ask else None
        sample = dict(bid=int(bid),ask=int(ask),both=int(bid and ask),
                      thin_shares=min(int(b['bid_shares_2pct']),int(b['ask_shares_2pct'])),
                      thin_money=min(int(b['bid_depth_2pct']),int(b['ask_depth_2pct'])),
                      active=int((name,tick//20) in activity),spread=spread)
        for group in ('all',state, 'distressed' if state!='normal' else 'normal_only'):
            samples[group].append(sample)
    result = {}
    for group in ('all','normal','low','zero','distressed'):
        ss = samples[group]
        result[group] = {'samples':len(ss), **{key:st.mean(vals) if (vals:=[s[key] for s in ss if s[key] is not None]) else None
                                              for key in ('bid','ask','both','thin_shares','thin_money','active','spread')}}
    return result


def aggregate_ecology(rows):
    results = {}
    for arm in sorted({r['arm'] for r in rows}):
        rr = [r for r in rows if r['arm']==arm]
        result = {k:st.mean(r[k] for r in rr) for k in ('market_net_return','tide_share_fraction','simulation_seconds','cli_seconds')}
        result.update(runs=len(rr), audits=sum(r['audits'] for r in rr),
                      faults=sum(r[k] for r in rr for k in ('errors','skipped','disabled')),
                      quality_pass=sum(r['quality']['pass'] for r in rr),
                      groups={g:{k:st.mean(r['groups'][g][k] for r in rr) for k in rr[0]['groups'][g]} for g in rr[0]['groups']},
                      owners={o:{k:st.mean(r['owners'][o][k] for r in rr) for k in rr[0]['owners'][o]} for o in rr[0]['owners']},
                      cash_ledger={k:st.mean(r['cash_ledger'][k] for r in rr) for k in rr[0]['cash_ledger']},
                      new_bot_risk={g:{k:sum(r['new_bot_risk'][g][k] for r in rr) for k in rr[0]['new_bot_risk'][g]} for g in rr[0]['new_bot_risk']})
        result['health'] = {}
        for group in rr[0]['health']:
            # Equal-match weighting. Missing-state matches are excluded and counted.
            present = [r['health'][group] for r in rr if r['health'][group]['samples']]
            result['health'][group] = dict(runs=len(present), samples=sum(p['samples'] for p in present),
                **{k:st.mean(vals) if (vals:=[p[k] for p in present if p[k] is not None]) else None for k in
                   ('bid','ask','both','thin_shares','thin_money','active','spread')})
        results[arm] = result
    base = results['control']
    for arm,v in results.items():
        if arm=='control': continue
        h,b = v['health'],base['health']
        v['acceptance'] = {
            'all_audits':v['audits']==20*v['runs'], 'zero_faults':v['faults']==0,
            'cash_only':all(x==0 for g in v['new_bot_risk'].values() for x in g.values()),
            'new_groups_profitable':all(v['groups'][g]['net_return']>=0 for g in v['new_bot_risk']),
            'market_return_not_lower':v['market_net_return']>=base['market_net_return'],
            'normal_both_95pct':h['normal']['both']>=.95,
            'normal_both_drop_max_1pp':h['normal']['both']>=b['normal']['both']-.01,
            'all_ask_improve_1pp':h['all']['ask']>=b['all']['ask']+.01,
            'distressed_ask_improve_5pp':h['distressed']['ask'] is not None and b['distressed']['ask'] is not None and h['distressed']['ask']>=b['distressed']['ask']+.05,
            'share_depth_at_least_90pct':h['all']['thin_shares']>=b['all']['thin_shares']*.9,
            'grid_loss_max25pct':v['groups']['grid']['gross_loss_share']<=.25,
            'grid_loss_increase_max5pp':v['groups']['grid']['gross_loss_share']<=base['groups']['grid']['gross_loss_share']+.05,
            'tide_share_not_up':v['tide_share_fraction']<=base['tide_share_fraction'],
        }
        v['accepted'] = all(v['acceptance'].values())
    return results


def competition(binary,out,stage):
    if stage=='dev':
        arms,start,count = ['control','harbor','relay','both'],90401,6
    else:
        development = json.loads((ROOT/'docs/ecology-v21-dev-results.json').read_text(encoding='utf-8'))
        arms,start,count = ['control',development['selected'] or 'both'],90501,10
    configs = {arm:ROOT/f'scenarios/ecology-v21-{arm}.toml' for arm in arms}
    source_hashes = {str(p.relative_to(ROOT)):sha(p) for p in (ROOT/'crates').rglob('*.rs')}
    frozen = dict(binary_sha256=sha(binary),source_hashes=source_hashes,
                  configs={p.name:sha(p) for p in (ROOT/'scenarios').glob('ecology-v21-*.toml')},
                  script_sha256=sha(Path(__file__)), review_sha256=sha(ROOT/'docs/ecology-v21-review.md'))
    if stage=='holdout':
        assert frozen == development['frozen'], 'No tuning on holdout'
    save(out/'frozen.json',frozen)
    rows = []
    for i,seed in enumerate(range(start,start+count)):
        for repeat in range(2):
            previous = None
            rotation=(i+repeat)%len(arms)
            for arm in arms[rotation:]+arms[:rotation]:
                folder=out/f'{arm}-{seed}-r{repeat}'
                cfg=tomllib.loads(configs[arm].read_text(encoding='utf-8'))
                summary,row=run_match(binary,configs[arm],folder,seed)
                if previous:
                    assert opening(summary)==opening(previous)
                    assert summary['terminal_payments']==previous['terminal_payments']
                    assert summary['final_dividends']==previous['final_dividends']
                    for a,b in zip(summary['accounts'],previous['accounts']):
                        for field in ('initial_cash','initial_shares','initial_equity','news_multiplier','forecast_access'):
                            assert a[field]==b[field],field
                    assert sha(folder/'news.csv')==sha(previous_folder/'news.csv')
                previous,previous_folder=summary,folder
                row.update(arm=arm,seed=seed,repeat=repeat,health=health(folder,cfg))
                rows.append(row)
                save(out/'progress.json',rows)
                print(f"{len(rows)}/{count*2*len(arms)} {arm} seed={seed} r={repeat} "
                      f"market={row['market_net_return']:.3f} ask={row['health']['all']['ask']:.3f}",flush=True)
    aggregate=aggregate_ecology(rows)
    eligible=[a for a in arms if a!='control' and aggregate[a]['accepted']]
    selected=max(eligible,key=lambda a:(aggregate[a]['health']['distressed']['ask'],aggregate[a]['market_net_return'])) if eligible else None
    assert sha(binary)==frozen['binary_sha256']
    assert all(sha(ROOT/p)==h for p,h in source_hashes.items())
    assert sha(Path(__file__))==frozen['script_sha256']
    assert all(sha(ROOT/'scenarios'/p)==h for p,h in frozen['configs'].items())
    save(ROOT/f'docs/ecology-v21-{stage}-results.json',dict(frozen=frozen,selected=selected,aggregate=aggregate,rows=rows))
    print(json.dumps({a:v.get('acceptance') for a,v in aggregate.items()},indent=2),flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('stage', choices=['prepare', 'calibrate', 'ablation', 'dev', 'holdout'])
    args = parser.parse_args()
    if args.stage == 'prepare':
        prepare()
        return
    from build_release import windows_build_budget
    budget = windows_build_budget() if os.name == 'nt' else os.cpu_count()
    protected = subprocess.check_output(['git','ls-tree','-r','--name-only','694183c','--','crates/bots/src'],cwd=ROOT,text=True).splitlines()
    protected = [p for p in protected if 'test' not in Path(p).name and Path(p).name != 'lib.rs']
    subprocess.run(['git','diff','--quiet','694183c','--',*protected],cwd=ROOT,check=True)
    binary = ROOT/('target/release/llm-market.exe' if args.stage in ('dev','holdout') else 'runs/dividend-v20-source/llm-market.exe')
    out = ROOT/f'runs/ecology-v21-{args.stage}'
    out.mkdir(exist_ok=False)
    manifest = dict(stage=args.stage, cpu_budget=budget, threads=4, binary_sha256=sha(binary),
                    scenarios={p.name: sha(p) for p in (ROOT/'scenarios').glob('ecology-v21-*.toml')},
                    protocol_sha256=sha(ROOT/'docs/ecology-v21-protocol.md'),
                    script_sha256=sha(Path(__file__)))
    save(out/'manifest.json', manifest)
    if args.stage in ('dev','holdout'):
        competition(binary,out,args.stage)
    else:
        (calibrate if args.stage == 'calibrate' else ablation)(binary, out)
    assert sha(binary) == manifest['binary_sha256']


if __name__ == '__main__':
    main()
