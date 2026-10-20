"""Render the fixed study's documentation from its saved results."""
import json
from pathlib import Path

ROOT=Path(__file__).resolve().parents[1]
D=json.loads((ROOT/"docs/news-swift-results.json").read_text(encoding="utf-8"))
B=D["batches"]


def pair(a,b): return next(p for p in D["pairs"] if p["a"]==a and p["b"]==b)
def ci(xs): return f"[{xs[0]:+.2f}, {xs[1]:+.2f}]"


news=["""# News 修复与增强版（2026-10-19）

新增了 `news_fixed` 与 `news_v2`。旧 `news` 保留为对照，正式 v5/v6 的背景阵容继续使用旧实现。Swift 和引擎没有因本轮修改。用户在别处完成的引擎修正已包含在本次运行中，不能直接与旧版本历史收益相减。

**追加审计限制：** 已有比赛发现全市场普通派息超过合法上限的情况，详见 [Swift 机制报告的派息核验](swift-mechanism.md#追加核验派息与除息机制)。下列成绩仍是当前引擎的有效运行记录，但尚未排除派息机制异常对收益与相对排名的影响。

## 修复了什么

旧 News 以“下单尝试次数”递减计划，三次尝试后就删除；公开消息通常找不到原计划，无法完成文档承诺的事件退出。此外，事件 id 的去重通常只让一条多股票新闻中的第一只股票进入计划。它也没有把计划完成与真实成交量对应起来。

`news_fixed` 改为按 (事件 id、股票) 保存信息并合并后续区间。私下消息产生相对开局持股的增减仓目标，公开时刻到达后取消这个事件的目标；根据账户实际持股继续下单，直到净目标达成。部分成交、拒单、未成交不会被当成已完成的仓位。重叠事件先按同股票净额合并，不把同一笔持仓重复归给多条新闻。这里“平仓”是撤销事件带来的偏离，回到开局 20 股锚点，并非卖光原始库存；不是逐事件成本归属账本。

它还共享所有股票的买入预算，遵守可售股数和动作上限，以最优价 IOC 执行，不允许卖空。完整生命周期、多股票参与、执行价和预算都有改变，因此不能将全部收益差归因于单个 bug。

## news_v2 的交易规则

增强版保留 Swift 的订单流、盘口、动量、迟滞开关、小仓位和最优价执行，加入合法收到的新闻偏置。它不再把新闻公开本身当作强制退出条件，而是持续判断市场压力与信息方向。正式 Swift 源码保持冻结；news_v2 的新闻权重为零时，测试验证其逐步输出与原版 Swift 相同。

`score = Swift 原分数 + 0.35 × news_impulse`

每个已收到的事件，对该股计算区间中点和半宽；方向幅度按中点相对当前股息的 10% 标准化，裁剪到 ±1；置信权重为 `|中点| / max(1, |中点| + 半宽)`。私下阶段时间权重为 1，公开后为 `0.5 × exp(-公开后tick数 / 30)`，90 tick 后删除。合并多条新闻后再裁剪到 ±1。

新闻项只使用已投递的区间、公开时间和当前股息，交易部分使用公共行情、规则及本账户；不读取世界种子、隐藏事件或其他账户。公开新闻也可贡献短暂偏置，不能将增强版的改善全部称为私有信息收益。固定 news_weight=0.35，没有在本批收益上搜索参数。

## 固定参数比较

每行 40 局，种子 21001–21040，同一当前引擎、同样的追加账户位置、现金 20,000 元和每股 20 股。打穿门槛仍是平均排名百分位 ≥90，至少 80% 的局进入前 25%。收益已含分红、费用和终末结算。

| 市场 | 挑战者 | 平均收益 | 收益中位数 | 亏损局 | 平均排名百分位 | 前 25% | 打穿 |
|---|---|---:|---:|---:|---:|---:|---|
"""]
for market in ["v5","v6"]:
    for bot in ["news","news_fixed","news_v2","swift"]:
        b=B[f"{market}-{bot}-plain"]
        news.append(f'| {market} | {bot} | {b["mean_return_pct"]:+.2f}% | {b["median_return_pct"]:+.2f}% | {b["losses"]}/40 | {b["mean_percentile"]:.2f} | {b["top25"]}/40 | {"是" if b["pass"] else "否"} |\n')
