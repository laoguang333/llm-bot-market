# v11：加入固定的 Horizon 对手

本页为历史 v11，保存在 `scenarios/baseline-v11.toml`；当前默认已升为 [v12](baseline-v12.md)。v11 按用户要求将 Horizon 加入背景；为保持新策略研究的对手固定，加入的是原样保存的 **Horizon v2**，名称 `horizon-incumbent`。新版 `horizon`（v3）在同一背景上另加一个账户比较，详见 [v3 研究](horizon-v3.md)。

相对 v10，唯一场景变化是追加一个账户：

```toml
[[bots]]
bot = "horizon_v2"
owner = "horizon-incumbent"
count = 1
cash = 20_000_000
shares_each = 200
news_multiplier = 1.0
forecast_access = { horizon = 300, coverage = 0.5, range_width = 3.0 }
```

现金单位为分，即 20 万元；每只股票 200 股，与 Titan 相同。选择 300-tick 权限是因为上一轮 v2 在这一权限下表现更好，不按本轮新世界重新挑参数。固定对手代码与改造前逐字节相同，收费不减免。

背景共 **100–224 个账户、13 种策略**，seed 7 为 148 个。保留普通 Swift、小资金 single Swift、NewsV2 和普通权限 Titan；Astra、Falcon 不加入。新增的账户同时增加资金与初始股票供给，会影响内生成交、价格和排名；不能把 v10/v11 的收益变化解释为只有研究能力一个因素。

世界仍为 10000 ticks、每股平均约 100 ticks 涉及一次消息、50% 概率的分阶段成长者；普通股息和组合收益逐 tick 派发，闲置费每 50 ticks 为平均权益的 0.6%，贷款利率 0.3%，成交费每边 5 bps。期末兑现平均相当于半局时长，提示随时长缩放。新闻生成、普通披露、世界真值、撮合、强平、决策频率与调度均未在本轮改动。

新研究场景 `scenarios/horizon-v11.toml` 追加 v3/1000 的 `researcher`，所以是 **101–225 个账户**，资金与固定 Horizon 和 Titan 相同。纯背景不要求新 bot 打穿；候选研究的排名与收益单独记录，不能把基准成员本身的表现当作挑战成绩。

纯背景性能比较使用 release、4 工作线程、Windows 10 核亲和性预算；种子 7、27006、28012，各三次，串行运行。v10/v11 进程墙钟中位数为 4.928/4.832 秒，差异不视为加速；九对外生新闻、最终股息和终值抽签一致。结果见 [v3 研究](horizon-v3.md)及其机器可读汇总。

v10 及更早场景保留。历史 `horizon-v10.toml` 只将策略注册名改为 `horizon_v2`，世界和经济参数不变，防止当前 `horizon` 名称升级后误跑新版；旧冻结配置与二进制未改。
