"""Verify and summarize the frozen Osprey feasibility experiment."""
import json
from pathlib import Path

from research_v7 import sha, write

ROOT = Path(__file__).resolve().parents[1]


def main():
    out = ROOT / 'runs/osprey-holdout'
    data = json.loads((out/'results.json').read_text(encoding='utf-8'))
    assert [r['seed'] for r in data['rows']] == list(range(62001,62041))
    assert sha(ROOT/'target/release/llm-market.exe') == data['manifest']['binary_sha256']
    for path, digest in data['manifest']['source'].items():
        assert sha(ROOT/path) == digest, path
    dev={}
    for name in ['osprey-dev','osprey-dev-v2','osprey-dev-v3']:
        d=json.loads((ROOT/'runs'/name/'results.json').read_text(encoding='utf-8'))
        dev[name]=d['summary']
    data['development']=dev
    data['checks_log']='runs/osprey-final-checks.log'
    data['plan']=json.loads((ROOT/'runs/osprey-holdout-plan.json').read_text(encoding='utf-8'))
    dragon=data['summary'].get('dragon',{})
    passed=bool(dragon and dragon['mean_delta_pp']>0 and dragon['median_delta_pp']>0
                and dragon['overlay_wins']/dragon['worlds']>=0.6)
    data['descriptive_dragon_target_met']=passed
    data['promoted_to_baseline']=False
    audits=sum(r[a]['audits'] for r in data['rows'] for a in ['osprey','control'])
    seconds=sum(r[a]['simulation_seconds'] for r in data['rows'] for a in ['osprey','control'])
    data['total_audits']=audits
    data['simulation_seconds']=seconds
    data['quality_pass_count']=sum(r[a]['quality'] for r in data['rows'] for a in ['osprey','control'])
    write(ROOT/'docs/osprey-results.json',data)
    lines=[]
    for label,group in [('全部','all'),('有指定大龙','dragon'),('无指定大龙','no_dragon')]:
        d=data['summary'][group]
        lines.append(f"| {label} | {d['worlds']} | {d['control']['mean_return_pct']:+.2f}% | {d['osprey']['mean_return_pct']:+.2f}% | {d['mean_delta_pp']:+.2f} | {d['median_delta_pp']:+.2f} | {d['overlay_wins']}/{d['worlds']} |")
    a=data['summary']['all']['osprey']
    text=f'''# Osprey：强势成长股策略的可行性研究

2026-10-20，新增工厂名 `osprey`，普通资金与普通消息权限。当前为研究候选，未加入默认 v14 阵容；“有大龙时明显更强”不能由策略名称或开发集收益推出。预先定义的描述性目标本次**{'达到' if passed else '未达到'}**：有指定大龙的世界里，配对收益差均值和中位数为正、胜率至少 60%。这不是显著性检验或跨市场保证。

## 策略

Osprey 复用 Kestrel 的新闻估值、最优价 IOC 执行、统一买入预算及负债风控，增加一个持续强势信号。600-tick 指数均线分别跟踪公开股息和按剩余派息期限归一化的价格；股息相对均线对数偏离大于 0.08、价格偏离大于 0.025 时激活。最多选择两只综合偏离最大的股票，将估值增加 `min(0.025, 0.08 × 综合偏离)`，最后 700 ticks 逐渐归零。

股息转弱、价格趋势明显转弱、已收到新闻预示股息下降超过 10%，或价格超过当前前瞻平价估值的 1.5 倍时，撤掉强势加成。撤掉加成不强制清仓，普通交易核心仍可持有。这里的 1.5 倍只限制额外加成，不是全策略买入价格上限。

默认杠杆目标 1.8、单股目标权重上限 0.55、每次调整至多权益 8%，仍由原有风控限制。没有通过降低决策频率提速。强势溢价会改变仓位、成交、融资与费用；它是一项可检验的趋势延续假设，不能保证预测增长。

`strength=0` 关闭加成，在相同观察序列上与 Kestrel 动作一致。额外选项 `min_rate_multiple=1.5` 要求股息超过初始值 1.5 倍，开发测试没有改善表现，因此默认关闭。代码不使用 seed、账户身份、候选名单、指定大龙、固定分化时点、隐藏路径或外部 IO。世界标签只供赛后分组。

## 开发与冻结

开发种子 61001–61008，每个版本都与同账户位置的关闭信号对照串行比较。保留全部三次开发结果：

- v1：强势股持仓下限，平均配对收益差 {dev['osprey-dev']['all']['mean_delta_pp']:+.2f} 个百分点，胜 {dev['osprey-dev']['all']['overlay_wins']}/8；失败，弃用。
- v2：有限估值加成，平均差 {dev['osprey-dev-v2']['all']['mean_delta_pp']:+.2f} 个百分点，胜 {dev['osprey-dev-v2']['all']['overlay_wins']}/8；选为冻结方案。
- v3：再加初始股息倍数过滤，平均差 {dev['osprey-dev-v3']['all']['mean_delta_pp']:+.2f} 个百分点，胜 {dev['osprey-dev-v3']['all']['overlay_wins']}/8；过滤默认关闭。

多线程重复的对照本身会变化；开发结果用于选择方案，不是留出证明。最终版本保留可选过滤参数但默认行为采用 v2。在第一次正式世界运行前保存源码与二进制哈希、源代码 ZIP、场景和计划；正式种子为 62001–62040，各重复 1 次，之后没有调参。每个世界两次独立市场运行，共 80 场；全部为 v14 背景外加一个同位置、同本金的 Osprey 研究账户，两组仅 `strength` 不同。

## 正式结果

收益率为最终权益 / 初始总权益 − 1，包含分红和期末兑现，并已扣所有费用；相同种子的差值不表示逐笔市场轨迹相同。

| 分组 | 世界数 | 关闭信号平均收益 | Osprey 平均收益 | 平均配对差（百分点） | 配对差中位数 | Osprey 胜场 |
|---|---:|---:|---:|---:|---:|---:|
{chr(10).join(lines)}

全部世界 Osprey 平均排名百分位 **{a['mean_percentile']:.2f}**，前 25% 为 **{a['top25']}/40**；收益中位数 **{a['median_return_pct']:+.2f}%**，最差 **{a['worst_return_pct']:+.2f}%**，强平 **{a['liquidations']}** 次。胜过同场常驻 Kestrel 为 **{a['beat_incumbent_kestrel']}/40**；两账户资金相同，但消息抽签和账户位置不同，因此它是补充竞争结果，因果对照仍应看同位置 `strength=0`。

有指定大龙只是世界类型，不能证明利润都来自那只股票，也不能把“指定成长者”视为每局股价必然大涨。无指定大龙的世界仍有普通新闻、成长候选和经营波动。没有为这份报告关闭世界规则或用事后身份给 bot 选股。

## 核验与限制

主代理自行审查策略与共享执行代码；没有独立代理审查。64 项 bots release 测试通过，含 5 项 Osprey 测试：关闭加成与 Kestrel 一致、确认成长及失效、价格单独上涨不激活、身份隔离及买入预算、可选初始股息过滤。bots 严格 Clippy 与格式化通过。没有引擎改动。

40 对世界的外生新闻和最终股息一致，研究账户初始现金 20,000 元、每只股票 20 股、普通新闻倍率 1、零额外研究均核对。全部运行的 bot 错误、禁用、跳过为零，完成 {audits} 次守恒审计。市场质量全项通过 {data['quality_pass_count']}/80，未放宽门槛。

Windows 10 核共享亲和性预算、4 工作线程，release 串行运行。80 场累计模拟实际耗时 {seconds:.2f} 秒，平均每场 {seconds/80:.2f} 秒；不含编译和报告生成，不是引擎性能改善测试。

原始数据位于 `runs/osprey-holdout/`，开发目录为 `runs/osprey-dev*`；完整逐世界数据与哈希见 [osprey-results.json](osprey-results.json)。重新汇总运行 `python tools/report_osprey.py`，复现实验必须使用新的目录名：

```sh
python tools/arena_gate.py --cwd . -- python tools/research_osprey.py osprey-replay --first-seed 62001 --seeds 40
```

## 后续目标调整（2026-10-20）

用户明确放宽要求：无大龙时可以只有普通收益、不必打穿全市场基准；重点是强势股操作技巧有效，能够利用大龙行情。此前 40 世界的冻结结果和失败结论不重写，也不能因为放宽全场排名要求，就把大龙组仅 5/17 配对胜场当作已经成功。

下一轮应把评价集中在有实际上涨机会的强势股交易上：确认信号后的介入时点、主升段参与、回撤减仓及恢复后重入、该股票的交易和持有收益，并与相同账户位置和资金的关闭信号对照比较。隐藏身份只能用于赛后审计，不能传给策略；全场排名保留为补充指标。当前尚未完成这组直接交易归因，不能把“大龙世界里账户赚钱”当成“赚到了大龙”。

当前模型生成成长股息，而非指定成交价格。对现有 17 个有指定大龙世界的价格与股息核验见 [dragon-opportunities.json](dragon-opportunities.json)：两个运行组的股息峰值中位数均为初始 10.6875 倍，股价峰值相对首个有成交 K 线开盘价的中位数分别约 3.70/3.77 倍。Osprey 组无十倍股价，对照组 1/17 达到十倍。后半程最大价格回撤中位数约 38%，该指标包含末段下跌，不等同于主升段的中途回撤。价格倍数不含已领股息，后验低点到高点也不是可实现收益承诺。
'''
    (ROOT/'docs/osprey.md').write_text(text,encoding='utf-8')
    print(json.dumps(data['summary'],ensure_ascii=False,indent=2))


if __name__=='__main__':
    main()