news.append("\n修复版的收益中位数和排名有所改善，但平均收益没有提升；两版市场中修复版减旧版的平均收益差区间都跨零。修好事件管理并不自动使单纯新闻跟单成为强策略。旧 News 也不是在所有新测试中必然亏损：其平均收益被少数高收益世界抬高，同时约一半世界亏损。\n\n")
for market in ["v5","v6"]:
    p=pair(f"{market}-news_v2-plain",f"{market}-swift-plain")
    news.append(f'{market} 中增强版减 Swift 的平均收益差为 **{p["mean_difference_pp"]:+.2f} 个百分点**，世界 bootstrap 95% 区间 **{ci(p["ci95"])}**，同种子比较赢 {p["a_wins"]}/40 局。前后两批各 20 局的差为 {p["first20_difference_pp"]:+.2f} / {p["second20_difference_pp"]:+.2f} 个百分点。\n\n')
news.append("""这支持本轮固定配置下“新闻信息能给订单流策略带来增益”，不意味着所有新闻信号都有价值。v6 增强版仍有 19/40 局亏损、排名只有 54.36，远未打穿；前一半世界相对 Swift 的增益仅 0.78 个百分点，表现不稳定。没有根据后半批再调参。

修复版在 v5 有 1 次拒单、2 次强平，其他本表挑战者均零拒单、零强平。全部账户零跳过、零 bot 错误、零停用。不能将修复版当作已经充分解决风险管理的最终策略。

## 实现与验证

- [修复版](../crates/bots/src/news.rs)、[增强版](../crates/bots/src/news_v2.rs) 由工厂分别注册，旧 `news` 保持历史行为。
- [运行前审查](news-swift-bot-review.md)与[固定方案](news-swift-mechanism-protocol.md)保留；全工作区 Clippy、81 项 Rust 测试、release 构建通过。
- 研究全程 4 工作线程，Windows 子进程共享 10 核预算，实验串行。没有改决策频率、消息投递、撮合或结算。
- [完整结果](news-swift-results.json)含每局收益、费用和诊断；[冻结计划](news-swift-plan.json)含源码与二进制哈希。原始摘要/轨迹及源码 ZIP 在 `runs/news-swift-20261019/`。本轮共用当前未提交工作区，Git HEAD 单独不能标识实验版本。

单独运行增强版：

```sh
python tools/build_release.py
./target/release/llm-market challenge scenarios/baseline-v5.toml --bot news_v2 --out runs/news-v2-v5-new --seeds 40 --first-seed 21001 --threads 4
./target/release/llm-market challenge scenarios/baseline-v6.toml --bot news_v2 --out runs/news-v2-v6-new --seeds 40 --first-seed 21001 --threads 4
```

市场反馈与并发路径会改变复跑值；本轮没有轮换账户位置，不能把它当作与调度无关的绝对策略排名。Swift 为什么赚钱的独立机制分析见 [Swift 机制研究](swift-mechanism.md)。
""")
(ROOT/"docs/news-v2.md").write_text("".join(news),encoding="utf-8")

m=["""# Swift：新闻、成交方向、活跃度与大阳线（2026-10-19）

**现有证据支持：新闻产生的价值变化和市场反应，提供了更强的可利用方向；仅增加成交活跃度不能恢复 Swift 的高收益。Swift 不必在大阳线已经出现后才买入，但这不等于后续大涨对利润不重要。总收益还包含大量分红，不能全称为炒价差。**

**追加核验发现派息权归属异常。** 在 160 局已有轨迹中，15 局的全市场普通派息超过“全部初始股票全程领息”的上限。下列策略成绩尚未排除这一机制问题的影响，不能据此认定高收益完全来自正常的市场机会。核验细节见本页“追加核验：派息与除息机制”。

这次共 23 组 × 40 = **920 局**，固定种子 21001–21040，前后两半分别报告。原版 Swift、正式 v5/v6 文件与引擎保持本轮开始时的实现。信号干预仅施加给挑战者；无新闻和高活跃对照另复制场景，分别关闭新事件、改变背景 Noise 参数。原始参数没有用结果重新优化。[预先固定方案](news-swift-mechanism-protocol.md)、[离线分析口径](news-swift-analysis-plan.md)、[运行前审查](news-swift-bot-review.md)和[完整结果](news-swift-results.json)可复核。News 的比较另见 [News 修复与增强](news-v2.md)。

## 活跃本身是否足够

三组每组 40 局、相同 Swift：

| 市场 | 平均总成交额（万元/局） | 平均成交笔数 | Swift 平均收益 | 中位收益 | 亏损局 |
|---|---:|---:|---:|---:|---:|
"""]
for label,key in [("原版 v5","v5-swift-plain"),("关闭新新闻","no_news-swift-plain"),("无新闻 + 高频随机 Noise","active_no_news-swift-plain")]:
    b=B[key]
    m.append(f'| {label} | {b["mean_market_volume_yuan"]/10000:,.1f} | {b["mean_market_trades"]:,.0f} | {b["mean_return_pct"]:+.2f}% | {b["median_return_pct"]:+.2f}% | {b["losses"]}/40 |\n')
