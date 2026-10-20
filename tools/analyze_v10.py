"""Read frozen experiments, audit liquidation fills and compare same-capital accounts."""
import json,statistics as st,os,hashlib,tomllib
from pathlib import Path
from build_release import windows_build_budget
ROOT=Path(__file__).resolve().parents[1];OUT=ROOT/'runs/v10-research'
if os.name=='nt':windows_build_budget()
def read(p):return json.loads(p.read_text(encoding='utf-8'))
def challenger(row):return next(a for a in row['summary']['accounts'] if a['account']=='researcher')
def titan(row):return next(a for a in row['summary']['accounts'] if a['account']=='titan-value')
def describe(rows):
    pairs=[(challenger(r),titan(r)) for r in rows]
    returns=[a['return_ratio']-1 for a,t in pairs]
    return dict(runs=len(rows),mean_return=st.mean(returns),median_return=st.median(returns),
        min_return=min(returns),positive=sum(v>0 for v in returns),
        worst_decile_mean=st.mean(sorted(returns)[:max(1,len(returns)//10)]),
        negative_final_equity=sum(a['equity']<0 for a,t in pairs),
        mean_decision_us=st.mean(a['stats']['decision_nanos_total']/max(1,a['stats']['decisions'])/1000 for a,t in pairs),
        mean_percentile=st.mean(a['percentile'] for a,t in pairs),top_quartile=sum(a['percentile']>=75 for a,t in pairs),
        beat_titan=sum(a['return_ratio']>t['return_ratio'] for a,t in pairs),
        titan_mean_return=st.mean(t['return_ratio']-1 for a,t in pairs),
        forecasts=st.mean(a['forecasts_received'] for a,t in pairs),
        mean_fees={k:st.mean(a['stats'][k]/100 for a,t in pairs) for k in ['idle_fees','trade_fees','interest','dividends','combo_dividends']},
        mean_trades=st.mean(a['stats']['trades'] for a,t in pairs),
        max_leverage=max(a['stats']['max_leverage'] for a,t in pairs),
        liquidations=sum(a['stats']['liquidations'] for a,t in pairs),bad_debt=sum(a['stats']['bad_debt']/100 for a,t in pairs))
def main():
    groups={group:read(OUT/f'{group}.json') for group in ['pilot','pilot-execution','pilot-path','batch','ablation','bench'] if (OUT/f'{group}.json').exists()}
    for group,count in [('batch',240),('ablation',40),('bench',18)]:
        assert len(groups.get(group,[]))==count, f'{group} is incomplete; use progress_v10.py for interim results'
    manifest=read(OUT/'frozen/manifest.json')
    assert all(hashlib.sha256((ROOT/p).read_bytes()).hexdigest()==h for p,h in manifest['source'].items()), 'Current source differs from frozen experiment'
    assert all(hashlib.sha256((OUT/'configs'/p).read_bytes()).hexdigest()==h for p,h in manifest['configs'].items())
    assert (ROOT/'scenarios/baseline.toml').read_bytes()==(OUT/'configs/v10.toml').read_bytes()
    assert (ROOT/'scenarios/horizon-v10.toml').read_bytes()==(OUT/'configs/new1000.toml').read_bytes()
    old,new=[tomllib.loads((OUT/'configs'/f'{arm}.toml').read_text(encoding='utf-8')) for arm in ['v9','v10']]
    old.pop('scoring');new.pop('scoring');assert old==new
    for seed in [7,27006,28012]:
        for repeat in range(3):
            dirs=[OUT/'bench'/f'{arm}-{seed}-{repeat}' for arm in ['v9','v10']]
            assert (dirs[0]/'news.csv').read_bytes()==(dirs[1]/'news.csv').read_bytes()
            summaries=[read(d/'summary.json') for d in dirs]
            assert summaries[0]['final_dividends']==summaries[1]['final_dividends']
    rows=[r for g in groups.values() for r in g];risk_count=0
    for row in rows:
        for a in row['summary']['accounts']:
            events=a['liquidation_events'];assert len(events)==a['stats']['liquidations']
            assert sum(e['bad_debt'] for e in events)==a['stats']['bad_debt']
            for e in events:
                assert e['sale_notional']==sum(f['qty']*f['price'] for f in e['fills'])
                assert e['trade_fees']==sum(f['fee'] for f in e['fills'])
                assert e['cash_before']+e['sale_notional']-e['trade_fees']+e['bad_debt']==e['cash_after']
                assert all(x-y==sum(f['qty'] for f in e['fills'] if f['stock']==s) for s,(x,y) in enumerate(zip(e['shares_before'],e['shares_after'])))
                risk_count+=1
    result=dict(runs=len(rows),audits=sum(r['summary']['audits'] for r in rows),
        skips=sum(a['stats']['skipped'] for r in rows for a in r['summary']['accounts']),
        errors=sum(a['stats']['bot_errors'] for r in rows for a in r['summary']['accounts']),
        disabled=sum(bool(a['disabled']) for r in rows for a in r['summary']['accounts']),
        liquidation_events=risk_count,risk_reconciliation_errors=0,frozen_source_verified=True,paired_news_files_identical=9,arms={},paired={},ablation={},performance={},terminal_insolvencies=[],background={})
    batch=groups.get('batch',[])
    for arm in sorted({r['arm'] for r in batch}):
        chosen=[r for r in batch if r['arm']==arm];result['arms'][arm]=describe(chosen)
        assert {r['seed'] for r in chosen}==set(range(31001,31041)) and len(chosen)==40
        result['arms'][arm]['quality_pass']=sum(r['summary']['quality']['pass'] for r in chosen)
        result['arms'][arm]['halves']={}
        for name,selected in [('first20',[r for r in chosen if r['seed']<=31020]),('second20',[r for r in chosen if r['seed']>31020])]:
            if selected:
                stats=describe(selected)
                stats['rank_gate_pass']=stats['mean_percentile']>=90 and stats['top_quartile']/stats['runs']>=0.8
                result['arms'][arm]['halves'][name]=stats
        result['arms'][arm]['dragon_strata']={}
        for selected_dragon in [False,True]:
            subset=[r for r in chosen if (r['summary']['dragon']['selected_stock'] is not None)==selected_dragon]
            if subset:result['arms'][arm]['dragon_strata']['present' if selected_dragon else 'absent']=describe(subset)
        result['background'][arm]={}
        for account in ['titan-value','swift-standard','swift-early-single']:
            accounts=[next(a for a in r['summary']['accounts'] if a['account']==account) for r in chosen]
            result['background'][arm][account]=dict(mean_return=st.mean(a['return_ratio']-1 for a in accounts),median_return=st.median(a['return_ratio']-1 for a in accounts),mean_percentile=st.mean(a['percentile'] for a in accounts),negative_final_equity=sum(a['equity']<0 for a in accounts))
    baseline={r['seed']:challenger(r)['return_ratio'] for r in batch if r['arm']=='old1000'}
    for arm in ['new0','new300','new600','new1000','titan1000']:
        delta=[challenger(r)['return_ratio']-baseline[r['seed']] for r in batch if r['arm']==arm and r['seed'] in baseline]
        if delta:result['paired'][arm]=dict(worlds=len(delta),improved=sum(d>0 for d in delta),mean_delta=st.mean(delta))
    for arm in sorted({r['arm'] for r in groups.get('ablation',[])}):
        chosen=[r for r in groups['ablation'] if r['arm']==arm]
        result['ablation'][arm]=describe(chosen)
        seeds={r['seed'] for r in chosen};staged=[r for r in batch if r['arm']=='new1000' and r['seed'] in seeds]
        if staged:result['ablation']['staged_same_worlds']=describe(staged)
    for row in batch+groups.get('ablation',[]):
        a=challenger(row)
        if a['equity']<0:
            marked_gross=sum(q*p for q,p in zip(a['shares'],row['summary']['final_marks']))
            result['terminal_insolvencies'].append(dict(arm=row['arm'],seed=row['seed'],net_return=a['return_ratio']-1,cash=a['cash']/100,terminal_value=a['terminal_value']/100,final_equity=a['equity']/100,marked_equity=(a['cash']+marked_gross)/100,liquidations=a['stats']['liquidations'],bad_debt=a['stats']['bad_debt']/100))
    for arm in ['v9','v10']:
        chosen=[r for r in groups.get('bench',[]) if r['arm']==arm]
        if chosen:result['performance'][arm]=dict(runs=len(chosen),median=st.median(r['wall'] for r in chosen),min=min(r['wall'] for r in chosen),max=max(r['wall'] for r in chosen),engine_median=st.median(r['summary']['wall_seconds'] for r in chosen))
    (ROOT/'docs/baseline-v10-results.json').write_text(json.dumps(result,ensure_ascii=False,indent=2),encoding='utf-8')
    print(json.dumps(result,ensure_ascii=False,indent=2))
if __name__=='__main__':main()
