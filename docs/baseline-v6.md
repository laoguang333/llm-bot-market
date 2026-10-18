# 正式基准 v6（2026-10-19）

后续更新：当前 v6 已缩减为标准版和较早进退版两个 Swift，且使用逐 tick 股息；本文实验中的五 Swift 配置另行归档，历史成绩不代表缩减后的基准。见 [当前基准](baseline-v6-current.md)。

**这是周期派息 v6 的历史验收报告。** 验收时它曾成为默认基准；随后普通股息改为逐 tick 派发，当前 [baseline.toml](../scenarios/baseline.toml) 与 [baseline-per-tick.toml](../scenarios/baseline-per-tick.toml) 相同，沿用这里的五个 Swift 阵容。原周期规则保存在 [baseline-v6-five-swift.toml](../scenarios/baseline-v6-five-swift.toml)，以下成绩不代表新规则。新机制、测速和对照见[逐 tick 派息报告](per-tick-dividends.md)。

v6 在 v5 原有人群中增加五个不同参数的小资金 Swift，要求新策略面对同类抢单竞争。原有背景策略、人数范围、股票、费用、观察和调度规则均未因本次升级改变。每个 Swift 为现金 20,000 元、每标的 20 股、独立 owner；挑战时会再追加一个候选账户，因此 `--bot swift` 的 v6 挑战共有六个 Swift。

参数在本轮运行前已确定，没有根据验收结果调整。原版 Swift 源码及单独挑战默认参数保持冻结。猎手研究继续保持中止；本轮只是基准验收。

## 五个参数档

| 账户 | 入场阈值 | 退出阈值 | 盘口权重 | 动量权重 | 单股权益比例 | 总杠杆目标 | 买入价差上限 |
|---|---:|---:|---:|---:|---:|---:|---:|
| swift-standard | 0.15 | −0.30 | 0.30 | 5.0 | 35% | 1.5 | 1.2% |
| swift-early | 0.05 | −0.10 | 0.20 | 3.0 | 25% | 1.0 | 1.0% |
| swift-selective | 0.30 | −0.15 | 0.25 | 4.0 | 25% | 1.0 | 0.8% |
| swift-flow | 0.18 | −0.25 | 0.00 | 0.0 | 30% | 1.2 | 1.2% |
| swift-depth | 0.20 | −0.35 | 0.60 | 2.0 | 30% | 1.2 | 1.0% |

依次覆盖默认、较早进退、严格入场、仅成交方向信号、较重盘口信号。它们共享 10/30 tick 成交窗口、最优价 IOC、真实资金/持仓限制和末期退出，不能称为五条完全独立的策略路线。flow 档的信号只用成交方向，但执行和仓位计算仍需要盘口价格。

这项改动来自[猎手研究的重要发现](swift-hunter-findings.md)：小资金策略的优势受同类竞争和流动性反馈影响，不能只在缺少同类抢单者的市场里验收。目标收益受损、猎手百分比少亏、猎手盈利仍是三个不同问题。

## 固定验收与默认 Swift 挑战

在读取收益前固定了[验收方案](baseline-v6-protocol.md)，并完成[策略审查](baseline-v6-bot-review.md)。四组各用种子 **20001–20020**，共 **80 局**；每配置/种子只跑一次，不挑选重复结果。每局 3000 tick、4 个工作线程，使用同一个普通 release 二进制。

本机为 Windows、20 个逻辑核心，总预算 10。构建、测试和比赛的子进程共享 10 核亲和性；四批比赛通过 arena_gate 串行运行。没有降低 bot 决策频率。

v5 与 v6 使用同一份当前工作区源码，包含用户在其他位置完成的终末估值边界、滚动标记价格、闲置费预留保护、强平和账户命名修正。因此下表是本次重跑的对照，不与旧引擎的历史分数直接相减。

| 默认 Swift 挑战 | 平均排名百分位 ± 标准差 | 进入前 25% | 平均收益率 | 收益率中位数 | 亏损局数 | 打穿 |
|---|---:|---:|---:|---:|---:|---|
| v5 | 96.51 ± 2.47 | 20/20 | +133.01% | +117.36% | 0/20 | 是 |
| v6 | 60.21 ± 18.78 | 4/20 | +17.65% | +13.22% | 5/20 | 否 |