p=pair("active_no_news-swift-plain","no_news-swift-plain")
m.append(f"""
提高随机 Noise 的报单频率和数量后，成交额达到原版 v5 的 **{B['active_no_news-swift-plain']['mean_market_volume_yuan']/B['v5-swift-plain']['mean_market_volume_yuan']:.2f} 倍**，但相对普通无新闻市场，Swift 平均收益只变化 **{p['mean_difference_pp']:+.2f} 个百分点**，95% 区间 {ci(p['ci95'])}。前后两半差异一正一负，尚无稳定改善。

这不是精确匹配所有流动性条件的实验：Noise 同时改变了价差、深度、库存分布和成交结构。但实际活跃度已经更高，仍没有恢复新闻环境下的高收益，因此“仅仅因为交易变多”不足以解释。

无新闻市场的价值穿越检查均未通过，部分世界的收益自相关也未达标；这是有意移除价值冲击后的结构变化。双边盘口覆盖、价差和深度仍良好，不是把交易全部停掉。高 Noise 对照的价差中位数约 0.077%，原版 v5 约 0.320%。不能宣称三种市场完全等价。

## 方向是否真的有信息

在仅订单流的 Swift 上关闭盘口/动量权重，然后只改变它看到的主动买卖方向。每次观察内，成交 tick、价格、数量和顺序都保留；随机方向由独立常量初始化的伪随机流产生，不读世界种子。变更策略动作后，后续市场当然仍会发生反馈，因此不是固定同一价格路径的无冲击回放。

| v5 对照 | 平均收益 | 中位收益 | 亏损局 | 平均排名百分位 |
|---|---:|---:|---:|---:|
""")
for label,key in [("完整 Swift","v5-swift-plain"),("只用真实成交方向","v5-swift-flow"),("成交方向随机化","v5-swift-random"),("成交方向反转","v5-swift-reverse")]:
    b=B[key]
    m.append(f'| {label} | {b["mean_return_pct"]:+.2f}% | {b["median_return_pct"]:+.2f}% | {b["losses"]}/40 | {b["mean_percentile"]:.2f} |\n')
p=pair("v5-swift-random","v5-swift-flow")
m.append(f"""
随机方向减真实方向的收益差为 {p['mean_difference_pp']:+.2f} 个百分点，95% 区间 {ci(p['ci95'])}。v6 也有同样方向的结果：真实方向 +11.62%，随机方向 −27.33%。这说明真实方向的内容提供了超越成交量启用门槛的信息，不能将它替换为任意买卖符号。

赛后按观察到的活跃度与方向分层，计算卖一买入、30 tick 后买一卖出的报价收益（加普通分红、买卖各扣 5 bp 费用），先在世界内汇总再对世界等权：

| v5 观察条件 | 平均后续报价收益 | 世界 bootstrap 95% 区间 |
|---|---:|---|
""")
t=D["traces"]["v5-swift-plain"]
for label,key in [("高活跃 + 强主动买入","high_buy"),("高活跃 + 弱方向","high_weak"),("高活跃 + 强主动卖出","high_sell"),("普通活跃 + 强主动买入","ordinary_buy")]:
    v=t["groups"][key]
    m.append(f'| {label} | {v["mean_quote_return_pct"]:+.2f}% | {ci(v["ci95"])} |\n')
