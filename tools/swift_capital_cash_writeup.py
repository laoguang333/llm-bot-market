"""Render the fresh-world control separating cash from initial inventory."""
import json
from swift_capital import ROOT
from swift_capital_report import fmt_ci


def main():
    data = json.loads((ROOT / "docs/swift-capital-cash-results.json").read_text(encoding="utf-8"))
    by = {r["case"]: r for r in data["combined"]}
    text = ["# 新增现金与新增初始库存的区分", "",
            "本组是在主矩阵发现大额初始股票可能影响结论后追加的机制核验，使用新种子 17001–17020，不参与原矩阵选参。方案见[追加核验协议](swift-capital-cash-protocol.md)。", "",
            f"完成 {data['runs']} 局。每个双账户配置为 40 局、20 个种子、两种顺序；始终只有一个不变的 Swift。", "",
            "cash_active 只保留每只股票 200 股，其余新增权益为现金；允许使用全部初始权益的仓位额度，100 / 300 倍时普通信号脉冲为 400 / 1,200 股。超目标仓位减仓、末期清仓可以超过脉冲股数。scaled 使用相同总权益、额度和脉冲股数，但开局分别有每股 2,000 / 6,000 股。策略方向规则相同，额外库存必须在 cash_active 中真实买到。", "",
            "reserve 在这里仅指具有同样开局资产的持现金对照：卖出初始 200 股后停止买入。它与现金攻击者具有相同初始股票和现金。", "",
            "| 倍率 | 处理 | Swift 平均收益 | 对手平均收益 | 对手收益率差 / 95%区间 | 对手胜局 | Swift 亏损局 | 双亏时对手少亏 |",
            "|---|---|---:|---:|---:|---:|---:|---:|"]
    for m in [100, 300]:
        for case, label in [(f"relay-cash_active-{m}", "新增现金、主动交易"),
                            (f"relay-scaled-{m}", "同时增加初始股票、主动交易"),
                            (f"cash-reserve-{m}", "同现金开局、卖出后持现金"),
                            (f"cash-scaled-{m}", "同大库存开局、卖出后持现金")]:
            r = by[case]
            text.append(f"| {m} | {label} | {r['target']['mean_return_pct']:+.2f}% | {r['predator']['mean_return_pct']:+.2f}% | {r['mean_advantage_pp']:+.2f} / {fmt_ci(r['advantage_ci95'])} | {r['wins']}/{r['runs']} | {r['target']['losses']}/{r['runs']} | {r['both_lose_predator_wins']}/{r['both_lose']} |")
    text += ["", "收益包含全部分红、终值和费用，不是纯价差利润。置信区间先在种子内平均两种顺序，再做 3,000 次 bootstrap。", "",
             "## 现金组与库存组的直接配对", "", "下表为 cash_active 减 scaled；两者本金、仓位额度和普通信号脉冲股数相同。", "",
             "| 倍率 | Swift 收益变化 / 95%区间 | 对手收益变化 / 95%区间 | 对手绝对损益变化（元） |", "|---|---:|---:|---:|"]
    for r in data["cash_active_minus_scaled"]:
        text.append(f"| {by[r['case']]['multiple']} | {r['target_return_pct']:+.2f} / {fmt_ci(r['target_return_pct_ci95'])} | {r['predator_return_pct']:+.2f} / {fmt_ci(r['predator_return_pct_ci95'])} | {r['predator_pnl_yuan']:+,.0f} |")
    text += ["", "## 相对于同资产的持现金对照", "", "| 处理 | 对照 | Swift 收益变化 / 95%区间 | 对手收益变化 |", "|---|---|---:|---:|"]
    for r in data["comparisons"]:
        if by[r["case"]]["mode"] == "relay":
            text.append(f"| {r['case']} | {r['reference']} | {r['target_return_pct']:+.2f} / {fmt_ci(r['target_return_pct_ci95'])} | {r['predator_return_pct']:+.2f} |")
    text += ["", "## 金额、费用和执行", "", "| 配置 | 对手本金（元） | Swift 损益（元） | 对手损益（元） | 对手交易费% / 闲置费% | 平均持仓总值/权益 | Swift 直接与对手成交占比 |", "|---|---:|---:|---:|---:|---:|---:|"]
    for r in data["combined"]:
        if r["mode"] is None:
            continue
        t, p = r["target"], r["predator"]
        text.append(f"| {r['case']} | {p['mean_initial_equity_yuan']:,.0f} | {t['mean_pnl_yuan']:+,.0f} | {p['mean_pnl_yuan']:+,.0f} | {p['trade_fees_pct']:.2f} / {p['idle_fees_pct']:.2f} | {p['mean_gross_leverage']:.3f} | {100*t['direct_counterparty_notional_share']:.2f}% |")
    a = data["audit_totals"]
    rejects = sum(r[o]["rejects"] for r in data["combined"] for o in ["target", "predator"] if o in r)
    liquidations = sum(r[o]["liquidations"] for r in data["combined"] for o in ["target", "predator"] if o in r)
    text += ["", f"共 {a['audits']} 次账务审计；全账户跳过 {a['all_skipped']}、错误 {a['all_errors']}、停用 {a['all_disabled']}；Swift 与对手拒单 {rejects}、强平 {liquidations}。", "",
             "四个 relay 配置各 40 局均未通过全部市场质量阈值，现金主动组的双边盘口覆盖率均值仅约 55%；无猎手组约 99.5%。这支持大资金交易明显改变了流动性环境，不能将收益压制直接解释为正常流动性下的预测优势。", "",
             "不同处理改变市场价格路径，不能视为同一行情的固定回放。只有给定策略与资金范围受到检验，失败不能证明不存在其他现金攻击方法。所有市场质量检查与失败样本保留于[完整数据](swift-capital-cash-results.json)。", ""]
    (ROOT / "docs/swift-capital-cash-results.md").write_text("\n".join(text), encoding="utf-8")


if __name__ == "__main__":
    main()
