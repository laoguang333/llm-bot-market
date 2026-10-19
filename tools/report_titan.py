"""Collect frozen Titan challenges and development trials without rerunning or tuning them."""
import argparse
import hashlib
import json
from pathlib import Path
import statistics

ROOT = Path(__file__).resolve().parents[1]


def read(path):
    return json.loads(path.read_text(encoding='utf-8'))


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def collect(dest):
    score = read(dest / 'challenge.json')
    manifest = read(dest / 'manifest.json')
    worlds = []
    for seed in score['seeds']:
        p = dest / f'seed-{seed}'
        s = read(p / 'summary.json')
        a = next(x for x in s['accounts'] if x['owner'] == 'challenger')
        stages = a.get('news_received_by_stage')
        quality = {q['name']: q['value'] for q in s['quality']['checks']}
        worlds.append(dict(seed=seed,accounts=len(s['accounts']),rank=a['rank'],
            percentile=a['percentile'],return_pct=100*(a['return_ratio']-1),
            initial_equity=a['initial_equity'],equity=a['equity'],stats=a['stats'],
            news_multiplier=a.get('news_multiplier',1.0),news_received_by_stage=stages,
            all_skipped=sum(x['stats']['skipped'] for x in s['accounts']),
            all_errors=sum(x['stats']['bot_errors'] for x in s['accounts']),
            all_disabled=sum(bool(x['disabled']) for x in s['accounts']),audits=s['audits'],
            engine_seconds=s['wall_seconds'],quality=quality,
            quality_failed=[q['name'] for q in s['quality']['checks'] if not q['pass']],
            world_news_sha256=sha(p/'news.csv'),final_dividends=s['final_dividends'],
            terminal_payments=s['terminal_payments']))
    returns=[w['return_pct'] for w in worlds]
    contributions={key:statistics.mean(w['stats'][key]/w['initial_equity']*100 for w in worlds)
                   for key in ['dividends','combo_dividends','trade_fees','idle_fees','interest']}
    contributions['trading_inventory_residual']=statistics.mean(returns)-contributions['dividends']-contributions['combo_dividends']+sum(contributions[k] for k in ['trade_fees','idle_fees','interest'])
    return dict(command=read(dest/'command.json'),manifest=manifest,
        summary=dict(mean_percentile=score['mean'],top25_share=score['top25_share'],
                     passed=score['pass'],mean_return_pct=statistics.mean(returns),
                     median_return_pct=statistics.median(returns),worst_return_pct=min(returns),
                     best_return_pct=max(returns),wins=sum(w['rank']==1 for w in worlds),
                     losses=sum(w['return_pct']<0 for w in worlds),
                     quality_passes=sum(not w['quality_failed'] for w in worlds),n=len(worlds)),
        contributions_pct_of_initial_equity=contributions,
        worlds=worlds)


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--out',default='docs/titan-v6-results.json')
    args=parser.parse_args()
    paths=sorted((ROOT/'runs/bench').glob('titan-*'))
    trials={p.name.removeprefix('titan-'):collect(p) for p in paths
            if p.is_dir() and not (p/'INVALID.json').exists() and (p/'challenge.json').exists()}
    frozen=['frozen-validation','frozen-official','frozen-holdout','frozen-ordinary-holdout']
    assert all(k in trials for k in frozen), 'frozen challenges incomplete'
    manifests=[trials[k]['manifest'] for k in frozen]
    assert all(m==manifests[0] for m in manifests), 'frozen source/binary/config mismatch'
    a,b=[trials[k]['worlds'] for k in ['frozen-holdout','frozen-ordinary-holdout']]
    assert len(a)==len(b)==40
    for x,y in zip(a,b):
        for key in ['seed','accounts','initial_equity','world_news_sha256','final_dividends','terminal_payments']:
            assert x[key]==y[key], (x['seed'],key)
        assert x['news_multiplier']==10 and y['news_multiplier']==1
    paired=dict(n=40,mean_return_difference_pp=statistics.mean(x['return_pct']-y['return_pct']
                for x,y in zip(a,b)),advantage_outperforms=sum(x['return_pct']>y['return_pct']
                for x,y in zip(a,b)),exogenous_worlds_identical=True,
                mean_first_stage_events={str(mult):statistics.mean(w['news_received_by_stage'][0]
                    for w in ws) for mult,ws in [(10,a),(1,b)]})
    result=dict(baseline_commit='dfc685d',capital=dict(cash_cents=20_000_000,shares_each=200),
                paired_information_effect=paired,trials=trials,
                excluded=['titan-invalid-oldbinary-noadvantage: multiplier not applied; build overlap'],
                engine_checks=read(ROOT/'docs/titan-engine-checks.json'))
    (ROOT/args.out).write_text(json.dumps(result,ensure_ascii=False,indent=2),encoding='utf-8')
    print(json.dumps({k:v['summary'] for k,v in trials.items()},ensure_ascii=False,indent=2))
    print(json.dumps(paired,ensure_ascii=False))


if __name__=='__main__':
    main()