m.append("""
高活跃定义为该股票 volume10 至少同世界中位数的两倍；强方向阈值为 flow10 ≥0.3 或 ≤−0.3。这是预先写明的赛后分层，不是给 bot 使用的未来中位数。报价收益不保证该数量能成交，不包含闲置费、利息或组合股息，也不是策略实际平仓利润。强卖出时高成交量同样可伴随负收益，“量大”本身没有确定的买入含义。

## 是不是只是在追大阳线

固定“大阳线代理”为过去 30 tick 盘口中价上涨至少 1%，这是滑动价格窗口，并非固定边界 OHLC。两种买入过滤只删掉相应买单，所有卖出规则保持原样。

| v5 对照 | 平均收益 | 中位收益 | 平均排名百分位 |
|---|---:|---:|---:|
""")
for label,key in [("完整 Swift","v5-swift-plain"),("禁止大阳线之后买入","v5-swift-exclude_green"),("只允许大阳线之后买入","v5-swift-only_green"),("只用价格开关（含平滑）","v5-swift-green")]:
    b=B[key]
    m.append(f'| {label} | {b["mean_return_pct"]:+.2f}% | {b["median_return_pct"]:+.2f}% | {b["mean_percentile"]:.2f} |\n')
m.append("""
“禁止大阳线之后买入”仍平均盈利 +80.89%，说明已经出现大阳线不是入场的必要条件；但比完整 Swift 少赚 56.02 个百分点，且只买大阳线之后也少赚。当前两类时段都贡献机会。

价格单独对照的实施细节：30 tick 涨幅达到 1% 时打开价格开关，降至零或以下关闭，把开关映射为方向输入，经过 Swift 原有 10/30 tick 平滑和执行器；所以它有额外平滑延迟。它仍有成交量启用门槛。这是一个固定价格控制，不代表所有 K 线策略，也不应描述成看到大阳线就立刻无条件买入。

实际买入成交的分类进一步支持这一点：

| v5 Swift 买入时点 | 平均买入金额占比 | 买入后 30–34 tick 报价收益 |
|---|---:|---:|
""")
for label,key in [("此前已有大阳线","green"),("此前没有大阳线","not_green")]:
    f=t["fills"][key]
    m.append(f'| {label} | {f["mean_notional_share"]*100:.1f}% | {f["mean_quote_return_pct"]:+.2f}% |\n')
m.append("""
**这里仅检验“是否必须等大阳线出现后入场”。** 排除这类入场仍允许先买入、再经历大阳线，因此没有证明利润不依赖后续较大的上涨。也没有将所有价格趋势剔除：盘口、主动成交和价格仍然相互影响。

v6 中排除大阳线后买入，平均收益由 +11.02% 降到 +3.06%，差异区间跨零；拥挤环境里不能机械复用 v5 的贡献比例。

## 新闻时点与实际买入

仅在赛后使用真实 news.csv 标签，策略本身始终看不到隐藏新闻。每个时点归给过去 180 tick 内最近一条影响该股的新闻，区分首次披露到公开之前和公开之后；重叠事件会干扰这种归属。

| v5 实际买入的事件窗口 | 平均买入金额占比 | 买入后 30–34 tick 报价收益 |
|---|---:|---:|
""")
for label,key in [("最近利好处于私下披露阶段","positive_private"),("最近利好已经公开","positive_public"),("最近利空处于私下披露阶段","negative_private"),("最近利空已经公开","negative_public"),("距最近新闻首次披露超过 180 tick / 无新闻","far_from_news")]:
    f=t["fills"][key]
    m.append(f'| {label} | {f["mean_notional_share"]*100:.1f}% | {f["mean_quote_return_pct"]:+.2f}% |\n')
