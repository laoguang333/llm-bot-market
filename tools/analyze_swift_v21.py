"""Offline Swift attribution. Future quotes are diagnostic, never trading inputs."""
import argparse
from collections import Counter, defaultdict, deque
import csv
import json
import os
from pathlib import Path
import random
import statistics as st

from research_swift_v21 import ROOT, OUT, read, account_metrics
from validate_v18 import sha, save

OWNERS=['swift-standard','swift-early-single']


def mean(xs):
    xs=list(xs)
    return st.mean(xs) if xs else None


def interval(xs):
    rng=random.Random(210906)
    values=sorted(mean(rng.choices(xs,k=len(xs))) for _ in range(4000))
    return [values[100],values[3899]]


def aggregate(rows):
    result={}
    for arm in sorted({r['arm'] for r in rows}):
        rr=[r for r in rows if r['arm']==arm]
        result[arm]={}
        for owner in OWNERS:
            aa=[r['accounts'][owner] for r in rr]
            result[arm][owner]={k:mean(a[k] for a in aa) for k in aa[0]}
            result[arm][owner].update(runs=len(aa),wins=sum(a['net_return']>0 for a in aa),
                                     median_return=st.median(a['net_return'] for a in aa),
                                     min_return=min(a['net_return'] for a in aa),max_return=max(a['net_return'] for a in aa))
    paired={}
    for arm in sorted(result):
        if arm=='control': continue
        base='control' if arm=='both' else 'both'
        if base not in result: continue
        for owner in OWNERS:
            diffs=[]
            for seed in sorted({r['seed'] for r in rows}):
                a=[r['accounts'][owner]['net_return'] for r in rows if r['arm']==arm and r['seed']==seed]
                b=[r['accounts'][owner]['net_return'] for r in rows if r['arm']==base and r['seed']==seed]
                if a and b: diffs.append(mean(a)-mean(b))
            paired[f'{arm} minus {base}: {owner}']=dict(worlds=len(diffs),mean=mean(diffs),
                descriptive_bootstrap95=interval(diffs),positive_worlds=sum(x>0 for x in diffs),differences=diffs)
    return dict(arms=result,paired=paired)


def history():
    stages={}
    for stage in ['dev','holdout']:
        old=read(ROOT/f'docs/ecology-v21-{stage}-results.json')
        rows=[]
        for row in old['rows']:
            folder=Path(row['folder'])
            assert sha(folder/'summary.json')==row['summary_sha256']
            summary=read(folder/'summary.json')
            accounts={a['owner']:a for a in summary['accounts']}
            metrics={o:account_metrics(accounts[o]) for o in OWNERS}
            if stage=='holdout':
                makers={o:Counter() for o in OWNERS}
                buy_makers={o:Counter() for o in OWNERS}
                sell_makers={o:Counter() for o in OWNERS}
                sizes={o:[] for o in OWNERS}
                market_takers=Counter()
                market_total=0
                with (folder/'trades.csv').open(newline='',encoding='utf-8') as f:
                    for t in csv.DictReader(f):
                        value=int(t['price'])*int(t['qty'])
                        market_total+=value
                        market_takers[accounts[t['taker']]['strategy']]+=value
                        if t['taker'] in metrics:
                            owner=t['taker'];group=accounts[t['maker']]['strategy']
                            makers[owner][group]+=value
                            (buy_makers if t['aggressor']=='buy' else sell_makers)[owner][group]+=value
                            sizes[owner].append(int(t['qty']))
                for o in OWNERS:
                    assert len(sizes[o])==accounts[o]['stats']['trades']
                    metrics[o].update(mean_fill_qty=mean(sizes[o]),one_share_fills=sum(q==1 for q in sizes[o])/max(1,len(sizes[o])))
                row_extra=dict(makers={o:dict(c) for o,c in makers.items()},buy_makers={o:dict(c) for o,c in buy_makers.items()},
                               sell_makers={o:dict(c) for o,c in sell_makers.items()},
                               market_takers={g:n/market_total for g,n in market_takers.items()})
            else: row_extra={}
            rows.append(dict(arm=row['arm'],seed=row['seed'],repeat=row['repeat'],accounts=metrics,**row_extra))
        stages[stage]=dict(aggregate=aggregate(rows),rows=rows)
    hold=stages['holdout']
    hold['counterparties']={}
    for arm in ['control','both']:
        rr=[r for r in hold['rows'] if r['arm']==arm]
        report={}
        for key in ['makers','buy_makers','sell_makers']:
            report[key]={}
            for o in OWNERS:
                groups={g for r in rr for g in r[key][o]}
                report[key][o]={g:mean(r[key][o].get(g,0)/max(1,sum(r[key][o].values())) for r in rr) for g in groups}
        groups={g for r in rr for g in r['market_takers']}
        report['market_takers']={g:mean(r['market_takers'].get(g,0) for r in rr) for g in groups}
        hold['counterparties'][arm]=report
    save(ROOT/'docs/swift-v21-history-analysis.json',stages)
    print(json.dumps({s:v['aggregate'] for s,v in stages.items()},indent=2),flush=True)


