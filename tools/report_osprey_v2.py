"""Verify frozen four-arm research and write v15/Osprey reports."""
import csv
import hashlib
import json
from collections import Counter, defaultdict
from pathlib import Path
import re
import statistics as st
import tomllib
import zipfile

from research_osprey_v2 import ARMS, metrics, summary
from research_v7 import sha, write

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / 'runs/osprey-v2-holdout'


def cashflow(folder, cfg, stock):
    """Unrounded ordinary cash-flow opportunity for one continuously held share."""
    name = cfg['stocks'][stock]['name']
    changes = defaultdict(int)
    with (folder / 'news.csv').open(encoding='utf-8', newline='') as f:
        for event in csv.DictReader(f):
            for pair in event['changes'].split():
                symbol, delta = pair.split(':')
                if symbol == name:
                    changes[int(event['effective'])] += int(delta)
    rate = cfg['stocks'][stock]['dividend']
    total = 0
    for tick in range(cfg['ticks']):
        rate += changes[tick]
        if tick > 0:
            total += max(rate, cfg['world']['min_dividend'])
    s = json.loads((folder / 'summary.json').read_text(encoding='utf-8'))
    assert max(rate, cfg['world']['min_dividend']) == s['final_dividends'][stock]
    return total / cfg['world']['dividend_scale'] / 100


