"""Offline state-aware market diagnostics. Labels never enter a bot observation."""
from bisect import bisect_left
from collections import defaultdict
import csv
import json
from pathlib import Path
import statistics as st
import tomllib


def csv_rows(path):
    with path.open(encoding='utf-8', newline='') as handle:
        return list(csv.DictReader(handle))


def rates_and_events(cfg, news):
    events = defaultdict(list)
    for event in news:
        for item in event['changes'].split():
            stock, delta = item.split(':')
            if int(delta):
                events[stock].append(dict(id=int(event['id']), delta=int(delta),
                    first=int(event['first_reveal']), public=int(event['public_tick']),
                    effective=int(event['effective'])))
    rates, dividends = {}, {}
    for stock in cfg['stocks']:
        name = stock['name']
        changes = defaultdict(int)
        for e in events[name]:
            changes[e['effective']] += e['delta']
        value, path, prefix = stock['dividend'], [], [0.0]
        for tick in range(cfg['ticks']):
            value += changes[tick]
            assert value >= 0, (name, tick, value)
            path.append(value)
            prefix.append(prefix[-1] + (value / cfg['world']['dividend_scale'] if tick else 0))
        rates[name], dividends[name] = path, prefix
    return rates, dividends, events


def quote_window(books, stock, tick):
    """Three consecutive samples, ending no later than the requested tick."""
    end = tick // 10 * 10
    times = (end-20, end-10, end)
    rows = [books.get((stock, t)) for t in times]
    if any(r is None or not r['bid'] or not r['ask'] for r in rows):
        return None
    return end-10, st.mean((int(r['bid'])+int(r['ask']))/2 for r in rows)


def information_response(cfg, events, dividends, books):
    observations = []
    sample_keys = []
    counts = defaultdict(int)
    for stock in cfg['stocks']:
        name = stock['name']
        ee = sorted(events[name], key=lambda e: e['first'])
        times = [e['first'] for e in ee]
        for e in ee:
            if abs(e['delta']) < .05*stock['dividend']:
                continue
            counts['large_events'] += 1
            t = e['first']
            if (t < 150 or t+100 >= cfg['ticks'] or e['effective'] >= cfg['ticks']
                    or e['public'] != t+60):
                counts['boundary_or_timing'] += 1
                continue
            left, right = bisect_left(times, t-120), bisect_left(times, t+101)
            if right-left != 1:
                counts['overlap'] += 1
                continue
            counts['eligible'] += 1
            windows = [quote_window(books, name, t+offset) for offset in (-120, -20, 80)]
            if any(w is None for w in windows):
                counts['missing_quotes'] += 1
                continue
            (t0,p0),(t1,p1),(t2,p2) = windows
            prefix = dividends[name]
            direction = 1 if e['delta'] > 0 else -1
            pre = direction*((p1+prefix[t1+1]-prefix[t0+1])/p0-1)
            after = direction*((p2+prefix[t2+1]-prefix[t1+1])/p1-1)
            raw = direction*(p2/p1-1)
            observations.append(dict(pre=pre, response=after, excess=after-pre,
                                     hit=float(after>0), raw_hit=float(raw>0)))
            sample_keys.append(f"{name}:{e['id']}")
            counts['positive' if direction>0 else 'negative'] += 1
    return {**{k:counts[k] for k in ('large_events','boundary_or_timing','overlap','eligible',
                                    'missing_quotes','positive','negative')},
            'used':len(observations), 'event_ids':sample_keys,
            **{k:st.mean(o[k] for o in observations) if observations else None
               for k in ('pre','response','excess','hit','raw_hit')}}