打穿条件不变：平均排名百分位 ≥ 90，且至少 80% 的世界进入前 25%。默认 Swift 在当前 v5 仍通过，在当前 v6 未通过；这说明本配置下的同类竞争已形成明显障碍，不证明所有新策略都无法打穿 v6。

## 五个账户的独立表现

左侧为 v6 背景 20 局；右侧为再加入默认 Swift 挑战者后的另外 20 局。收益率按每个账户期末权益/初始权益计算，已包含费用、分红和终末结算。各档始终单独汇总，不以策略名 `swift` 的合并分数代替。

| 账户 | 背景平均百分位 | 背景平均收益 | 背景收益中位数 | 背景亏损局 | 加挑战者后平均百分位 | 加挑战者后平均收益 | 加挑战者后亏损局 |
|---|---:|---:|---:|---:|---:|---:|---:|
| swift-standard | 84.75 | +64.44% | +60.50% | 1/20 | 86.47 | +60.79% | 1/20 |
| swift-early | 83.42 | +47.64% | +48.61% | 0/20 | 85.36 | +50.02% | 0/20 |
| swift-selective | 72.11 | +25.49% | +22.36% | 2/20 | 72.80 | +27.70% | 1/20 |
| swift-flow | 68.61 | +22.01% | +18.37% | 3/20 | 64.18 | +20.12% | 6/20 |
| swift-depth | 68.19 | +22.80% | +22.09% | 4/20 | 57.66 | +13.84% | 7/20 |

**位置与成交路径限制：** v6 挑战组里，`swift-standard` 与 `challenger` 的代码、参数和初始资产相同，但平均百分位分别为 86.47 和 60.21，收益分别为 +60.79% 和 +17.65%。它们的账户位置不同；调度器在同一已完成 tick 下按账户索引领取任务，并发实际成交路径也不确定。此次没有交换位置或重复抽样，不能把档位之间的差异全部归因于参数，也没有量化位置效应占比。后续比较策略强弱应另外做位置轮换；本次保留现行追加挑战者的验收口径，不修改撮合或调度。

## 市场质量与执行完整性

| 批次 | 九项质量全部通过 | 引擎累计墙钟 | 命令墙钟（含报告） |
|---|---:|---:|---:|
| v5-background | 16/20 | 8.44 s | 9.24 s |
| v6-background | 17/20 | 10.43 s | 11.47 s |
| v5-swift-challenge | 16/20 | 7.58 s | 8.32 s |
| v6-swift-challenge | 19/20 | 8.29 s | 9.12 s |

四组未通过的世界都只差 `value_crossings`（价值穿越次数至少 2）。没有放宽质量阈值。v6 背景通过率为 85%，当前 v5 为 80%；每组只有 20 局，不能据此认定质量显著改善。

以下为背景市场各指标在 20 个世界中的中位数，括号是达标局数：

| 指标 | 原门槛 | v5 | v6 |
|---|---:|---:|---:|
| `trade_coverage` | ≥ 0.9 | 0.985 (20/20) | 0.993333 (20/20) |
| `max_gap_ticks` | ≤ 200 | 36.25 (20/20) | 31 (20/20) |
| `two_sided_share` | ≥ 0.95 | 0.996667 (20/20) | 0.996667 (20/20) |
| `median_spread` | ≤ 0.02 | 0.00269237 (20/20) | 0.00357901 (20/20) |
| `depth_2pct_in_trades` | ≥ 3 | 12.1628 (20/20) | 8.88641 (20/20) |
| `median_abs_log_dev` | ≤ 0.25 | 0.124408 (20/20) | 0.128757 (20/20) |
| `share_beyond_2x` | ≤ 0.05 | 0 (20/20) | 0 (20/20) |
| `value_crossings` | ≥ 2 | 3 (16/20) | 4 (17/20) |
| `abs_return_autocorr` | ≥ 0.05 | 0.298165 (20/20) | 0.244798 (20/20) |