def main():
    data = json.loads((OUT / 'results.json').read_text(encoding='utf-8'))
    rows = data['rows']
    assert [r['seed'] for r in rows] == list(range(72001, 72021))
    m = data['manifest']
    assert sha(ROOT / 'target/release/osprey-eval.exe') == m['binary_sha256']
    with zipfile.ZipFile(OUT / 'source.zip') as archive:
        for path, digest in m['source'].items():
            assert sha(ROOT / path) == digest, path
            assert hashlib.sha256(archive.read(Path(path).as_posix())).hexdigest() == digest, path
    configs = {}
    for arm in ARMS:
        assert sha(OUT / f'{arm}.toml') == m['configs'][arm]
        configs[arm] = tomllib.loads((OUT / f'{arm}.toml').read_text(encoding='utf-8'))
    old_cfg = tomllib.loads((ROOT / 'scenarios/baseline-v14.toml').read_text(encoding='utf-8'))
    new_cfg = tomllib.loads((ROOT / 'scenarios/baseline-v15.toml').read_text(encoding='utf-8'))
    assert sha(ROOT / 'scenarios/baseline.toml') == sha(ROOT / 'scenarios/baseline-v15.toml')
    assert new_cfg['world']['dragon'].pop('start_fraction') == 1 / 3
    assert new_cfg == old_cfg

    quality, world_rows, risk = {}, [], {}
    for arm in ARMS:
        failures = Counter()
        costs, interest = [], []
        for row in rows:
            folder = OUT / f'{arm}-{row["seed"]}'
            raw = json.loads((folder / 'summary.json').read_text(encoding='utf-8'))
            assert raw['threads'] == 4 and raw['audits'] == 20
            account = next(a for a in raw['accounts'] if a['owner'] == 'osprey-research')
            stats = account['stats']
            record = json.loads((folder / 'stock-record.json').read_text(encoding='utf-8'))
            terminal_ordinary = sum((account['shares'][i] * raw['final_dividends'][i] * raw['terminal_payments'] * (raw['terminal_payment_ticks'] // raw['dividend_period']) + raw['dividend_scale'] // 2) // raw['dividend_scale'] for i in range(len(record['ledger'])))
            stock_total = sum(l['sold'] - l['bought'] - l['fees'] + l['dividends'] - account['initial_shares'][i] * record['opening_prices'][i] for i, l in enumerate(record['ledger'])) + terminal_ordinary
            # Shares burned for idle fees already disappear from stock contribution.
            # Deduct the cash part only; subtracting all idle fees double-counts burns.
            net = stock_total + stats['combo_dividends'] + account['terminal_value'] - terminal_ordinary - stats['idle_cash_fees'] - stats['interest'] + stats['bad_debt']
            assert net == account['equity'] - account['initial_equity'], (arm, row['seed'], net, account['equity'] - account['initial_equity'])
            costs.append(sum(stats[k] for k in ['trade_fees', 'idle_fees', 'interest']) / account['initial_equity'] * 100)
            interest.append(stats['interest'] / account['initial_equity'] * 100)
            fresh = metrics(folder, configs[arm])
            assert all(row[arm][k] == v for k, v in fresh.items())
            failures.update(fresh['quality_failures'])
        quality[arm] = dict(passed=sum(r[arm]['quality_pass'] for r in rows), failures=dict(failures))
        risk[arm] = dict(mean_cost_pct=st.mean(costs), mean_interest_pct=st.mean(interest),
                         liquidations=sum(r[arm]['stats']['liquidations'] for r in rows),
                         bad_debt=sum(r[arm]['stats']['bad_debt'] for r in rows),
                         max_leverage=max(r[arm]['stats']['max_leverage'] for r in rows),
                         worst_return_pct=min(r[arm]['return_pct'] for r in rows))
    for row in rows:
        stock = row['early_new']['dragon']
        if stock is None:
            continue
        item = dict(seed=row['seed'], stock=stock)
        for arm in ['late_old', 'early_old']:
            item[arm] = {k: row[arm][k] for k in ['dragon_price_peak_multiple', 'dragon_dividend_peak_multiple']}
            item[arm]['ordinary_cashflow_yuan_per_share'] = cashflow(OUT / f'{arm}-{row["seed"]}', configs[arm], stock)
        world_rows.append(item)
    world_summary = {}
    for arm in ['late_old', 'early_old']:
        rr = [r[arm] for r in world_rows]
        world_summary[arm] = {k: dict(mean=st.mean(r[k] for r in rr), median=st.median(r[k] for r in rr)) for k in rr[0]}
        world_summary[arm]['price_at_least_10x'] = sum(r['dragon_price_peak_multiple'] >= 10 for r in rr)
    cashflow_changes = [(r['early_old']['ordinary_cashflow_yuan_per_share'] / r['late_old']['ordinary_cashflow_yuan_per_share'] - 1) * 100 for r in world_rows]
    world_summary['paired_cashflow_change_pct'] = dict(mean=st.mean(cashflow_changes), median=st.median(cashflow_changes), positive=sum(x > 0 for x in cashflow_changes))
    data['summary'] = summary(rows)
    dragon = data['summary']['dragon']
    passed = {}
    for base in ['early_control', 'early_old']:
        d = dragon[f'dragon_contribution_vs_{base}']
        passed[base] = d['mean_delta'] > 0 and d['median_delta'] > 0 and d['wins'] / dragon['worlds'] >= .6
    dev = {}
    for name in ['osprey-v2-dev', 'osprey-v2-dip-dev', 'osprey-v2-hold-dev']:
        dev[name] = json.loads((ROOT / 'runs' / name / 'results.json').read_text(encoding='utf-8'))['summary']
    checks = (ROOT / 'runs/v15-final-checks.log').read_text(encoding='utf-8')
    assert 'failed;' in checks and not re.search(r'test result: FAILED', checks)
    test_count = sum(int(n) for n in re.findall(r'test result: ok\. (\d+) passed;', checks))
    baseline = json.loads((ROOT / 'runs/v15-default-check/summary.json').read_text(encoding='utf-8'))
    assert baseline['dragon']['divergence_tick'] == 3333
    assert len(baseline['accounts']) == 162
    assert all(a['strategy'] not in ['osprey', 'osprey_v1'] for a in baseline['accounts'])
    assert all(not a['disabled'] and not a['stats']['bot_errors'] and not a['stats']['skipped'] for a in baseline['accounts'])
    data.update(development=dev, quality=quality, risk=risk, world_comparison=world_summary,
                world_rows=world_rows, screen_pass=passed, promoted_to_baseline=False,
                plan=json.loads((ROOT / 'runs/osprey-v2-validation-plan.json').read_text(encoding='utf-8')),
                tests=test_count, default_check=dict(accounts=162, audits=baseline['audits'], quality=baseline['quality']))
    data['contribution_clarification'] = 'Frozen manifest wording "excludes shared idle fees" is shorthand: cash idle fees are excluded, but inventory burned for stock-paid idle fees is already reflected in stock contribution. Do not subtract stock-paid idle fees again. All 80 per-stock ledgers reconcile to account net profit after adding combo cash flows/terminal value and deducting cash idle fees/interest, plus any debt write-off.'
    data['simulation_seconds'] = sum(r[a]['seconds'] for r in rows for a in ARMS)
    data['process_seconds'] = sum(r[a]['process_seconds'] for r in rows for a in ARMS)
    data['audits'] = sum(r[a]['audits'] for r in rows for a in ARMS)
    write(ROOT / 'docs/osprey-v2-results.json', data)

    labels = {'late_old': '旧起点＋旧 Osprey', 'early_old': '新起点＋旧 Osprey', 'early_new': '新起点＋持有版 Osprey', 'early_control': '新起点＋关闭功能'}
    dragon_table = '\n'.join(f'| {labels[a]} | {d["return_pct"]:+.2f}% | {d["dragon_contribution_pct"]:+.2f} | {d["dragon_late_mean_exposure"]:.1%} |' for a, d in dragon['arms'].items())
    paired_table = '\n'.join(f'| {labels[b]} | {dragon[f"dragon_contribution_vs_{b}"]["mean_delta"]:+.2f} | {dragon[f"dragon_contribution_vs_{b}"]["median_delta"]:+.2f} | {dragon[f"dragon_contribution_vs_{b}"]["wins"]}/{dragon["worlds"]} | {"通过" if passed[b] else "未通过"} |' for b in passed)
    total_table = '\n'.join(f'| {label} | {data["summary"][group]["worlds"]} | ' + ' | '.join(f'{data["summary"][group]["arms"][a]["return_pct"]:+.2f}%' for a in ['early_control', 'early_old', 'early_new']) + ' |' for group, label in [('all', '全部'), ('dragon', '有指定大龙'), ('no_dragon', '无指定大龙')])
    dev_table = '\n'.join(f'| {label} | {dev[name]["dragon"]["dragon_contribution_vs_early_control"]["mean_delta"]:+.2f} | {dev[name]["dragon"]["dragon_contribution_vs_early_control"]["wins"]}/2 | {dev[name]["dragon"]["dragon_contribution_vs_early_old"]["mean_delta"]:+.2f} |' for name, label in [('osprey-v2-dev', '提高追涨估值'), ('osprey-v2-dip-dev', '冲高减仓、回调加成'), ('osprey-v2-hold-dev', '买入价不变、提高持仓容忍')])
    qtable = '\n'.join(f'| {labels[a]} | {q["passed"]}/20 | {", ".join(f"{k}: {v}" for k, v in q["failures"].items()) or "无"} |' for a, q in quality.items())
    risk_table = '\n'.join(f'| {labels[a]} | {r["mean_cost_pct"]:.2f}% | {r["mean_interest_pct"]:.2f}% | {r["worst_return_pct"]:+.2f}% | {r["liquidations"]} |' for a, r in risk.items() if a != 'late_old')
    net_comparison = '；'.join(f'相对{labels[b]}，账户净收益差均值 {dragon[f"new_vs_{b}"]["mean_delta"]:+.2f} 个百分点、中位数 {dragon[f"new_vs_{b}"]["median_delta"]:+.2f}，胜 {dragon[f"new_vs_{b}"]["wins"]}/{dragon["worlds"]}' for b in ['early_control', 'early_old'])
    results_text = f'''# Osprey v2：提前成长市场中的持有策略

2026-10-20。新版本工厂名 `osprey`，旧版保留为 `osprey_v1`。本轮先修改成长起点，再开发三个方案，最终冻结“买入价不变、提高持仓容忍”的版本。按预先写下的大龙直接收益筛选口径：对关闭功能对照**{'通过' if passed['early_control'] else '未通过'}**，对旧 Osprey **{'通过' if passed['early_old'] else '未通过'}**。仍保留为研究候选，未加入默认背景。

## 行为改变

使用已生效股息、本人收到的新闻、公开盘口和成交。600-tick 慢均线与 200-tick 股息快均线确认成长：股息超过初始 1.2 倍、相对慢均线对数偏离大于 0.08、归一化价格偏离大于 0.025，快股息均线超过慢线 3%。价格按剩余派息期限归一化，避免剩余期限变化被误判为趋势。

最多保留两个强势信号。普通买入价、买入目标仍按 Kestrel 判断；只将卖出估值容忍提高最多 4%，让已有仓位更耐心持有。它不是强制底仓，也不会因此提高买入报价。归一化价格从激活后高点回撤 10%–20% 时逐步撤销容忍，达到 20% 后冷却 100 ticks；重新满足条件可激活。股息或趋势转弱、已知新闻预示超过 10% 的股息下降、价格过高时也撤销容忍。撤销容忍不等于强制清仓，基础执行仍可能持有。最后 700 ticks 逐渐归零，负债风险优先于持仓容忍。

沿用单股目标上限 55%、杠杆目标 1.8、单次调整至多权益 8%。不减少观察或决策频率。`strength=0` 在相同观察上与 Kestrel 动作完全相同。没有读取 seed、账户身份、候选或大龙标签、未来事件、外部文件；隐藏标签仅用于赛后统计。

## 开发与冻结

三个方案均使用 71001–71006，每个四臂、24 场；仅 71005、71006 有指定大龙。全部结果保留，不把开发优势当作证明：

| 开发方案 | 大龙贡献相对关闭功能的平均差（百分点） | 胜场 | 相对旧版平均差 |
|---|---:|---:|---:|
{dev_table}

追涨版容易买贵；回调版过早减掉持续上涨的持仓。最终持有版有初步改善，但开发样本只有两条大龙，且没有稳定胜过旧版，因此另用 72001–72020 验证，未根据正式结果继续调参。

冻结计划在 `runs/osprey-v2-validation-plan.json`；正式源码 ZIP、源码/二进制/场景哈希和逐次命令位于 `runs/osprey-v2-holdout/`。四臂都在原阵容末尾追加同位置的普通研究账户，现金 20,000 元、每只股票 20 股、消息倍率 1、无额外研究。新起点三臂新闻相同；旧起点用于世界效果对照。相同种子不保证相同多线程交易路径。

## 正式结果

20 个世界 × 四臂 = 80 场，各重复一次；其中 {dragon['worlds']} 个有指定大龙，全部纳入，不只挑最终暴涨者。表中“大龙贡献”是该股卖出额减买入额与交易费，加普通股息、普通期末兑现，再减开局该股库存价值，除以开局账户总权益。它包含终值和股份缴费带来的库存减少，没有分配现金闲置费、融资利息及组合收入，**不是账户净收益，也不只是价差收益**。这是该股全局贡献，未单独分离确认信号以后的主升区间。

| 有龙组 | 账户平均净收益 | 大龙平均贡献（百分点） | 分化后平均大龙仓位/权益 |
|---|---:|---:|---:|
{dragon_table}

| 新版相对的对照 | 大龙贡献平均差（百分点） | 差值中位数 | 新版胜场 | 预设筛选 |
|---|---:|---:|---:|---|
{paired_table}

筛选要求配对平均差和中位差都为正，且胜场至少 60%；分别对两个对照判断。这是研究筛选，非统计显著性或跨市场保证。大龙仓位增加本身也不算成功；更多资本占用可能影响其他股票、费用和账户收益。

| 分组 | 世界数 | 关闭功能平均净收益 | 旧 Osprey | 新 Osprey |
|---|---:|---:|---:|---:|
{total_table}

有龙组的账户净收益配对结果：{net_comparison}。这与上面的“大龙单股贡献胜场”是不同指标。

| 全部世界的账户成本与风险 | 平均交易费＋闲置费＋利息/初始权益 | 其中融资利息/初始权益 | 最差净收益 | 强平次数 |
|---|---:|---:|---:|---:|
{risk_table}

成本比例的分母固定为开局权益，长局累计成本可以很高；不能将其理解成单笔费率。账户净收益已扣这些费用。单股贡献显式扣除交易费，并通过持仓减少反映股份缴费；若做账户账本还原，只能再扣现金闲置费与利息，不能重复减股份闲置费。冻结 manifest 中“排除共享闲置费”的简写不够精确，准确口径以此说明为准，公式及逐局数据未改变。

无大龙时不要求打穿基准。指定大龙以外仍可能有强势股，关闭功能也仍是有交易能力的 Kestrel 核心。这个实验没有证明稳定识别所有大龙，20 个独立世界和每臂一次运行仍有抽样与调度噪声。

## 核验

主代理审查全部新增策略、共享执行变化、世界参数与只读观察器后运行，没有独立代理审查。完整 workspace release 测试 {test_count} 项通过，bots 严格 Clippy 通过；包含买入价不被持仓功能抬高、风险优先、种子身份隔离、消息披露、撮合与现金/股份守恒测试。新起点前，远期研究仍无法区分有龙/无龙世界。

80 场共 {data['audits']} 次守恒审计；全部账户 bot 错误、停用、跳过为零。观察器的股息和交易费逐场与账户账本核对；全部股票贡献加组合现金流/终值，减现金闲置费和利息、加坏账核销后，与账户净利润逐分一致。各臂的原有市场质量检查如下，门槛未修改：

| 组 | 全项通过 | 失败项及场数（可重叠） |
|---|---:|---|
{qtable}

Windows 共用 10 核预算、4 市场工作线程、release 串行运行。模拟核心累计 {data['simulation_seconds']:.2f} 秒，包含输出写盘的子进程累计 {data['process_seconds']:.2f} 秒，整批运行 {data['elapsed_seconds']:.2f} 秒（不含编译）。这些都是实际经过时间，即墙钟时间，不是 CPU 累计时间；本次不是性能优化实验。

数据与逐世界结果见 [osprey-v2-results.json](osprey-v2-results.json)，成长调整见 [v15](baseline-v15.md)。复跑必须使用新的目录名：

```sh
python tools/arena_gate.py --cwd . -- python tools/research_osprey_v2.py osprey-v2-replay --first-seed 72001 --seeds 20
python tools/report_osprey_v2.py
```

汇总命令验证当前代码与正式冻结哈希一致后重新生成本报告；若以后改代码，应在本轮归档环境汇总，不能把新源码标成旧成绩。
'''
    (ROOT / 'docs/osprey-v2.md').write_text(results_text, encoding='utf-8')

    world_table = '\n'.join(f'| {label} | {world_summary[arm][key]["mean"]:.2f} | {world_summary[arm][key]["median"]:.2f} |' for key, title in [('dragon_price_peak_multiple', '股价峰值/真实开局参考价'), ('dragon_dividend_peak_multiple', '股息峰值/初始股息'), ('ordinary_cashflow_yuan_per_share', '全局普通股息机会，元/股')] for arm, label in [('late_old', f'旧起点：{title}'), ('early_old', f'新起点：{title}')])
    text = f'''# v15：大龙成长提前至全局约 1/3

默认 `baseline.toml` 与归档 `baseline-v15.toml` 相同。相对于 v14，唯一配置值变化是 `world.dragon.start_fraction = 0.3333333333333333`；保留 102–226 个账户、16 种策略。Tide 与 Kestrel 各一个普通账户继续保留，Osprey 仍在独立研究中。

10000 ticks 的分化起点从 5500 提前到 3333。候选共有的温和成长在此前完成，指定者从这里开始允许持续成长分化。远期研究的身份保护截止点同步修改；不是在 3333 突然跳高派息，经营消息仍不定期出现、最早普通披露后 120 ticks 才实际生效。

成长强度、事件率、回落窗口（全局 67%–81%）、目标高点窗口（87%–97%）不变，因此成长更早、更长。候选早期爬升也会更快，无指定大龙的世界并非完全不受影响。普通三层消息、费用、终值、撮合、个人时钟和决策频率没有修改。旧场景省略参数仍取 0.55，旧脉冲模型仅允许该历史值；公开规则和报告给出实际分化 tick。

## 是否更接近十倍股价

正式种子 72001–72020，每个四臂，共 80 场；其中 {len(world_rows)} 个有指定大龙。下面只比较同一个旧 Osprey、同位置研究账户的旧/新起点，避免把策略变动混进成长效果。价格由市场交易形成，多线程路径有噪声；股息路径由世界决定。

| 指标 | 均值 | 中位数 |
|---|---:|---:|
{world_table}

股价峰值达到开局参考价十倍：旧起点 {world_summary['late_old']['price_at_least_10x']}/{len(world_rows)}，新起点 {world_summary['early_old']['price_at_least_10x']}/{len(world_rows)}。全局普通股息机会的配对增幅平均 {st.mean(cashflow_changes):+.2f}%，中位数 {st.median(cashflow_changes):+.2f}%，{sum(x > 0 for x in cashflow_changes)}/{len(world_rows)} 为正。该机会是持续持有一股的未逐 tick 取整现金流，不含买入成本、费用、组合及期末兑现，不能当作账户收益。

更早成长确实给中段留下更多派息时间，但价格还受剩余期限、隐藏终值提示、供需和策略交易影响。股息目标不是固定十倍，更没有把成交价硬设为十倍。本轮价格统计以真实开局参考价为分母；旧 [大龙机会记录](dragon-opportunities.json)采用首个有成交 K 线开盘价，口径不同，不能直接拼接。

## 回归与证据

配置比较确认除起点外 v14/v15 完全一致。seed 7 的默认场景另做一次纯背景冒烟运行，实际 162 个账户、没有 Osprey，{baseline['audits']} 次守恒审计，零 bot 错误、禁用、跳过；质量全项{'通过' if baseline['quality']['pass'] else '未通过，具体失败项见结果 JSON'}。完整 workspace release 测试 {test_count} 项通过，包括两种起点的信息保护、旧配置兼容、撮合和结算测试。

四臂实验完成 {data['audits']} 次守恒审计；每场观察器的分红和交易费与账户统计一致。所有原有质量门槛保留，旧起点＋旧 Osprey 全项通过 {quality['late_old']['passed']}/20，新起点＋旧 Osprey 通过 {quality['early_old']['passed']}/20。完整失败项、收益和耗时见 [Osprey v2 报告](osprey-v2.md)及 [逐局数据](osprey-v2-results.json)。这些是带一个普通研究账户的世界对照，不是纯背景的大样本重新认证。

```sh
target/release/llm-market.exe run scenarios/baseline-v15.toml --out runs/v15-example --seed 7 --threads 4
```

旧 v14 配置与结果保留；历史强 bot 成绩继续属于当时的市场，不能移作新版本成绩。
'''
    (ROOT / 'docs/baseline-v15.md').write_text(text, encoding='utf-8')
    print(json.dumps(dict(screen_pass=passed, dragon=dragon, world=world_summary,
                         tests=test_count, audits=data['audits'], seconds=data['simulation_seconds'],
                         process_seconds=data['process_seconds']), ensure_ascii=False, indent=2))


if __name__ == '__main__':
    main()
