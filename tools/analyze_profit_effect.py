"""Audit paired profit-effect results and export readable decision evidence."""
import csv,json,os,statistics as st
from pathlib import Path
from collections import Counter,defaultdict
from build_release import windows_build_budget
from research_v7 import read,write,sha
from research_profit_effect import OUT,ROOT,ARMS,SEEDS,verify_pair

def cohort(row):
    return [a for a in row['summary']['accounts'] if a['account'].startswith('effect#')]

def group_return(row):return st.mean(a['return_ratio']-1 for a in cohort(row))

def analyze_trace():
    manifest=read(OUT/'trace/trace-manifest.json')
    counters=Counter(); examples=[]; paths=[]
    for entry in manifest['starts']:
        start=entry['start'];name=start['account'];idx=start['account_index']
        csv_path=OUT/f'decisions-{name.replace("#","-")}.csv';paths.append(str(csv_path.relative_to(ROOT)))
        with csv_path.open('w',encoding='utf-8-sig',newline='') as dst:
            fields=['tick','stock','last_price_yuan','peer_profit_pct','own_profit_pct','score',
                    'target_weight_pct','target_shares','held_before','held_after','buy_requested','sell_requested',
                    'buy_filled_now','sell_filled_now','buy_fill_price_yuan','sell_fill_price_yuan','cash_yuan']
            writer=csv.DictWriter(dst,fieldnames=fields);writer.writeheader()
            prev={}
            for line in (OUT/'trace/trace'/f'account-{idx}.jsonl').open(encoding='utf-8'):
                row=json.loads(line)
                if not row['decided']:continue
                assert row['profit_effect_verified'] is True
                counters['verified_decisions']+=1
                for s,signal in enumerate(row['profit_effect']):
                    req=Counter();filled=Counter()
                    for action in row['actions']:
                        order=action.get('Limit',action.get('Market'))
                        if order and order['stock']==s:req[order['side']]+=order['qty']
                    for fill in row['post_fills']:
                        if fill['stock']==s:
                            filled[fill['side']]+=fill['qty']
                            filled[fill['side']+'_notional']+=fill['qty']*fill['price']
                    counters['buy_requests']+=bool(req['Buy']);counters['sell_requests']+=bool(req['Sell'])
                    counters['buy_with_positive_peer']+=bool(req['Buy'] and signal['peer_profit']>0)
                    counters['sell_with_negative_peer']+=bool(req['Sell'] and signal['peer_profit']<0)
                    if signal['target_shares']==0 and row['pre']['shares'][s]>0:
                        counters['wants_exit_stock_ticks']+=1
                        counters['exit_without_immediate_fill_stock_ticks']+=not bool(filled['Sell'])
                    values=dict(tick=row['tick'],stock=start['rules']['stocks'][s]['name'],last_price_yuan=row['quotes'][s][5]/100,
                        peer_profit_pct=signal['peer_profit']*100,own_profit_pct=signal['own_profit']*100,score=signal['score'],
                        target_weight_pct=signal['target_fraction']*100,target_shares=signal['target_shares'],
                        held_before=row['pre']['shares'][s],held_after=row['post']['shares'][s],
                        buy_requested=req['Buy'],sell_requested=req['Sell'],buy_filled_now=filled['Buy'],sell_filled_now=filled['Sell'],
                        buy_fill_price_yuan=filled['Buy_notional']/filled['Buy']/100 if filled['Buy'] else None,
                        sell_fill_price_yuan=filled['Sell_notional']/filled['Sell']/100 if filled['Sell'] else None,
                        cash_yuan=row['pre']['cash']/100)
                    writer.writerow(values)
                    kind='buy' if filled['Buy'] and signal['peer_profit']>0 else 'sell' if filled['Sell'] and signal['peer_profit']<0 else None
                    if kind and not any(x['account']==name and x['kind']==kind for x in examples):
                        examples.append(dict(account=name,kind=kind,**values))
    assert all(v['profit_effect_mismatches']==0 for v in manifest['verification'])
    return dict(seed=SEEDS[0],accounts=2,counters=dict(counters),examples=examples,csv=paths,
                note='An additional traced match; correlations are descriptive, not causal proof. Fills-now omit resting fills delivered next tick.')