v6 的价差中位数由约 0.269% 增至 0.358%，两侧 2% 深度相对典型成交量由 12.16 降至 8.89；竞争消耗了部分最优档流动性，但这两项以及双边盘口覆盖率在全部世界仍达标。原始指标、失败世界和逐账户结果完整保留，不能描述为全部市场健康。

80 局共 4,756 个账户局、14,268,000 次决策；每个账户每局恰好 3000 次，**零跳过、零 bot 错误、零停用**，480 次现金/股份守恒审计全部通过。五档 Swift 及挑战者均零拒单、零强平。所有账户期末递延闲置费均为零。

全市场仍有策略请求被规则拒绝：四组依次为 267,596、260,535、270,590、262,442 次，主要来自 grid/noise；拒单不等于 bot 崩溃，不能把“Swift 零拒单”写成“全部账户零拒单”。四组全市场均无强平。费用、拒单、成交量和持仓见逐账户 JSON。

墙钟只是本轮单次运行记录，不是性能提升实验。多线程比赛不保证同种子逐笔复现；没有使用当前分数修改 Swift 参数或选择种子。

## 代码兼容与复现

本次基于现有工作区完成验收，没有覆盖用户尚未提交的引擎和 bot 修改。正式升级的行为变化仅来自加入五个 Swift 竞争账户；没有改动观察频率、时间差上限、决策预算、撮合、结算或 Swift 源码。

历史 `astra_tournament.py`、`swift_report.py`、`swift_stress.py`、`swift_hunt.py`、`swift_capital.py`、`swift_crowding_check.py`、`swift_crowding_hunt.py` 及 hunt-eval 的两处 v5 测试夹具均显式使用 `baseline-v5.toml`。猎手中止标记和防续跑逻辑保留。通用 `bench_bot.py`、`perf_engine.py` 和默认 CLI 场景使用者跟随 v6。

运行普通基准或挑战：

```sh
python tools/build_release.py
./target/release/llm-market series scenarios/baseline.toml --out runs/v6-new --seeds 20 --first-seed 20001 --threads 4
./target/release/llm-market challenge scenarios/baseline.toml --bot swift --out runs/swift-v6-new --seeds 20 --first-seed 20001 --threads 4
# 旧 v5 对照必须明确指定归档配置
./target/release/llm-market challenge scenarios/baseline-v5.toml --bot swift --out runs/swift-v5-new --seeds 20 --first-seed 20001 --threads 4
```

整套审查后验收流程（先选新的输出目录；report 会重写仓库内 v6 结果 JSON，保留旧报告后再复跑）：

```sh
python tools/arena_gate.py --cwd . -- python tools/validate_baseline_v6.py checks --out runs/v6-repeat
python tools/validate_baseline_v6.py prepare --out runs/v6-repeat
python tools/arena_gate.py --cwd . -- python tools/validate_baseline_v6.py run --out runs/v6-repeat
python tools/validate_baseline_v6.py report --out runs/v6-repeat
```

已通过 `cargo fmt --all -- --check`、Clippy 全工作区/全目标严格检查、71 项 Rust 测试、11 项 Python 测试及 release 全二进制构建。默认切换后的测试另行记录在 [升级记录](baseline-v6-promotion.json)。现有测试覆盖价格/时间优先级、消息交付、现金/股份守恒、待结算预留、终末边界、无成交时标记价格和账户命名。

证据包括[冻结计划与源码哈希](baseline-v6-plan.json)、[80 局完整结果](baseline-v6-results.json)和[升级记录](baseline-v6-promotion.json)。原始日志、每局 CSV/HTML、源码 ZIP、原始工作区差异及冻结二进制保存在本机 `runs/baseline-v6-20261019/`。Git HEAD 为 `ef2eba7`，但该提交号单独不足以标识本次构建，因为运行包含用户已有的未提交修改；应以冻结清单和源码快照为准。

验收时 v6 文件头仍标为 candidate。验收后只修改了三行状态注释，再将相同字节复制为默认配置；解析后的 TOML 完全一致，前后 SHA-256 均记录在升级记录。v5 归档未变，Swift 源码 SHA-256 仍为 `bfdbf02ec196234a3346b1f046e5f3b6080c9c9ed511bc1382f66e5d62ccf9fb`。
