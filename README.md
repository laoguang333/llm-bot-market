# llm-bot-market

当前默认为 [v18：分散 Grid 初始资本](docs/baseline-v18.md)，共 105–229 个账户、19 种策略。Almanac 与 Osprey v2 各一个普通账户，现金 20,000 元、每股 20 股、消息倍率 1、无额外研究；Lattice 继续保留。Grid 保留简单网格逻辑和人数，实际开局降到普通账户规模；多出的现金、每只股票按 Coral、Value、Hunter、Momentum、Shark 各 20% 分配，逐分逐股守恒。沿用 v15 的全部市场规则：10000 ticks，大龙从 tick 3333 起允许与其他候选分化，成长高点仍在后段，分红约十倍不保证股价十倍；指定成长者概率 50%，期末兑现期限约为整局 1/6–5/6。保留 Tide、Kestrel、两个 Swift、NewsV2、ProfitEffect 群体及普通权限十倍资金 Titan；Horizon v3 资金与 Titan 相同，额外研究最大提前 1000 ticks。[Horizon v3 研究](docs/horizon-v3.md)保留其在旧 v11 背景中的历史对照。

[v16 生态调查](docs/ecology-v16.md)保留升级前的 80 局评估及全部 35 个注册策略的评价。v16 基准 20 个世界各重复两次；Almanac、Osprey 各另做 20 局普通挑战，之后按用户要求加入 v17。

[Coral 与 Grid 逐笔分析](docs/grid-coral-v16.md)进一步区分主动/被动成交、同类互成交、含息成交后表现及库存损失，避免把成交量直接当作流动性贡献或把 Grid 的亏损全归于抄底。

[五类支付群体改造](docs/payer-v19.md)新增公开新闻过度反应者 `news_reactor`、价格反应迟到者 `latecomer` 和库存受限网格 `grid_v2`。候选为 `scenarios/payer-v19-balanced.toml` / `payer-v19-social.toml`；第五轮五类亏损份额达到 67%–70%，但质量与盘口深度未通过保护条件，暂未晋级默认。300 局开发及完整失败记录见报告，独立留出尚未使用。

[v20 分红风险实验](docs/dividend-v20.md)已实现长期减息、持续停息与不确定恢复，只通过分红及消息区间表达。世界形态检查通过，但开发组市场质量与深度下降，暂不替换默认；现有生产 bot 保持不变。

[v21 生态实验](docs/ecology-v21.md)完成 Tide 消融、分红压力校准与 Harbor / Relay 群体竞争，共 136 局。中等风险路径通过校准，但两种新策略未改善生态，独立验证失败，默认继续保留 v18。新增按分红状态区分的卖盘覆盖与股数深度诊断，并核对闲置费造成的现金损耗。

[Swift 退化诊断](docs/swift-v21-diagnosis.md)复核 v21 的 88 局并新增 68 局逐笔记录/参数消融：Coral 的成交服务未被 Relay 接替，短线优势减弱，standard 持续调仓进一步放大损失。生产 Swift 和市场规则未修改。

[v22 市场功能与基础生态研究](docs/ecology-v22.md)增加分状态流动性、消息响应及群体持续性报告，独立比较收费与基础阵容，再检验 Coral 杠杆和 Noise 单笔规模。较低收费没有使成交和消息响应消失，但群体风险与亏损集中仍须处理；实验未晋级默认，强 bot 回填受基础生态验收约束。完整协议、失败及追加对照均保留。

`baseline.toml` 与 `baseline-v18.toml` 相同；[v17](docs/baseline-v17.md)、[v16](docs/baseline-v16.md)、[v15](docs/baseline-v15.md)、[v14](docs/baseline-v14.md)、[v13](docs/baseline-v13.md)、[v12](docs/baseline-v12.md)、[v11](docs/baseline-v11.md)、[v10](docs/baseline-v10.md)、[v9](docs/baseline-v9.md)、[v8 长局](docs/baseline-v8.md)、3000 ticks 的 [v7](docs/baseline-v7.md)、旧八节点模型和 v6 场景均保留。普通股息、组合收益仍逐 tick 派发，闲置费仍为每 50 ticks 平均权益的 0.6%。Astra/Falcon 不加入。[交易调查台](docs/swift-debugger.md)支持整轮操作、FIFO 连线、消息收报到生效的交易关联，以及[强平与坏账账本](docs/liquidation-audit.md)；旧录制保持原貌。

新挑战者 [Almanac](docs/almanac.md) 以普通账户打穿 v15：正式留出 1001–1020 平均百分位 99.57、20/20 进前 25%。它按实测的消息消化曲线买入价格尚未消化的部分，并按比赛阶段调整基础仓位；现已作为普通账户加入 v17。

