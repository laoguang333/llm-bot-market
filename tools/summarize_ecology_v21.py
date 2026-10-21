"""Descriptive accounting of frozen v21 matches; never tunes strategies."""
import json
from pathlib import Path
import statistics as st

from validate_v18 import save, sha

ROOT = Path(__file__).resolve().parents[1]


def main():
    results = {}
    for stage in ('dev', 'holdout'):
        experiment = json.loads((ROOT/f'docs/ecology-v21-{stage}-results.json').read_text(encoding='utf-8'))
        groups = {}
        for arm in experiment['aggregate']:
            rows = [r for r in experiment['rows'] if r['arm'] == arm]
            per_run = []
            for row in rows:
                path = Path(row['folder'])/'summary.json'
                assert sha(path) == row['summary_sha256']
                summary = json.loads(path.read_text(encoding='utf-8'))
                group_stats = {}
                for strategy in sorted({a['strategy'] for a in summary['accounts']}):
                    accounts = [a for a in summary['accounts'] if a['strategy'] == strategy]
                    capital = sum(a['initial_equity'] for a in accounts)
                    metrics = {k: sum(a['stats'][k] for a in accounts)/capital for k in
                               ('dividends','combo_dividends','idle_cash_fees','idle_stock_fees','trade_fees','interest','bad_debt','volume_notional')}
                    metrics.update(net_return=sum(a['equity'] for a in accounts)/capital-1,
                                   final_cash=sum(a['cash'] for a in accounts)/capital,
                                   terminal=sum(a['terminal_value'] for a in accounts)/capital,
                                   final_share_quantity=sum(sum(a['shares']) for a in accounts),
                                   traded=sum(a['stats']['trades'] for a in accounts),
                                   mean_decision_us=sum(a['stats']['decision_nanos_total'] for a in accounts)/max(1,sum(a['stats']['decisions'] for a in accounts))/1000,
                                   rejects=sum(a['stats']['rejects'] for a in accounts))
                    group_stats[strategy] = metrics
                per_run.append(group_stats)
            groups[arm] = {g:{k:st.mean(r[g][k] for r in per_run) for k in per_run[0][g]} for g in per_run[0]}
        results[stage] = groups
    save(ROOT/'docs/ecology-v21-accounting-results.json',results)
    for stage,arms in results.items():
        for arm,groups in arms.items():
            print(stage,arm,{g:{k:round(groups[g][k],5) for k in ('net_return','idle_cash_fees','idle_stock_fees','volume_notional','mean_decision_us')}
                             for g in ('value','coral','harbor','relay','tide') if g in groups})


if __name__ == '__main__':
    main()
