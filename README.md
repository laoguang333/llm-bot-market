# llm-bot-market

一个连续限价盘口的股票市场，让多个 LLM 各自写的交易 bot 在里面对抗，只比谁赚得多（按收益率的排名百分位计分）。完整规则见 [PRD.md](PRD.md)，内置策略的对比结果见 [docs/strategies.md](docs/strategies.md)。每个账户是一个独立的 owner；场景里写了 `owner` 的那组只能有一个账户。

## 快速开始

```sh
cargo build --release
# 跑一局，输出 summary.json、成交/K 线 CSV 和离线 report.html
./target/release/llm-market run scenarios/tournament.toml --out runs/t1
# 在 20 个随机世界里各跑一局，输出每个策略的平均排名百分位 ± 标准差
./target/release/llm-market series scenarios/tournament.toml --out runs/series --seeds 20
# 基准市场 v3（含 hunter 和 shark；旧版是 baseline-v1.toml、baseline-v2.toml）：参数和人数都随机的对照 bot，输出 9 项市场验收的通过率
./target/release/llm-market series scenarios/baseline.toml --out runs/base --seeds 20
# 打穿测试：一个挑战者账户加入基准市场，在留出种子 1001–1020 上跑
./target/release/llm-market challenge scenarios/baseline.toml --bot shark --out runs/ch-shark --seeds 20
# 列出内置 bot
./target/release/llm-market bots
```

## 目录

| 路径 | 内容 |
|---|---|
| `crates/bot-api` | bot 与引擎之间的全部类型：`Rules`、`Observation`、`Action`、`Bot` trait |
| `crates/market` | 撮合、调度、结算、世界生成、计分与报告 |
| `crates/bots` | 内置 bot：`baseline.rs` 是对照组，`strategic.rs` 是博弈型策略 |
| `crates/cli` | 命令行 `llm-market` |
| `scenarios/` | `baseline.toml`（只有对照组）、`tournament.toml`（对照组 + 博弈型）、`arena.toml`（只有博弈型和两个对照） |

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

## 开发

```sh
cargo fmt && cargo clippy --all-targets -- -D warnings && cargo test
```

release 构建关闭整数溢出检查；`cargo build --profile release-checked` 打开。