[Lattice](docs/lattice.md) 已加入 v16 基准。它此前作为普通挑战者通过 v15：冻结后的传统验收 1001–1020 平均百分位 **91.95**、20/20 进前 25%；本轮预留 73001–73020 为 **90.13**、19/20，按项目门槛通过但余量有限。这些成绩属于 v15，复现脚本已绑定归档场景。独立实现，未阅读旧 bot 策略代码。

[Tide](docs/strategies.md#tide-打穿-v132026-10-20) 以普通挑战者账户打穿 v13：正式留出 1001–1020 平均百分位 96.63、20/20 进前 25%。它按"跌下去的股息会回升"估值，买价格低于这一估值的股票，并在闲置费结算前用同 tick 卖出买回补足成交额抵扣。

[Titan 十倍资金挑战](docs/titan-v6.md)记录此前普通权限未通过、增强提前消息概率后通过的研究；新基准采用其普通权限版本，不沿用消息优势，也不移植旧排名成绩。

一个连续限价盘口的股票市场，让多个 LLM 各自写的交易 bot 在里面对抗，只比谁赚得多（按收益率的排名百分位计分）。完整规则见 [PRD.md](PRD.md)，内置策略的对比结果见 [docs/strategies.md](docs/strategies.md)。每个账户是一个独立的 owner；场景里写了 `owner` 的那组只能有一个账户。

[当前市场设定](docs/market-settings-current.md)按实际代码说明种子差异、三层新闻、平静/动荡切换、普通股息、固定组合奖励和随机期末兑现；其中初始股息与组合奖金不随种子改变，未来股息路径与隐藏终值倍数会改变。

[大龙价格与局长调查](docs/dragon-duration.md)比较 1 万、2 万、4 万 ticks，并用相同世界的有龙／仅候选成长对照研究分红、价格倍数、流动性和全市场收益；实验配置独立保存，默认局长保持 1 万 ticks。

## 快速开始

强势股候选 [Osprey](docs/osprey-v2.md) 已实现为 `osprey`：在 Kestrel 执行与风控上加入股息和价格趋势确认、持仓容忍及回撤退出。现已作为普通账户加入 v17；原版保留为 `osprey_v1`，[旧报告](docs/osprey.md)的结论仍只属于旧版和 v14。

新增普通账户挑战者 [Kestrel](docs/kestrel.md)：把已收到的新闻估值与成交方向结合，按价差和费用调整仓位，并控制期末负债。默认现金 2 万元、每股 20 股，普通消息权限、无额外研究；冻结后的 v13 留出验收与复现命令见报告。

新研究：[赚钱／亏钱效应群体](docs/profit-effect-bots.md)。`profit_effect` 参考组内和自身近期实际盈亏、简单价格变化；部分成员另有[追市场领涨股的权重](docs/profit-effect-leaders.md)，不读估值或新闻。该策略已纳入 v13，独立场景 `scenarios/profit-effect-v12.toml` 和 80 局研究结果继续保留；活跃度提高不代表更赚钱。

```sh
cargo build --release
# 跑一局，输出 summary.json、成交/K 线 CSV 和离线 report.html
./target/release/llm-market run scenarios/tournament.toml --out runs/t1
# 在 20 个随机世界里各跑一局，输出每个策略的平均排名百分位 ± 标准差
./target/release/llm-market series scenarios/tournament.toml --out runs/series --seeds 20
# 当前默认：v18，保留原有策略与规则，分散开局资本
./target/release/llm-market series scenarios/baseline.toml --out runs/base --seeds 20
# 打穿测试：一个挑战者账户加入基准市场，在留出种子 1001–1020 上跑
./target/release/llm-market challenge scenarios/baseline.toml --bot shark --out runs/ch-shark --seeds 20
# Swift：只读盘口、成交与价格走势的挑战者
./target/release/llm-market challenge scenarios/baseline.toml --bot swift --out runs/ch-swift --seeds 20
# 复现 Coral 打穿 v4 的挑战（v5 已将 Coral 纳入对手阵容）
./target/release/llm-market challenge scenarios/baseline-v4.toml --bot coral --out runs/ch-coral-v4 --seeds 20
# 三名 Astra 独立策略的统一留出验收和六种出场顺序对抗
python tools/arena_gate.py --cwd . -- python tools/astra_tournament.py --out runs/astra-v5-tournament
# 列出内置 bot
./target/release/llm-market bots
# 录制完整决策输入和成交，再打开交互调查台（自动构建 release）
python tools/swift_debugger.py run --out runs/swift-debug --seed 7 --threads 4
python tools/swift_debugger.py serve --run runs/swift-debug --open
```

## 目录

| 路径 | 内容 |
|---|---|
| `crates/bot-api` | bot 与引擎之间的全部类型：`Rules`、`Observation`、`Action`、`Bot` trait |
| `crates/market` | 撮合、调度、结算、世界生成、计分与报告 |
| `crates/bots` | 内置 bot：基础对照、估值策略，以及 `swift.rs` 中只读价格和成交的策略 |
| `crates/cli` | 命令行 `llm-market` |
| `scenarios/` | `baseline.toml` / `baseline-v18.toml`（当前）；v17、v16、v15、v14、v13、v12、v11、v10、v9、v8、v7、v6 及更早版本（归档）；`horizon-v11.toml` 保留上轮研究阵容 |
| `tools/debugger/` | 离线交互交易调查台；[使用方法、数据口径与验证](docs/swift-debugger.md) |

## 写一个 bot

实现 `bot_api::Bot`：

```rust
pub trait Bot: Send {
    fn on_tick(&mut self, obs: &Observation) -> Vec<Action>;
    fn on_end(&mut self, _result: &FinalResult) {}
}
```

开局通过 `Start` 拿到全部公开规则（`Rules`）、自己的初始现金和持股、场景里给的参数。每个 tick 收到盘口、成交、自己的挂单和成交回报、收到的新闻、收入明细和闲置费预估，返回动作列表（限价 GTC/IOC、市价、撤单、全撤）。在 `crates/bots/src/lib.rs` 的 `factory` 里注册名字后即可在场景 TOML 里使用。

WASM 沙箱（让外部 LLM 提交编译好的 bot）还没做，见 PRD 第 7 节。

当前默认普通股息与组合收益均每个个人 tick 发放（tick 0 除外）。`Rules.dividend_scale = 50`，股票股息和新闻变化的整数单位都是 **1/50 分**；先乘账户在该股票的持仓，再四舍五入到分，不累计余数。新闻公开不会提前改变派息，仍等 `effective_tick`。组合独立使用 `combo_scale=50`，按每条规则的整仓套数合计后对称取整，负组合也逐 tick 扣款。终值提示的每单位仍代表 50 tick。当前闲置费为每 50 ticks 平均权益的 **0.6%**，合格成交额的 1% 抵扣不变。旧场景省略新字段时保持原规则；改造前扩容场景为 `baseline-v6-periodic-combos.toml`。设计、性能和市场影响见[普通派息报告](docs/per-tick-dividends.md)及[组合与费用改造报告](docs/per-tick-combos.md)。

当前运行的所有 bot 都先经过代码审查。`Start.seed` 与世界种子的关系有意保留，不作为安全边界；审查时禁止策略利用这一关系重建隐藏新闻或终值。这种可推导性本身是已接受的设计。

同一策略可以分成多个未指定 `owner` 的参数组，自动名称按各组最大人数预留连续编号，因此不会重名，也不会随其他组的随机人数漂移。显式 `owner` 与这些自动名称冲突时，场景校验会报错。

## 调度容忍度

Windows 环境只使用可用逻辑核心的一半（向下取整，至少 1 个）。引擎会将自动及显式指定的工作线程数限制在此上限内；Linux 没有半核限制。`--threads 0` / `schedule.threads = 0` 自动选择平台允许的线程数，且不会超过账户数。构建、测试和并发实验也应共享相同的 Windows CPU 预算；引擎的线程限制不会为多个独立进程分配总预算。

当前基准场景配置为 4 个工作线程，可用 `--threads 0` 启用自动线程数。正常策略即使暂时落后也保留决策机会：只有最近 8 次实际决策中至少 6 次超过 `schedule.decision_budget_us = 2000` 微秒，并且落后达到 `skip_decision_lag`，才跳过决策。计时只包括 bot 的 `on_tick`。快账户仍受 `max_tick_skew` 约束，等待较慢的账户。

`skip_decision_lag = 0` 禁用跳过；`decision_budget_us = 0` 可复现旧的仅按落后量跳过规则。新规则不是原生代码的硬超时。每局 `summary.json` 记录线程数、调度门槛和各账户的超时/跳过次数；旧调度下的历史分数应单独比较。

多线程执行顺序不确定，同一个 seed 不保证逐笔复现；本项目优先考虑性能。性能优化及实测见 [性能记录](docs/performance.md)。

Windows 上可用 `python tools/bench_bot.py coral-train --first-seed 1` 跑当前基准开发集；追加 `--scenario scenarios/baseline-v4.toml` 可选择旧版。脚本保存完整报告、命令、源码及二进制哈希和汇总诊断到 `runs/bench/`。

三名 Astra medium 独立开发的 `astra_a`、`astra_b`、`astra_c` 已注册。统一赛制、冻结记录、账务修复和比赛结果见 [三方 v5 比赛](docs/astra-v5-tournament.md)。

`swift` 使用主动成交方向、盘口深度和短期价格动量，完全不读取分红、新闻或清算估值。开发过程、冻结验收和复现命令见 [Swift v5 报告](docs/swift-v5.md)。

固定 Swift 的后续 [跨市场压力测试](docs/swift-stress.md) 共 780 局，覆盖消息传播、波动、对手、手续费、资金容量和决策频率，并单独检验拥挤效应。原版 v5 通过复核，但小资金优势不能外推到大资金或大量同策略竞争者。

新增 [News 修复与增强版](docs/news-v2.md)：`news_fixed` 补全事件退出，`news_v2` 将新闻方向与 Swift 信号、执行结合，已在 v6 替换旧 News，v7 继续保留。在此前周期派息的研究中，每组 40 局，增强版在当时 v5/v6 平均收益 +151.24%/+21.24%，同批 Swift +136.91%/+11.02%；增强版未打穿当时的 v6。旧实现与历史配置保留，这些成绩不能作为新阵容的表现。[920 局机制研究](docs/swift-mechanism.md)发现，单纯增加随机成交活跃度不足以恢复新闻环境下的高收益；真实成交方向有用，已出现大阳线不是入场必要条件，分红也贡献了大量收益。

**机制审计补充：** 旧周期派息的 160 局轨迹中，15 局的普通派息超过按统一股权登记计算的全市场上限，[详情及逐局证据](docs/swift-mechanism.md)已归档。当前逐 tick 方案按用户选择保留个人时钟和邮箱结算，接受转手期间少量多派/少派，不增加统一登记；这不是严格股权归属修复。上述收益均是旧机制记录，新机制需要单独验证。

[Swift 策略特性与研究档案](docs/swift-research.md) 是后续研究的统一入口，集中记录信号、执行、容量、失败条件和冻结证据。[单一狩猎者实验](docs/swift-hunt-results.md) 使用一个十倍资金的 Falcon 与一个固定 Swift 对抗，分别比较目标受损和对手是否相对胜出。Falcon 的实验模式通过参数指定，relay 为 `mode=6`，定时买卖对照为 `mode=7`。

[资金优势实验](docs/swift-capital-results.md) 已完成矩阵、独立确认与[现金/库存机制核验](docs/swift-capital-cash-results.md)，共 2,140 局。大本金、大初始库存的 relay 往往能百分比少亏；只多给现金的证据较弱，猎手本身仍亏损，且会恶化盘口流动性。

[猎手研究的重要发现](docs/swift-hunter-findings.md) 已归档。多 Swift 实验按用户要求中止，500 局成组结果见[中止报告](docs/swift-crowding-hunt-results.md)；另有 8 个未完成配置的原始摘要，不纳入统计。实验不会自动续跑。

[周期派息 v6](docs/baseline-v6.md) 加入五个不同参数的小资金 Swift，现保存在 `baseline-v6-five-swift.toml`；当前默认已改为两个 Swift、逐 tick 普通股息。旧研究脚本已绑定归档。在当时引擎的 80 局固定验收中，默认 Swift 挑战 v5/v6 的平均百分位分别为 **96.51 / 60.21**，v6 未打穿。该历史结果不能当作当前基准的成绩；派息改造见[改造报告](docs/per-tick-dividends.md)，历史阵容见[两个 Swift 的 v6](docs/baseline-v6-current.md)，当前见 [v18](docs/baseline-v18.md)。

## 开发

```sh
cargo fmt && cargo clippy --all-targets -- -D warnings && cargo test
```

release 构建关闭整数溢出检查；`cargo build --profile release-checked` 打开。

release 已启用 ThinLTO 和单个 codegen unit，以增加跨模块优化机会；首次 release 构建会更慢。用 `python tools/build_release.py` 构建时，Windows 的 Cargo、LLVM 和链接器子进程会共享半核 CPU 预算。追加 `--native` 可为本机 CPU 指令集优化，产物单独保存在 `target/native/release/`，仅适合本机或指令集兼容的 CPU；普通 `target/release/` 构建不启用此本机指令集限定。

Windows 下用 `cargo build --release -j N` 限制构建并发，`N` 不超过逻辑核心数的一半；测试建议 `cargo test -j N -- --test-threads=1`，避免多个内含工作线程的测试同时运行。例如 20 个逻辑核心的机器取 `N=10`。Linux 无半核限制。

比较两个 release 可执行文件的性能（交替顺序、预热、保留完整报告和哈希）：

```sh
python tools/perf_engine.py --binary before=runs/llm-market-before.exe --binary after=target/release/llm-market.exe --threads 1 4 10 --repeats 3 --out runs/perf-comparison
```