m.append("""
利好私下阶段买入后的报价表现较强，与信息逐步传导留下机会的解释相容；公开利空窗口较弱。但这个标签来自事后真值，并不表示 Swift 知道事件方向或能识别谁是知情账户。离开近新闻窗口也有正的报价表现，不能把所有利润归给单条新闻。

全体观察点在利好私下阶段的平均 flow10 仍约 −0.38，远离近新闻窗口约 −0.56；这里表现为相对卖压减轻，并非所有利好时刻都变成净主动买入。Swift 选择的是其中较强的短期买压片段，不是逢新闻必买。

## 总收益里面还有多少分红

下表按每局平均金额拆账，单位为元。交易/持股估值部分以总损益减去收入、加回费用求得，包含开局股票与期末持股的估值变化，未扣表中各项费用；不能当成只统计已平仓订单的利润。

| 项目 | v5 Swift | v6 Swift | 无新闻 Swift | 无新闻 + 高活跃 Swift |
|---|---:|---:|---:|---:|
""")
ks=["v5-swift-plain","v6-swift-plain","no_news-swift-plain","active_no_news-swift-plain"]
for label,kind in [("交易/持股估值部分","price"),("普通及组合分红","div"),("交易费","trade_fees"),("闲置费","idle_fees"),("利息","interest"),("总损益","total")]:
    vals=[]
    for k in ks:
        b=B[k]; inc=b["mean_income_cost_yuan"]
        val=b["mean_price_inventory_pnl_yuan"] if kind=="price" else b["mean_pnl_yuan"] if kind=="total" else inc["dividends"]+inc["combo_dividends"] if kind=="div" else -inc[kind]
        vals.append(f"{val:+,.0f}")
    m.append(f'| {label} | '+" | ".join(vals)+" |\n")
m.append("""
v5 的交易/持股估值和分红两部分都为正。v6、无新闻和高活跃无新闻组的交易/持股估值部分为负，靠分红覆盖后总收益才为正。因此“Swift 平均盈利”不能解释成相同百分比的纯短线价差利润。它虽然不读取分红预测，但持股仍会按游戏规则领取分红。

这些是同一已发生路径的账务分类，不是关闭分红后的反事实实验。分红会改变现金、权益、仓位目标与其他账户行为；从最终损益中减去分红不能预测改变派息规则后会赚多少。

## 追加核验：派息与除息机制

用户提出“是否领完息仍能原价卖出”后，对已有 160 局轨迹进行了事后核验，没有新增比赛或修改引擎。

**价格没有强制按分红下调。** 引擎在账户个人 tick 的 50 倍数先入账分红、再执行决策，没有在这个步骤统一下调已有挂单或成交价格。另一方面，共用的 `payments_after`/`hold_value` 会少计已支付的一期，所以估值型 bot 的参考价值会下降。市场价格依靠各账户撤单与重新报价调整；旧买单可能提供领息后的卖出机会，但不保证有足够数量成交，也不代表所有买家愿意原价接盘。此设计是否应改为统一除息规则，需要与派息权修复分开评估。

描述性检查使用 Swift 看到的双边盘口中价：以派息前 5 tick 为起点，v5 在派息 tick、之后 10 tick、之后 25 tick 的平均价格变化，约为当期每股分红的 −0.007、−0.496、−0.771 倍。价格平均逐步下调，并非一直不跌；这些窗口同时包含新闻、闲置费、利息及异步时钟影响，不能当成精确的除息因果效应，也不能当成 Swift 的实际卖价。

**另外发现真正的派息总额异常。** 每股只能按每期实际流通量领一次普通分红。股票没有增发、只能因费用销毁，因此不用知道销毁时间，就能得到全市场合法派息的上下界：

`最终流通股数 × 各期每股分红之和 ≤ 合法总派息 ≤ 初始流通股数 × 各期每股分红之和`

逐股票计算后求和，与全部账户 `stats.dividends` 对照；排除组合分红和终末结算，并验证期末股份 = 初始股份 − 已销毁股份。

| 已有轨迹组 | 超过合法上限的局数 | 单局最大已证实超额（全市场，元） |
|---|---:|---:|
| v5 Swift | 4/40 | 1,121.76 |
| v6 Swift | 9/40 | 1,543.75 |
| 无新闻 Swift | 0/40 | 0.00 |
| 无新闻 + 高活跃 Swift | 2/40 | 1,307.90 |

15/160 局越过上限，足以证明现有派息不总能满足“一股一期一份分红”的权利约束。未越界不等于正确：重复和遗漏可能抵消，销毁时间也使上下界宽于精确应付值。表中超额只是已经证明的下限，不是异常总额，更不是 Swift 获得的异常分红。

代码中的直接风险是按每个账户各自的时钟结算、以当时持股计息，没有全市场统一的股权登记快照。领先账户领息后将股票转给尚未结算该期的账户，后者可能再次领息。需要专门的最小复现和修复后对照来量化这一通道。现金守恒检查不能代替权利检查：额外入账同时扣减 treasury，钱仍然守恒。

因此，目前不能证明 Swift 的高收益全来自缺陷，也不能声称已经排除了缺陷影响。v5 在原轨迹中扣掉分红和全部费用，仍有约 +24,007 元的平均交易/库存估值净贡献；但该轨迹本身也可能被分红、过时报价和派息权异常改变。应先验证统一权利登记，再比较修复前后；单独把分红数字从利润中减掉不够。

完整逐局上限与报价窗口保存在 [追加审计 JSON](swift-dividend-audit.json)，由 [只读核验脚本](../tools/swift_dividend_audit.py)生成，原始输入哈希随结果记录。本次未调整引擎、股票派息或正式 bot，也未将这 160 局算作新的实验样本。

## 支持到哪里、还没有确定什么

本轮最强证据是：方向内容对结果重要；提高随机成交活跃度不足以恢复新闻市场里的收益；已出现的大阳线不是必需的入场前提。新闻窗口中的实际买入与后续报价也支持信息传导留下可利用机会的解释。

仍不能精确分离真实价值变化、其他账户的反应滞后、盘口供给、自身成交反馈，以及同类竞争分别贡献了多少。随机方向对照改的是可见信息，高 Noise 改的是市场环境；二者都有后续反馈。没有构造“完全相同成交量/价差/价格路径而只改变新闻”的独立世界。所有组固定追加位置，不包含位置轮换；此前 v6 的位置分差仍是限制。

每组只用 40 个世界，每配置/种子一次，多线程不逐笔复现。95% 区间按世界 bootstrap，未作多重比较校正，不能把大量相关观察点当作几十万个独立实验。本研究仍限于有限分红的模拟市场，未验证真实市场普适盈利。

## 完整批次与执行核验

| 批次 | 前 20 局平均收益 | 后 20 局平均收益 | 质量全部达标 | 命令墙钟 |
|---|---:|---:|---:|---:|
""")
for k,b in B.items():
    m.append(f'| `{k}` | {b["first20_return_pct"]:+.2f}% | {b["second20_return_pct"]:+.2f}% | {b["quality_passes"]}/40 | {b["command_wall_seconds"]:.2f} s |\n')
