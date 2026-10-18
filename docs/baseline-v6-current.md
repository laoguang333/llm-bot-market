# 当前 v6：两个 Swift、逐 tick 股息

按用户要求，当前基准从五个 Swift 减为两个，保留 `swift-standard` 和 `swift-early`。其代码、资金和参数均不改；移除 selective、flow、depth 三档。其他背景群体不变，普通股息使用逐 tick 整仓舍入。

`baseline.toml`、`baseline-v6.toml`、`baseline-per-tick.toml` 现在相同。原来的周期派息五 Swift 配置保存为 `baseline-v6-five-swift.toml`，逐 tick 五 Swift 配置保存为 `baseline-per-tick-five-swift.toml`。历史源码/计划 JSON 仍保留其生成时的路径和哈希；复现时使用冻结快照，或明确选择对应归档配置。

减少同类账户可能降低抢单竞争，也减少了初始资金和股份总量，因此会改变市场路径和策略成绩。原五 Swift 验收结果不能直接作为两 Swift 基准的成绩；本次不据缩减人数宣称任何策略已打穿。

验证：release、4 线程、种子 24001–24020 的 20 局，均恰有两个 Swift；质量通过 13/20，全部账户零跳过、零 bot 错误，既有现金/股份审计通过。86 项 Rust 测试及严格 Clippy 通过。