def liquidity(cfg, books, rates, active):
    sums, counts = defaultdict(lambda:defaultdict(float)), defaultdict(int)
    initial = {s['name']:s['dividend'] for s in cfg['stocks']}
    for b in books.values():
        tick, name = int(b['tick']), b['stock']
        rate = rates[name][min(tick,cfg['ticks']-1)]
        state = 'zero' if rate==0 else 'low' if rate<=initial[name]*.25 else 'normal'
        phase = 'early' if tick < cfg['ticks']//2 else 'late'
        bid,ask = bool(b['bid']),bool(b['ask'])
        both = bid and ask
        thin = min(int(b['bid_shares_2pct']),int(b['ask_shares_2pct'])) if both else 0
        vals = dict(bid=bid,ask=ask,both=both,depth10=thin>=10,thin_shares=thin,
                    active=(name,tick//20) in active)
        if both:
            vals['spread'] = (int(b['ask'])-int(b['bid']))/((int(b['ask'])+int(b['bid']))/2)
        keys = ['all',state,phase,f'{phase}_{state}']
        if state != 'normal': keys += ['distressed',f'{phase}_distressed']
        for key in keys:
            counts[key] += 1
            for k,v in vals.items(): sums[key][k] += v
    result = {}
    for key in ('all','normal','low','zero','distressed','early','late',
                'early_normal','late_normal','early_distressed','late_distressed'):
        n = counts[key]
        result[key] = dict(samples=n, **{k:sums[key][k]/n if n else None
                         for k in ('bid','ask','both','depth10','thin_shares','active')},
                         spread=sums[key]['spread']/sums[key]['both'] if sums[key]['both'] else None)
    return result


def analyze(folder, config):
    folder = Path(folder)
    cfg = tomllib.loads(Path(config).read_text(encoding='utf-8'))
    summary = json.loads((folder/'summary.json').read_text(encoding='utf-8'))
    aa = summary['accounts']
    accounts = {a['account']:a for a in aa}
    assert len(accounts)==len(aa), 'Trade attribution requires unique account names'
    books = {(b['stock'],int(b['tick'])):b for b in csv_rows(folder/'books.csv')}
    rates, dividends, events = rates_and_events(cfg, csv_rows(folder/'news.csv'))
    active, late, external, maker, trade_cash = set(),set(),defaultdict(int),defaultdict(int),defaultdict(int)
    total_volume, fee_edge_volume = 0,0
    for trade in csv_rows(folder/'trades.csv'):
        tick = int(trade['tick'])
        a,b = trade['taker'],trade['maker']
        amount = int(trade['qty'])*int(trade['price'])
        active.add((trade['stock'],tick//20))
        if tick>=cfg['ticks']//2: late.update((a,b))
        buyer,seller = (a,b) if trade['aggressor']=='buy' else (b,a)
        trade_cash[buyer] -= amount
        trade_cash[seller] += amount
        if accounts[a]['strategy'] != accounts[b]['strategy']:
            external[a] += amount
            external[b] += amount
        maker[b] += amount
        total_volume += amount
        if tick % cfg['fees']['idle_period'] >= cfg['fees']['idle_period']-4:
            fee_edge_volume += amount
    capital = sum(a['initial_equity'] for a in aa)
    gross_loss = sum(max(0,a['initial_equity']-a['equity']) for a in aa)
    fields = ('dividends','combo_dividends','idle_cash_fees','idle_stock_fees','trade_fees','interest','bad_debt','volume_notional')
    groups = {}
    for strategy in sorted({a['strategy'] for a in aa}):
        members = [a for a in aa if a['strategy']==strategy]
        start = sum(a['initial_equity'] for a in members)
        gains = sum(a['equity']-a['initial_equity'] for a in members)
        flows = {k:sum(a['stats'][k] for a in members) for k in fields}
        residual = sum(trade_cash[a['account']]+a['terminal_value']-a['initial_equity']+a['initial_cash'] for a in members)
        explained = (residual+flows['dividends']+flows['combo_dividends']-flows['idle_cash_fees']
                     -flows['trade_fees']-flows['interest']+flows['bad_debt'])
        assert gains==explained, (strategy,gains,explained)
        groups[strategy] = dict(accounts=len(members),capital_share=start/capital,net_return=gains/start,
            **{k:v/start for k,v in flows.items()}, trading_inventory_terminal=residual/start,
            late_active=st.mean(a['account'] in late for a in members),
            ruined=st.mean(a['equity']<=a['initial_equity']*.1 for a in members),
            liquidated=st.mean(a['stats']['liquidations']>0 for a in members),
            external_share=sum(external[a['account']] for a in members)/max(1,flows['volume_notional']),
            maker_share=sum(maker[a['account']] for a in members)/max(1,total_volume),
            loss_share=sum(max(0,a['initial_equity']-a['equity']) for a in members)/max(1,gross_loss),
            final_cash=sum(a['cash'] for a in members)/start,
            final_shares=sum(sum(a['shares']) for a in members))
    initial_shares = sum(sum(a['initial_shares']) for a in aa)
    final_shares = sorted((sum(a['shares']) for a in aa),reverse=True)
    return dict(health=liquidity(cfg,books,rates,active),discovery=information_response(cfg,events,dividends,books),
        groups=groups,market_return=sum(a['equity']-a['initial_equity'] for a in aa)/capital,
        turnover=2*total_volume/capital,fee_edge_share=fee_edge_volume/max(1,total_volume),
        burned_fraction=sum(summary['shares_burned'])/initial_shares,
        top5_share=sum(final_shares[:5])/max(1,sum(final_shares)),
        cash_ledger={k:sum(a['stats'][k] for a in aa)/capital for k in fields},
        owners={a['owner']:a['return_ratio']-1 for a in aa if a['owner'].startswith(('slot-','swift')) or '-incumbent' in a['owner']},
        audits=summary['audits'],errors=sum(a['stats']['bot_errors'] for a in aa),
        skipped=sum(a['stats']['skipped'] for a in aa),disabled=sum(bool(a['disabled']) for a in aa),
        negative_equity=sum(a['equity']<0 for a in aa),bad_debt=sum(a['stats']['bad_debt'] for a in aa),
        legacy_quality=summary['quality']['pass'],simulation_seconds=summary['wall_seconds'])


def mean_tree(values):
    present = [v for v in values if v is not None]
    if not present: return None
    if isinstance(present[0],dict):
        keys = sorted(set().union(*(v.keys() for v in present)))
        return {k:mean_tree([v.get(k) for v in present]) for k in keys if k!='event_ids'}
    return st.mean(present)


def aggregate(rows):
    result = {}
    for arm in sorted({r['arm'] for r in rows}):
        rr = [r for r in rows if r['arm']==arm]
        # Repeats first, then independent worlds. Equal repeat counts are also asserted by runner.
        worlds = [mean_tree([r['metrics'] for r in rr if r['seed']==seed]) for seed in sorted({r['seed'] for r in rr})]
        stats = mean_tree(worlds)
        independent_events = {(r['seed'],key) for r in rr for key in r['metrics']['discovery']['event_ids']}
        stats.update(runs=len(rr),worlds=len(worlds),event_samples=len(independent_events),
                     total_audits=sum(r['metrics']['audits'] for r in rr),
                     total_faults=sum(r['metrics'][k] for r in rr for k in ('errors','skipped','disabled')))
        result[arm] = stats
    return result


def foundation_checks(candidate, control):
    h,g,d = candidate['health'],candidate['groups'],candidate['discovery']
    def ge(x,v): return x is not None and x>=v
    def le(x,v): return x is not None and x<=v
    return dict(audits=candidate['total_audits']==20*candidate['runs'],
        zero_faults=candidate['total_faults']==0,
        no_insolvency=candidate['negative_equity']==0 and candidate['bad_debt']==0,
        normal_both=ge(h['normal']['both'],.95),normal_active=ge(h['normal']['active'],.90),
        normal_depth=ge(h['normal']['depth10'],.75),normal_spread=le(h['normal']['spread'],.02),
        late_both=ge(h['late_normal']['both'],.93),late_active=ge(h['late_normal']['active'],.85),
        distressed_ask=ge(h['distressed']['ask'],(control['health']['distressed']['ask'] or 0)-.02) and h['distressed']['samples']>0,
        backbone_survival=all(ge(g[s]['net_return'],-.20) and ge(g[s]['late_active'],.80) for s in ('value','coral')),
        no_group_exhaustion=all(le(v['ruined'],.25) for v in g.values()),
        limited_burn=le(candidate['burned_fraction'],.30),grid_loss=le(g['grid_v2']['loss_share'],.30),
        enough_events=candidate['event_samples']>=30,news_direction=ge(d['hit'],.55),
        news_excess=d['excess'] is not None and d['excess']>0)


if __name__ == '__main__':
    import argparse
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('folder');p.add_argument('config');p.add_argument('--out')
    args=p.parse_args()
    text=json.dumps(analyze(args.folder,args.config),ensure_ascii=False,indent=2)
    if args.out: Path(args.out).write_text(text+'\n',encoding='utf-8')
    else: print(text)