def trace_account(folder,start,result):
    n=len(start['shares']);capital=result['initial_equity'];rules=start['rules'];params=start['params']
    ticks=rules['ticks_total']
    lots=[deque() for _ in range(n)]
    for s,q in enumerate(start['shares']):
        if q: lots[s].append([q,rules['stocks'][s]['initial_price'],0,True])
    assert start['cash']+sum(q*s['initial_price'] for q,s in zip(start['shares'],rules['stocks']))==capital
    counterparties=defaultdict(deque)
    market=read(folder/'market/summary.json')
    strategies={a['owner']:a['strategy'] for a in market['accounts']}
    with (folder/'market/trades.csv').open(newline='',encoding='utf-8') as f:
        names={s['name']:i for i,s in enumerate(rules['stocks'])}
        for trade in csv.DictReader(f):
            if trade['taker']==start['owner']:
                key=(int(trade['tick']),names[trade['stock']],int(trade['price']),int(trade['qty']),trade['aggressor'])
                counterparties[key].append(strategies[trade['maker']])
    realized=0;burn_cost=0;initial_realized=0;new_realized=0;paired_volume=0;winning_volume=0;holding_qty_ticks=0;paired_qty=0
    by_stock=[dict(realized=0,burn=0,dividends=0) for _ in range(n)]
    framecount=0;cash=start['cash'];fees=0;idle_cash=0;divs=0;combo=0
    fills=[];contexts={};signals=[];quotes=[];pre_shares=[];cum_div=[[0.0]*n]
    counters=Counter();request=Counter();filled=Counter();buy_groups=defaultdict(lambda:dict(notional=0,count=0,qty=0))
    peak=capital;maxdd=0;loss_timeline=[];gross=[];buy_context={};periods=defaultdict(Counter)
    file=folder/'trace'/f'account-{start["account_index"]}.jsonl'
    for line in file.open(encoding='utf-8'):
        frame=json.loads(line);t=frame['tick'];framecount+=1
        assert frame['swift_verified'] is not False
        for f in frame['fills']:
            s,q,p=f['stock'],f['qty'],f['price'];fees+=f['fee']
            cp_key=(f['tick'],s,p,q,f['side'].lower())
            assert counterparties[cp_key],('missing counterparty',cp_key)
            counterparty=counterparties[cp_key].popleft()
            cash+=(-p*q if f['side']=='Buy' else p*q)-f['fee']
            context=contexts.get(f['order_id'])
            if f['side']=='Buy':
                assert context is not None
                before_qty=sum(l[0] for l in lots[s])
                cost=sum(l[0]*l[1] for l in lots[s])/max(1,before_qty)
                # All fills of an opening IOC are one entry, even if several
                # resting makers fill it. Only a later decision is a top-up.
                adding=context['kind']=='buy_add'
                groups=['all','add' if adding else 'entry',context['state'],'counterparty_'+counterparty]
                if adding and before_qty and p<cost: groups.append('add_below_cost')
                if context['depth_needed']: groups.append('depth_needed')
                if context['reason']!='entry': groups.append('not_fresh_entry_signal')
                for group in groups:
                    a=buy_groups[group];a['notional']+=p*q;a['count']+=1;a['qty']+=q
                lots[s].append([q,p,f['tick'],False])
                fills.append(dict(stock=s,qty=q,price=p,tick=f['tick'],side='buy',groups=groups,context=context))
                filled[context['kind']]+=q
            else:
                left=q
                while left:
                    assert lots[s],('unmatched sell',s,t)
                    lot=lots[s][0];take=min(left,lot[0]);pnl=(p-lot[1])*take
                    realized+=pnl;by_stock[s]['realized']+=pnl
                    if lot[3]: initial_realized+=pnl
                    else:
                        new_realized+=pnl;paired_volume+=lot[1]*take
                        winning_volume+=lot[1]*take if p>lot[1] else 0
                        paired_qty+=take;holding_qty_ticks+=take*(f['tick']-lot[2])
                    left-=take;lot[0]-=take
                    if not lot[0]: lots[s].popleft()
                fills.append(dict(stock=s,qty=q,price=p,tick=f['tick'],side='sell',context=context))
                if context: filled[context['kind']]+=q
        for income in frame['income']:
            if isinstance(income,str): continue
            kind,d=next(iter(income.items()))
            if kind in ('Dividend','Combo','BadDebtWrittenOff'): cash+=d['amount']
            elif kind=='Interest': cash-=d['amount']
            if kind=='Dividend': divs+=d['amount'];by_stock[d['stock']]['dividends']+=d['amount']
            elif kind=='Combo': combo+=d['amount']
        inventory=frame['post']['shares'] if frame['end'] else frame['pre']['shares']
        for s,q in enumerate(inventory):
            missing=sum(l[0] for l in lots[s])-q
            assert missing>=0,('unknown inventory',s,t,missing)
            while missing:
                lot=lots[s][0];take=min(missing,lot[0]);cost=take*lot[1]
                burn_cost+=cost;by_stock[s]['burn']+=cost;missing-=take;lot[0]-=take
                if not lot[0]: lots[s].popleft()
        cash-=frame['stats']['idle_cash_fees']-idle_cash;idle_cash=frame['stats']['idle_cash_fees']
        if frame['end']: continue
        assert frame['decided'] and t==len(signals)
        signals.append(frame['swift']);quotes.append(frame['quotes']);pre_shares.append(frame['pre']['shares'])
        cumulative=cum_div[-1].copy()
        if t>0 and t%rules['dividend_period']==0:
            cumulative=[v+b[6]/rules['dividend_scale'] for v,b in zip(cumulative,frame['quotes'])]
        cum_div.append(cumulative)
        eq=frame['post']['equity'];peak=max(peak,eq);maxdd=max(maxdd,1-eq/max(1,peak))
        if t%500==0: loss_timeline.append(dict(tick=t,equity_multiple=eq/capital,stats=frame['stats']))
        gross.append(frame['pre']['gross']/capital)
        counters['flat_ticks']+=not any(inventory)
        counters['negative_cash_ticks']+=frame['pre']['cash']<0
        for s,(signal,b) in enumerate(zip(frame['swift'],frame['quotes'])):
            state='zero' if b[6]==0 else ('low' if b[6]<=rules['stocks'][s]['initial_dividend']*.25 else 'normal')
            counters['stock_samples']+=1
            counters['reason_'+signal['execution_reason']]+=1
            if signal['target_shares']>inventory[s]:
                counters['buy_intents']+=1
                counters['buy_intent_'+signal['execution_reason']]+=1
            if inventory[s]>0:
                counters['held_samples']+=1
                counters['held_ask_missing']+=b[1]==0
                counters['held_bid_missing']+=b[0]==0
                counters['held_low_volume']+=signal['volume10']<=10
                counters['held_depth_hides_exit']+=(signal['target_fraction']>0 and signal['score']>=params['exit']
                    and signal['score']-params['depth']*signal['depth']<params['exit'])
            if signal['signal_reason']=='entry':
                counters['entry_signal_samples']+=1
                counters['entry_depth_needed']+=signal['score']-params['depth']*signal['depth']<=params['entry']
            periods[t//2000]['stock_samples']+=1
            periods[t//2000]['one_sided']+=b[0]==0 or b[1]==0
        action_context={}
        for a in frame['actions']:
            assert 'Limit' in a
            a=a['Limit'];s=a['stock'];signal=frame['swift'][s];b=frame['quotes'][s]
            side=a['side']
            kind=('buy_entry' if inventory[s]==0 else 'buy_add') if side=='Buy' else (
                'end_exit' if signal['signal_reason']=='end_exit' else ('signal_exit' if signal['target_fraction']==0 else 'resize_sell'))
            context=dict(kind=kind,reason=signal['signal_reason'],score=signal['score'],flow10=signal['flow10'],
                         flow30=signal['flow30'],depth=signal['depth'],momentum=signal['momentum'],volume=signal['volume10'],
                         state='zero' if b[6]==0 else ('low' if b[6]<=rules['stocks'][s]['initial_dividend']*.25 else 'normal'),
                         depth_needed=signal['score']>params['entry'] and signal['score']-params['depth']*signal['depth']<=params['entry'])
            request[kind]+=a['qty'];counters['actions_'+kind]+=1
            action_context[(s,side)]=context
        for f in frame['post_fills']:
            assert not f['maker']
            contexts[f['order_id']]=action_context[(f['stock'],f['side'])]
        del frame
    assert framecount==ticks+1
    assert all(not q for q in counterparties.values())
    assert len(fills)==result['stats']['trades']
    assert fees==result['stats']['trade_fees'] and divs==result['stats']['dividends'] and combo==result['stats']['combo_dividends']
    assert cash==result['cash'],('cash',cash,result['cash'])
    final_cost=sum(q*p for ll in lots for q,p,_,_ in ll)
    residual=realized-burn_cost+result['terminal_value']-final_cost
    metrics=account_metrics(result)
    assert abs(residual/capital-metrics['trading_inventory_terminal'])<1e-9
    markouts=defaultdict(lambda:dict(notional=0,pnl=0,count=0,missing=0))
    for f in fills:
        t,s,p,q=f['tick'],f['stock'],f['price'],f['qty']
        if f['side']!='buy': continue
        for h in (10,30,100):
            future=t+h
            for group in f['groups']:
                a=markouts[f'{group}_{h}']
                if future>=ticks or quotes[future][s][0]<=0:
                    a['missing']+=1;continue
                # Carry is ordinary dividend per share, excluding rounding and combos.
                carry=cum_div[future+1][s]-cum_div[t+1][s]
                a['notional']+=p*q;a['pnl']+=(quotes[future][s][0]+carry-p)*q;a['count']+=1
    for a in markouts.values(): a['return']=a['pnl']/a['notional'] if a['notional'] else None
    # Realized signal directions measured only on regularly sampled two-sided books.
    signal_forward=defaultdict(list)
    for t in range(30,ticks-100,10):
        for s,signal in enumerate(signals[t]):
            b=quotes[t][s];future=quotes[t+30][s]
            if not b[0] or not b[1] or not future[0] or not future[1]: continue
            mid=(b[0]+b[1])/2;later=(future[0]+future[1])/2
            ret=later/mid-1
            group='positive' if signal['score']>params['entry'] and signal['volume10']>10 else ('negative' if signal['score']<params['exit'] else 'neutral')
            signal_forward[group].append(ret)
    detail=dict(owner=start['owner'],account=metrics,frames=framecount,fills=len(fills),checks='cash/fills/fees/dividends/combos/FIFO inventory reconciliation exact',
                realized_price=realized/capital,realized_initial_price=initial_realized/capital,realized_new_price=new_realized/capital,
                burned_cost=burn_cost/capital,terminal_minus_open_cost=(result['terminal_value']-final_cost)/capital,
                paired_price_return=new_realized/max(1,paired_volume),paired_winning_cost_share=winning_volume/max(1,paired_volume),
                paired_qty_weighted_holding_ticks=holding_qty_ticks/max(1,paired_qty),max_drawdown=max(maxdd,1-result['equity']/max(1,peak)),
                mean_gross_per_initial=mean(gross),counters=dict(counters),request=dict(request),filled=dict(filled),
                buy_groups=dict(buy_groups),markouts=dict(markouts),timeline=loss_timeline,
                signal_forward={k:dict(samples=len(v),mean=mean(v),positive_share=mean(x>0 for x in v)) for k,v in signal_forward.items()},
                stock_cash={rules['stocks'][s]['name']:{k:v/capital for k,v in a.items()} for s,a in enumerate(by_stock)})
    return detail


def traces():
    study=read(ROOT/'docs/swift-v21-traces-results.json')
    rows=[]
    for run in study['rows']:
        folder=Path(run['folder']);manifest=read(folder/'trace-manifest.json')
        assert sha(folder/'market/summary.json')==run['summary_sha256']
        for item in manifest['starts']:
            start=item['start'];result=next(a for a in manifest['result']['accounts'] if a['owner']==start['owner'])
            row=trace_account(folder,start,result)
            row.update(arm=run['arm'],seed=run['seed'],source_manifest_sha256=sha(folder/'trace-manifest.json'))
            rows.append(row)
            save(OUT/'trace-analysis-progress.json',rows)
            print(run['arm'],run['seed'],start['owner'],'price',round(row['realized_new_price'],3),'markout30',row['markouts'].get('all_30',{}).get('return'),flush=True)
    save(ROOT/'docs/swift-v21-trace-analysis.json',dict(rows=rows))


def ablations():
    study=read(ROOT/'docs/swift-v21-ablations-results.json')
    result=aggregate(study['rows'])
    save(ROOT/'docs/swift-v21-ablation-analysis.json',result)
    print(json.dumps(result,indent=2),flush=True)


def bundle():
    history_result=read(ROOT/'docs/swift-v21-history-analysis.json')
    trace_rows=read(ROOT/'docs/swift-v21-trace-analysis.json')['rows']
    trace_summary={}
    for arm in ['control','both']:
        trace_summary[arm]={}
        for owner in OWNERS:
            rr=[r for r in trace_rows if r['arm']==arm and r['owner']==owner]
            summary={k:mean(r[k] for r in rr) for k in ['realized_new_price','realized_initial_price','burned_cost',
                'terminal_minus_open_cost','paired_price_return','paired_qty_weighted_holding_ticks','max_drawdown','mean_gross_per_initial']}
            summary['worlds']=len(rr)
            summary['buy_intent_rates']={k:mean(r['counters'].get(k,0)/max(1,r['counters']['buy_intents']) for r in rr)
                for k in ['buy_intent_buy','buy_intent_wide_spread','buy_intent_one_sided_book','buy_intent_no_budget']}
            summary['holding_rates']={k:mean(r['counters'].get(k,0)/max(1,r['counters']['held_samples']) for r in rr)
                for k in ['held_ask_missing','held_bid_missing','held_low_volume','held_depth_hides_exit']}
            groups={k for r in rr for k in r['buy_groups']}
            summary['buy_notional_share']={k:mean(r['buy_groups'].get(k,{}).get('notional',0)/r['buy_groups']['all']['notional'] for r in rr) for k in groups}
            groups={k for r in rr for k in r['markouts']}
            summary['markouts']={k:dict(worlds=len(vals),mean=mean(vals)) for k in groups
                if (vals:=[r['markouts'][k]['return'] for r in rr if r['markouts'].get(k,{}).get('return') is not None])}
            summary['fill_to_requested_qty']={k:mean(r['filled'].get(k,0)/max(1,r['request'].get(k,0)) for r in rr)
                for k in ['buy_entry','buy_add','signal_exit','resize_sell']}
            summary['signal_forward']={k:dict(mean=mean(r['signal_forward'][k]['mean'] for r in rr),
                observations=sum(r['signal_forward'][k]['samples'] for r in rr)) for k in ['positive','negative','neutral']}
            summary['counterparty_markout30_contribution']={g:mean(r['markouts'].get(g,{}).get('pnl',0)/r['markouts']['all_30']['notional'] for r in rr)
                for g in groups if g.startswith('counterparty_') and g.endswith('_30')}
            trace_summary[arm][owner]=summary
    factorial={}
    rows=history_result['dev']['rows']
    for owner in OWNERS:
        factorial[owner]={}
        for key,fn in [('replace_value',lambda x:.5*((x['harbor']-x['control'])+(x['both']-x['relay']))),
                       ('replace_coral',lambda x:.5*((x['relay']-x['control'])+(x['both']-x['harbor']))),
                       ('interaction',lambda x:x['both']-x['relay']-x['harbor']+x['control'])]:
            diffs=[]
            for seed in sorted({r['seed'] for r in rows}):
                values={a:mean(r['accounts'][owner]['net_return'] for r in rows if r['arm']==a and r['seed']==seed) for a in ['control','both','harbor','relay']}
                diffs.append(fn(values))
            factorial[owner][key]=dict(worlds=len(diffs),mean=mean(diffs),negative_worlds=sum(x<0 for x in diffs),
                descriptive_bootstrap95=interval(diffs),differences=diffs)
    source_files=['swift-v21-history-analysis.json','swift-v21-trace-analysis.json','swift-v21-ablation-analysis.json']
    result=dict(trace_summary=trace_summary,factorial=factorial,
        source_hashes={p:sha(ROOT/'docs'/p) for p in source_files},analysis_sha256=sha(Path(__file__)))
    save(ROOT/'docs/swift-v21-diagnosis-results.json',result)
    print('Saved trace aggregates, factorial effects and analysis hashes.')


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('stage',choices=['history','traces','ablations','bundle'])
    stage=parser.parse_args().stage
    if os.name=='nt':
        from build_release import windows_build_budget
        windows_build_budget()
    {'history':history,'traces':traces,'ablations':ablations,'bundle':bundle}[stage]()


if __name__=='__main__': main()