def main():
    if os.name=='nt':windows_build_budget()
    rows=read(OUT/'batch.json');perf=read(OUT/'performance.json')
    assert len(rows)==len(ARMS)*len(SEEDS) and len(perf)==18
    manifest=read(OUT/'frozen/manifest.json')
    for path,h in manifest['source'].items():assert sha(ROOT/path)==h,path
    for name,h in manifest['binaries'].items():assert sha(OUT/'frozen'/name)==h,name
    for name,h in manifest['configs'].items():assert sha(OUT/'configs'/name)==h,name
    for seed in SEEDS:
        pair=[r for r in rows if r['seed']==seed];assert len(pair)==4;verify_pair(pair)
    result=dict(worlds=len(SEEDS),runs=len(rows),ticks=10000,threads=4,cpu_budget=10,
        replacement_min=min(len(cohort(r)) for r in rows),replacement_max=max(len(cohort(r)) for r in rows),
        mean_initial_equity_share=st.mean(sum(a['initial_equity'] for a in cohort(r))/sum(a['initial_equity'] for a in r['summary']['accounts']) for r in rows),
        audits=sum(r['summary']['audits'] for r in rows),
        skips=sum(a['stats']['skipped'] for r in rows for a in r['summary']['accounts']),
        errors=sum(a['stats']['bot_errors'] for r in rows for a in r['summary']['accounts']),
        disabled=sum(bool(a['disabled']) for r in rows for a in r['summary']['accounts']),
        paired_world_and_capital_checks=20,arms={},paired={},performance={},background={})
    for arm in ARMS:
        selected=[r for r in rows if r['arm']==arm];returns=[group_return(r) for r in selected]
        quality=defaultdict(list)
        for r in selected:
            for c in r['summary']['quality']['checks']:quality[c['name']].append(c)
        result['arms'][arm]=dict(mean_return=st.mean(returns),median_return=st.median(returns),min_return=min(returns),max_return=max(returns),
            positive_worlds=sum(x>0 for x in returns),mean_percentile=st.mean(st.mean(a['percentile'] for a in cohort(r)) for r in selected),
            mean_trades_per_account=st.mean(st.mean(a['stats']['trades'] for a in cohort(r)) for r in selected),
            decision_us=st.mean(st.mean(a['stats']['decision_nanos_total']/max(1,a['stats']['decisions'])/1000 for a in cohort(r)) for r in selected),
            cohort_max_leverage=max(a['stats']['max_leverage'] for r in selected for a in cohort(r)),
            cohort_liquidations=sum(a['stats']['liquidations'] for r in selected for a in cohort(r)),
            mean_components={key:st.mean(st.mean(a['stats'][key]/a['initial_equity'] for a in cohort(r)) for r in selected)
                for key in ['dividends','combo_dividends','idle_fees','trade_fees','interest']},
            mean_trade_inventory_terminal=st.mean(st.mean((a['equity']-a['initial_equity']-a['stats']['dividends']-a['stats']['combo_dividends']+a['stats']['idle_fees']+a['stats']['trade_fees']+a['stats']['interest']-a['stats']['bad_debt'])/a['initial_equity'] for a in cohort(r)) for r in selected),
            market_trades_mean=st.mean(r['summary']['trades'] for r in selected),
            wall_median=st.median(r['wall'] for r in selected),quality_pass=sum(r['summary']['quality']['pass'] for r in selected),
            quality={n:dict(failures=sum(not c['pass'] for c in v),median=st.median(c['value'] for c in v)) for n,v in quality.items()})
        for present in [False,True]:
            subset=[r for r in selected if (r['summary']['dragon']['selected_stock'] is not None)==present]
            result['arms'][arm]['dragon' if present else 'no_dragon']=dict(worlds=len(subset),mean_return=st.mean(group_return(r) for r in subset))
        result['background'][arm]={name:st.mean(next(a['return_ratio']-1 for a in r['summary']['accounts'] if a['account']==name) for r in selected)
            for name in ['horizon-incumbent','titan-value','swift-standard','swift-early-single']}
    for other in ['noise','peer_off','price_off']:
        diff=[]
        for seed in SEEDS:
            pair={r['arm']:group_return(r) for r in rows if r['seed']==seed}
            diff.append(pair['full']-pair[other])
        result['paired']['full_minus_'+other]=dict(mean=st.mean(diff),median=st.median(diff),positive=sum(x>0 for x in diff))
    for arm in ['before','after']:
        chosen=[r for r in perf if r['arm']==arm]
        result['performance'][arm]=dict(runs=len(chosen),median=st.median(r['wall'] for r in chosen),
            min=min(r['wall'] for r in chosen),max=max(r['wall'] for r in chosen),
            engine_median=st.median(r['summary']['wall_seconds'] for r in chosen))
    result['compatibility']=read(OUT/'compatibility.json')
    result['trace']=analyze_trace()
    write(ROOT/'docs/profit-effect-results.json',result)
    print(json.dumps(result,ensure_ascii=False,indent=2))

if __name__=='__main__':main()
