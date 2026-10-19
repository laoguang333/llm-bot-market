"""Summarize frozen v9 research and independently reconcile forced liquidation fills."""
import json, statistics as st, os
from pathlib import Path
from build_release import windows_build_budget
if os.name=='nt':windows_build_budget()
ROOT=Path(__file__).resolve().parents[1]
OUT=ROOT/'runs/v9-research'
def read(p):return json.loads(p.read_text(encoding='utf-8'))
def strategy(a):return a['account'] if a['strategy']=='swift' else a['strategy']
def collect():
    all_rows=[]
    for group in ['pilot','bench','batch']:
        if (OUT/f'{group}.json').exists():
            all_rows.extend(dict(r,group=group) for r in read(OUT/f'{group}.json'))
    return all_rows
def aggregate(rows):
    accounts=[next(a for a in r['summary']['accounts'] if a['account']=='researcher') for r in rows]
    values=[a['return_ratio']-1 for a in accounts]
    return dict(runs=len(rows),mean_return=st.mean(values),median_return=st.median(values),
        positive=sum(v>0 for v in values),mean_percentile=st.mean(a['percentile'] for a in accounts),
        top_quartile=sum(a['percentile']>=75 for a in accounts),forecasts=st.mean(a['forecasts_received'] for a in accounts),
        mean_idle_fee=st.mean(a['stats']['idle_fees']/100 for a in accounts),mean_trade_fee=st.mean(a['stats']['trade_fees']/100 for a in accounts),
        mean_dividends=st.mean(a['stats']['dividends']/100 for a in accounts),
        mean_interest=st.mean(a['stats']['interest']/100 for a in accounts),
        mean_trades=st.mean(a['stats']['trades'] for a in accounts),
        mean_trading_terminal=st.mean((a['equity']-a['initial_equity']-a['stats']['dividends']-a['stats']['combo_dividends']+a['stats']['idle_fees']+a['stats']['interest']+a['stats']['trade_fees']-a['stats']['bad_debt'])/100 for a in accounts),
        liquidation_events=sum(a['stats']['liquidations'] for a in accounts),
        bad_debt=sum(a['stats']['bad_debt']/100 for a in accounts))
def main():
    all_rows=collect();risks=[]
    for r in all_rows:
        for a in r['summary']['accounts']:
            events=a['liquidation_events']
            assert len(events)==a['stats']['liquidations']
            assert sum(e['bad_debt'] for e in events)==a['stats']['bad_debt']
            for e in events:
                assert e['sale_notional']==sum(f['price']*f['qty'] for f in e['fills'])
                assert e['trade_fees']==sum(f['fee'] for f in e['fills'])
                assert e['cash_before']+e['sale_notional']-e['trade_fees']+e['bad_debt']==e['cash_after']
                for s,(before,after) in enumerate(zip(e['shares_before'],e['shares_after'])):
                    assert before-after==sum(f['qty'] for f in e['fills'] if f['stock']==s)
                if e['bad_debt']:assert all(q==0 for q in e['shares_after']) and e['cash_after']==0 and e['pending_orders']==0
                risks.append(dict(group=r['group'],arm=r['arm'],seed=r['seed'],repeat=r['repeat'],account=a['account'],strategy=a['strategy'],**e))
    result=dict(runs=len(all_rows),audits=sum(r['summary']['audits'] for r in all_rows),
        skipped=sum(a['stats']['skipped'] for r in all_rows for a in r['summary']['accounts']),
        bot_errors=sum(a['stats']['bot_errors'] for r in all_rows for a in r['summary']['accounts']),
        disabled=sum(bool(a['disabled']) for r in all_rows for a in r['summary']['accounts']),
        liquidation_events=len(risks),bad_debt_yuan=sum(e['bad_debt']/100 for e in risks),risk_reconciliation_errors=0,
        worst_liquidations=sorted(risks,key=lambda e:e['bad_debt'],reverse=True)[:12],performance={},arms={},background={},paired={})
    for arm in ['v8','v9']:
        rows=[r for r in all_rows if r['group']=='bench' and r['arm']==arm]
        if rows:result['performance'][arm]=dict(runs=len(rows),median=st.median(r['wall'] for r in rows),min=min(r['wall'] for r in rows),max=max(r['wall'] for r in rows),
            engine_median=st.median(r['summary']['wall_seconds'] for r in rows),bad_debt=sum(a['stats']['bad_debt']/100 for r in rows for a in r['summary']['accounts']),
            liquidations=sum(a['stats']['liquidations'] for r in rows for a in r['summary']['accounts']))
    for arm in ['h0','h300','h600','h1000']:
        rows=[r for r in all_rows if r['group']=='batch' and r['arm']==arm]
        if not rows:continue
        result['arms'][arm]=aggregate(rows)
        result['arms'][arm]['market_checks_pass']=sum(r['summary']['quality']['pass'] for r in rows)
        result['arms'][arm]['interval']=st.mean(r['summary']['world_diagnostics']['mean_stock_news_interval'] for r in rows)
        for group in sorted({strategy(a) for r in rows for a in r['summary']['accounts']}):
            means=[st.mean(a['return_ratio']-1 for a in r['summary']['accounts'] if strategy(a)==group) for r in rows]
            result['background'].setdefault(group,{})[arm]=dict(mean_return=st.mean(means),median_world_mean=st.median(means))
    if result['arms']:
        by={(r['arm'],r['seed']):next(a for a in r['summary']['accounts'] if a['account']=='researcher')['return_ratio'] for r in all_rows if r['group']=='batch'}
        for arm in ['h300','h600','h1000']:
            delta=[by[arm,seed]-by['h0',seed] for seed in range(29001,29041) if (arm,seed) in by and ('h0',seed) in by]
            if delta:result['paired'][arm]=dict(mean_delta=st.mean(delta),median_delta=st.median(delta),improved=sum(x>0 for x in delta),worlds=len(delta))
    (ROOT/'docs/baseline-v9-results.json').write_text(json.dumps(result,ensure_ascii=False,indent=2),encoding='utf-8')
    print(json.dumps({k:v for k,v in result.items() if k not in ['worst_liquidations','background']},ensure_ascii=False,indent=2))
    if risks:
        e=result['worst_liquidations'][0];print('WORST',json.dumps(e,ensure_ascii=False))
if __name__=='__main__':main()