m.append("""
920 局共有 52,391 个账户局，157,173,000 次决策。所有账户每局 3000 次决策，零跳过、零 bot 错误、零停用，5,520 次守恒审计通过。全市场共 27 次强平、10,489,038 次拒单；不能写成全部零拒单/零强平。原版 Swift 与 news_v2 挑战者均零拒单、零强平；news_fixed 有 1 次拒单、2 次强平。

实际成交特征匹配无错位。四组有轨迹的测试一共 49 笔最后一 tick 的卖出没有下一次 observation 可供回报，由真实 trades.csv 核对全部为期末卖出；买入成交全部记录。实际收益使用完整 summary，未删除这些卖出。后续报价分析本来就排除没有足够未来报价的窗口。

release、4 工作线程、每组/种子一次。Windows 20 个逻辑核心中只允许进程树共享 10 核，arena_gate 串行，累计引擎墙钟约 327.86 秒，命令墙钟约 346.62 秒（含报告/启动）。这不是性能对比实验。Clippy 严格检查、81 项 Rust 测试及全 release 二进制构建通过。

研究 harness 在 [flow-eval.rs](../crates/cli/src/bin/flow-eval.rs)，调度脚本为 [news_swift_study.py](../tools/news_swift_study.py)，汇总为 [news_swift_report.py](../tools/news_swift_report.py)。冻结计划、原始数据、源码 ZIP 与二进制位于 `runs/news-swift-20261019/`，代码使用了本轮开始时已有的未提交引擎修改。

在没有旧输出的工作区完整复跑（已有结果会拒绝覆盖；这些已公开种子不能再冒充新留出集）：

```sh
python -c "from pathlib import Path; Path('runs/news-swift-20261019').mkdir(parents=True, exist_ok=True)"
python tools/arena_gate.py --cwd . -- python tools/news_swift_study.py build
python tools/news_swift_study.py prepare
python tools/arena_gate.py --cwd . -- python tools/news_swift_study.py run
python tools/arena_gate.py --cwd . -- python tools/news_swift_report.py
python tools/news_swift_writeup.py
```
""")
(ROOT/"docs/swift-mechanism.md").write_text("".join(m),encoding="utf-8")
print("Wrote docs/news-v2.md and docs/swift-mechanism.md")
