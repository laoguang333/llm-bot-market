# Bot 接口

这份文档写给编写 bot 的人或 LLM。bot 的唯一目标是让自己账户的最终权益尽量高。

## 运行方式

- bot 是一个 Python 文件，定义 `class Bot`。只能用标准库。
- 每个 bot 跑在自己的进程里，看不到引擎和其他 bot 的内存。`print` 的内容写进自己的日志，不会影响协议。
- 每 tick 引擎把观察发给所有空闲的 bot，等待最多 `step_timeout_ms`（默认 250ms）。按时答复的 bot 的动作按随机顺序依次执行；超时的 bot 错过这一 tick，它迟到的答复会被丢弃，期间的成交、新闻等消息会合并到下一次观察里。连续 `max_late_ticks` 个 tick 没有答复的 bot 会被停用，挂单全部撤销。
- `on_tick` 抛异常只算这一 tick 没有动作；进程退出则被停用。
- 有内存上限和总 CPU 秒数上限（见场景的 `[sandbox]`）。

```python
class Bot:
    def __init__(self, start): ...
    def on_tick(self, obs): return [...]   # 动作列表
    def on_end(self, msg): ...             # 可选，msg["results"] 是自己的最终结果
```

## start

| 字段 | 含义 |
|---|---|
| account / owner | 自己的账户名和所属作者 |
| seed | 给自己用的随机种子（用它可以让结果可复现） |
| params | 场景里给这组 bot 配的参数 |
| cash / shares | 初始现金（分）和初始持股 |
| rules | 所有公开规则，见下 |

`rules` 包括：`stocks`（名称、初始股息、初始参考价）、`ticks_total`、`dividend_period`、`trade_fee_bps`、`carry_period`、`carry_rate`、`credit_ratio`、`credit_side`、`stp_scope`、`max_actions_per_tick`、`max_open_orders`、`step_timeout_ms`、`book_depth`、`final_mark`、`vwap_ticks`、`discount_rate`。

## obs（每 tick）

所有价格和金额都是整数分。

| 字段 | 含义 |
|---|---|
| tick / ticks_total | 当前 tick（从 0 开始）和总长度 |
| account.cash / cash_available | 现金，和扣除买单冻结后的可用现金 |
| account.positions[股票] | `shares` 持股，`available` 扣除卖单冻结后的可卖股数 |
| account.carry | 本费用周期的估算：`period_end_tick` 结算 tick，`fee_estimate` 按目前平均现金算的费用，`credit_estimate` 已获得的抵扣，`due_estimate` 预计要付的钱，`debt` 上期没付清的欠款 |
| open_orders | 自己的挂单：`order_id, stock, side, price, qty`（剩余）, `placed_tick` |
| market[股票] | `bids` / `asks`：前 N 档 `[价格, 总量]`，最优价在前；`last` 最新成交价；`dividend` 当前每股股息；`next_dividend_tick` |
| trades[股票] | 上次观察以来的公开成交 `[tick, 价格, 数量, 主动方 buy/sell]`，不含身份 |
| fills | 自己的成交：`order_id, stock, side, price, qty, fee, role(maker/taker), tick` |
| acks | 自己下单和被撤单的结果：`cid`（你下单时带的标记）, `order_id`, `status`, `remaining` |
| rejects | 被拒绝的动作和原因 |
| news | 刚刚告诉你的新闻：`id, stock, dividend_change`（每股股息变化，分）, `effective_tick`, `first_reveal_tick`, `public` |
| income | 股息到账和持有费扣款记录 |

`news` 只出现一次（告诉你的那一刻），需要自己记住。同一条新闻在公开时会再发一次，`public` 为 true。

`status` 的取值：`resting`（全部挂在盘口）、`partial_resting`、`filled`、`partial_cancelled`（IOC 部分成交）、`no_fill`、`cancelled_stp`（因自成交保护被撤）。

## 动作

```json
{"type": "limit", "stock": "ALPHA", "side": "buy", "price": 9950, "qty": 10, "tif": "gtc", "cid": "my-tag"}
{"type": "market", "stock": "ALPHA", "side": "sell", "qty": 10}
{"type": "cancel", "order_id": 123}
{"type": "cancel_all"}
{"type": "cancel_all", "stock": "ALPHA"}
```

- `tif` 是 `gtc`（默认，挂到成交或撤单）或 `ioc`（立即成交，剩余撤销）。市价单就是不限价的 IOC，买入数量受可用现金限制。
- 买单冻结 `数量 × (限价 + 每股最大交易费)`，卖单冻结股票。不够就拒绝。
- 一 tick 最多 `max_actions_per_tick` 个动作，同时最多 `max_open_orders` 张挂单。
- 动作按列表顺序执行，所以先撤单再下单是可以的。

## 钱是怎么算的

- **交易费**：每笔成交双方各付 `成交额 × trade_fee_bps / 10000`（向上取整）。
- **持有费**：每 `carry_period` 个 tick 结算一次，`费用 = 本期每 tick 收盘现金的平均值 × carry_rate`，`抵扣 = 本期符合条件的成交额 × credit_ratio`，实付 `max(0, 费用 − 抵扣)`。`credit_side` 决定挂单方、吃单方还是双方的成交额算数。现金不够付的部分记为欠款下期再扣。
- **股息**：每 `dividend_period` 个 tick（从第一个周期末开始）按持股派发 `持股 × 当前股息`。
- **最终权益**：`现金 + Σ 持股 × 结算价`。`final_mark = "vwap"` 时结算价是最后 `vwap_ticks` 个 tick 的成交均价；`"fundamental"` 时是 `股息 / discount_rate`。
- 排名用 `pnl_vs_start`：最终权益减去"初始持仓按同一结算价估值"，也就是相对于什么都不做的超额。
