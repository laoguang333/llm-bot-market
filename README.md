# llm-bot-market

当前默认为[扩容后的 v6](docs/baseline-v6-current.md)：98–222 个账户，seed 7 为 146 个；保留标准 Swift 与较早进退的单次入场 Swift，后者现金 2,000 元、每股初始 2 股，同一股票持仓后不再补仓。普通股息逐 tick 派发。`baseline.toml`、`baseline-v6.toml`、`baseline-per-tick.toml` 内容相同；扩容前配置归档到 `baseline-v6-pre-expansion.toml`，五 Swift 的旧配置分别保存在 `baseline-v6-five-swift.toml`（周期派息）、`baseline-per-tick-five-swift.toml`（逐 tick）。[扩容报告](docs/v6-population.md)记录流动性、涨幅和性能变化；历史成绩不代表当前基准。[交易调查台](docs/swift-debugger.md) 支持整轮操作与 FIFO 逐笔进出场连线。

一个连续限价盘口的股票市场，让多个 LLM 各自写的交易 bot 在里面对抗，只比谁赚得多（按收益率的排名百分位计分）。完整规则见 [PRD.md](PRD.md)，内置策略的对比结果见 [docs/strategies.md](docs/strategies.md)。每个账户是一个独立的 owner；场景里写了 `owner` 的那组只能有一个账户。

[当前市场设定](docs/market-settings-current.md)按实际代码说明种子差异、三层新闻、平静/动荡切换、普通股息、固定组合奖励和随机期末兑现；其中初始股息与组合奖金不随种子改变，未来股息路径与隐藏终值倍数会改变。

## 快速开始

```sh
cargo build --release
# 跑一局，输出 summary.json、成交/K 线 CSV 和离线 report.html
./target/release/llm-market run scenarios/tournament.toml --out runs/t1
# 在 20 个随机世界里各跑一局，输出每个策略的平均排名百分位 ± 标准差
./target/release/llm-market series scenarios/tournament.toml --out runs/series --seeds 20
# 当前默认：两个 Swift 的 v6、逐 tick 普通股息
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
| `scenarios/` | `baseline.toml` / `baseline-v6.toml` / `baseline-per-tick.toml`（当前两个 Swift、逐 tick 派息）；`baseline-v1` 至 `baseline-v5` 及 `baseline-v6-five-swift`（周期派息归档） |
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

当前默认普通股息每个个人 tick 发放（tick 0 除外）。`Rules.dividend_scale = 50`，股票股息和新闻变化的整数单位都是 **1/50 分**；先乘账户在该股票的持仓，再四舍五入到分，不累计余数。新闻公开不会提前改变派息，仍等 `effective_tick`。组合奖励继续每 50 tick，终值提示的每单位也仍代表 50 tick。`summary.json` 记录这些单位，`bars.csv` 另提供 `dividend_cents_per_payment`。旧场景省略新字段时保持原规则。设计、性能和市场影响见[逐 tick 派息报告](docs/per-tick-dividends.md)。

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

新增 [News 修复与增强版](docs/news-v2.md)：`news_fixed` 补全事件退出，`news_v2` 将新闻方向与 Swift 信号、执行结合。每组 40 局，增强版在 v5/v6 平均收益 +151.24%/+21.24%，同批 Swift +136.91%/+11.02%；增强版仍未打穿 v6。旧 News 与正式 Swift 均保留。[920 局机制研究](docs/swift-mechanism.md)发现，单纯增加随机成交活跃度不足以恢复新闻环境下的高收益；真实成交方向有用，已出现大阳线不是入场必要条件，分红也贡献了大量收益。

**机制审计补充：** 旧周期派息的 160 局轨迹中，15 局的普通派息超过按统一股权登记计算的全市场上限，[详情及逐局证据](docs/swift-mechanism.md)已归档。当前逐 tick 方案按用户选择保留个人时钟和邮箱结算，接受转手期间少量多派/少派，不增加统一登记；这不是严格股权归属修复。上述收益均是旧机制记录，新机制需要单独验证。

[Swift 策略特性与研究档案](docs/swift-research.md) 是后续研究的统一入口，集中记录信号、执行、容量、失败条件和冻结证据。[单一狩猎者实验](docs/swift-hunt-results.md) 使用一个十倍资金的 Falcon 与一个固定 Swift 对抗，分别比较目标受损和对手是否相对胜出。Falcon 的实验模式通过参数指定，relay 为 `mode=6`，定时买卖对照为 `mode=7`。

[资金优势实验](docs/swift-capital-results.md) 已完成矩阵、独立确认与[现金/库存机制核验](docs/swift-capital-cash-results.md)，共 2,140 局。大本金、大初始库存的 relay 往往能百分比少亏；只多给现金的证据较弱，猎手本身仍亏损，且会恶化盘口流动性。

[猎手研究的重要发现](docs/swift-hunter-findings.md) 已归档。多 Swift 实验按用户要求中止，500 局成组结果见[中止报告](docs/swift-crowding-hunt-results.md)；另有 8 个未完成配置的原始摘要，不纳入统计。实验不会自动续跑。

[周期派息 v6](docs/baseline-v6.md) 加入五个不同参数的小资金 Swift，现保存在 `baseline-v6-five-swift.toml`；当前默认已改为两个 Swift、逐 tick 普通股息。旧研究脚本已绑定归档。在当时引擎的 80 局固定验收中，默认 Swift 挑战 v5/v6 的平均百分位分别为 **96.51 / 60.21**，v6 未打穿。该历史结果不能当作当前基准的成绩；派息改造见[改造报告](docs/per-tick-dividends.md)，当前配置见[两个 Swift 的 v6](docs/baseline-v6-current.md)。

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
