# Swift: price-only v5 challenger

## 验收结果（2026-10-18）

**Swift 已按现有门槛通过 v5。** 使用盘口、主动成交方向和短期价格动量；
不读取估值，也没有改基准、对手、初始资金或调度规则。

| 数据集 | 局数 | 平均排名百分位 | 进入前 25% | 第一名次数 |
| --- | ---: | ---: | ---: | ---: |
| 初始开发 1–20 | 20 | 95.84 | 100% | 5 |
| 扩展开发 21–60 | 40 | 95.32 | 100% | 7 |
| 官方旧留出 1001–1020 | 20 | 97.05 | 100% | 6 |
| 新留出 A：5001–5040 | 40 | 95.89 | 100% | 10 |
| 新留出 B：6001–6040 | 40 | 96.76 | 100% | 12 |

两批新世界合计平均 **96.32**，最差排名百分位 **79.66**。游戏账户的平均
收益率为 **118.54%**，最差一局 **+6.10%**；正常到账的分红仍计入收益，
但不进入交易信号。新世界中，扣掉分红后的净损益平均仍为 **+19,259.69 元**
（已含交易费、闲置费与利息，包含期初持股到期末资产的价值变化）。

整个开发只实现并检验了一组参数，没有网格搜索。冻结提交为 `5289b76`，
其后 100 局验收源码与二进制哈希完全一致。100 局共完成 **600 次账务审计**，
所有账户零跳过、零停用、零 bot 错误；Swift 零拒单、零强平，每局完整决策
3,000 次，平均每次约 **1.5–1.8 微秒**。

格式检查、Clippy 严格检查、release 构建及全部 **34 项测试**通过。逐局分数、
收益、诊断和哈希在 [swift-v5-results.json](swift-v5-results.json)，汇总脚本为
`python tools/swift_report.py`。原始运行记录保存在本机 `runs/bench/swift-*`。

小资金利用的是最优一档的成交机会：只在当前最优价成交，未成交部分等待下一次
观察，不为迅速扩大仓位扫到更差价格。本轮保持标准挑战账户（现金 20,000 元、
每只股票 20 股）。上述验收之后完成的 [780 局跨市场压力测试](swift-stress.md)
补充了容量、拥挤和退出规则对照：本金扩大或同策略拥挤会明显削弱甚至反转优势，
因此这里的 v5 验收成绩不能视作跨市场通用结论。

## Scope and frozen evaluation protocol

Swift uses the public order book, executed trades, recent prices, trading clock,
and its own account limits. It does **not** read dividends, news, private terminal
hints, stock loadings, combo rules, other accounts, or a world seed. Its signal
does not call Coral, Knowledge, hold_value, or any fundamental-value function.
Dividends still accrue under normal game rules; receiving income is different
from using it as a forecast input. Equity is used only to size positions.

The unchanged v5 challenge uses cash 2,000,000 cents and 20 shares of each stock,
3,000 ticks, and four threads. No starting-capital discount, altered opponent
population, fee waiver, scoring change, or skipped-decision advantage is used.
The existing shared idle-fee accounting fix remains in the engine.

The pass criteria are mean rank percentile >= 90 and at least 80% of worlds
finishing in the top quartile. Percentiles rank final/initial equity.

Development used historical public prices/books/trades from baseline seeds 1–20.
The first implemented strategy passed live challenge seeds 1–20 (95.840 mean)
and then seeds 21–60 (95.324 mean), without parameter changes or a parameter grid.
After these runs the unused 100-tick accumulator was removed and regression
tests were added; the trading equations and parameters were unchanged.

Before opening validation results, the following evaluation was fixed:

| Set | Seeds | Purpose |
| --- | --- | --- |
| Official legacy | 1001–1020 | Existing v5 gate; these worlds were used by earlier bots |
| Fresh A | 5001–5040 | New-world validation, no tuning afterward |
| Fresh B | 6001–6040 | Independent second batch, same frozen executable |

Raw challenge reports, commands, source hashes and executable hashes are kept
under `runs/bench/swift-*`; the permanent summary is `swift-v5-results.json`.
Validation worlds must not be used to revise this candidate and
then relabeled as untouched holdout data.

## Mechanism

Per stock, maintain exponentially decayed signed trade volume over 10 and 30
ticks and a trailing 30-tick midprice average. The score is:

```
0.7 * flow10 + 0.3 * flow30 + 0.3 * depth_imbalance + 5 * momentum30
```

Flow is (aggressive buy quantity - aggressive sell quantity) / total quantity.
Depth imbalance is (bid quantity - ask quantity) / total displayed quantity,
using visible levels within 2% of mid. Momentum is mid/EMA30 - 1, clipped to
[-0.1, 0.1]. All observed tape trades are included, including the bot's own trades
when present in the public tape; no participant identities are available to it.

After 30 ticks, score > 0.15 with sufficient recent volume enables a position
target of 35% of equity in that stock. Score < -0.3 disables that target. The gap
between these thresholds reduces spread-paying churn. Overall target leverage
is capped at 1.5, with one shared fee-inclusive buying budget across stocks.

Orders are IOC limits at the current best price; they do not sweep worse prices.
Buys require spread <= 1.2%. Unfilled quantity waits for another observation.
The bot starts exiting 20 ticks before the end; empty bids can prevent a complete
exit. Small capital helps because its desired trades can fit in top-level depth.
There is no short selling or hidden terminal-value prediction.

## Checks

- A multi-tick metamorphic test changes fundamental metadata, seed, news,
  dividends, combo income, season and terminal hints while keeping permitted
  market/account inputs fixed, and checks identical actual buy/sell orders.
- Execution tests cover aggregate buying power, sellable shares, end-of-game
  exits, absent asks and excessive spreads.

Reproduce a challenge after `cargo build --release`:

```
python tools/bench_bot.py swift-replay --bot swift --first-seed 5001 --seeds 40
```

The multithreaded simulator is not bit-for-bit reproducible; seed fixes the world,
but OS scheduling can change matching paths. A rerun need not have identical ranks.
