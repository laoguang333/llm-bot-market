"""Render the completed predator study without suppressing unsuccessful candidates."""
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
NAMES = {"alone":"Swift 单独参赛", "pursuit":"提前同向争抢", "reverse":"反向承接",
         "supply":"提前持仓后供给", "pressure":"供给并增加买盘", "relay":"信号反馈 relay",
         "clock":"定时买卖 clock", "trap":"维持正向成交 trap", "sweep":"扩大扫单 sweep",
         "cash":"持现金对照", "blind":"固定暴露对照", "coral":"十倍资金 Coral"}


def load(name):
    return json.loads((ROOT/"docs"/name).read_text(encoding="utf-8"))


def main():
    result=load("swift-hunt-results.json")
    dev=load("swift-hunt-development.json")
    feedback=load("swift-hunt-feedback-development.json")
    assert result["runs"]==440
    cases={r["case"]:r for r in result["combined"]}
    batches={(r["case"],r["stage"]):r for r in result["by_stage"]}
    successes=[]
    for key in ["relay","clock"]:
        good=all(batches[key,stage]["mean_advantage_pp"]>0 and batches[key,stage]["wins"]>batches[key,stage]["runs"]/2
                 for stage in result["stages"])
        if good and cases[key]["advantage_ci95"][0]>0:
            successes.append(key)
    conclusion=("达到预定相对胜出门槛的路线："+"、".join(NAMES[k] for k in successes)) if successes else "本轮没有找到稳定比 Swift 亏得更少或赚得更多的狩猎策略。"
    total=sum(r["runs"] for r in [dev,feedback,result])
    lines=["# Swift 单一狩猎者研究结果", "", f"**{conclusion}**", "",
           f"完成 {total} 局：120 局第一轮开发、72 局反馈假设开发、440 局冻结验收。每局只有一个原版 Swift；新增对手是一个初始权益恰好十倍的账户。", "",
           "![狩猎研究概览](swift-hunt-overview.png)", "",
           "## 冻结验收结果", "",
           "对手胜局按收益率比较，不按绝对金额。两批共 40 个新种子；每个对手处理交换两次注册顺序，所以是 80 局而非 80 个独立世界。原版单独参赛为 40 局。", "",
           "| 处理 | Swift 平均收益 | 对手平均收益 | 对手相对优势 | 对手胜局 | 双方都亏时对手少亏 |",
           "|---|---:|---:|---:|---:|---:|"]
    for key in ["alone","relay","clock","cash","blind","coral"]:
        r=cases[key]
        if key=="alone":
            lines.append(f"| {NAMES[key]} | {r['target']['mean_return_pct']:+.2f}% | — | — | — | — |")
            continue
        lines.append(f"| {NAMES[key]} | {r['target']['mean_return_pct']:+.2f}% | {r['predator']['mean_return_pct']:+.2f}% | "
                     f"{r['mean_advantage_pp']:+.2f} 个百分点 | {r['wins']}/{r['runs']} | {r['both_lose_predator_wins']}/{r['both_lose']} |")
    lines += ["", "收益包括游戏分红、费用、利息及终值结算，不是纯差价利润或年化。双方都亏是事后筛选的子集，不能把这一列单独当作总体胜率。", "",
              "| 处理 | 平均收益差的95%区间 | A批收益差 / 胜局 | B批收益差 / 胜局 |", "|---|---:|---:|---:|"]
    for key in ["relay","clock","cash","blind","coral"]:
        r=cases[key]; a=batches[key,"validation-a"]; b=batches[key,"validation-b"]
        lo,hi=r["advantage_ci95"]
        lines.append(f"| {NAMES[key]} | [{lo:+.2f}, {hi:+.2f}] | {a['mean_advantage_pp']:+.2f} / {a['wins']}/40 | {b['mean_advantage_pp']:+.2f} / {b['wins']}/40 |")
    lines += ["", "区间按种子聚类：先在一个种子内平均两个注册顺序，再对 40 个种子做 3,000 次 bootstrap。区间是描述性的，没有为所有候选做多重比较校正。", "",
              "## 狩猎机制和成本", "",
              "Falcon 只从匿名公共成交、盘口和中价重建 Swift 的公开信号。relay 在推测 Swift 尚未启用仓位时小批量买入，推测其已启用仓位时小批量卖出。每次上限是自己的初始持股量五分之一，本实验为 40 股。它不知道 Swift 是否实际买到、持有多少，或者是否看到相同盘口。", "",
              "clock 每 20 tick 机械换方向，完全不按信号选择方向。它检验持续来回交易本身的影响；即使它优于 relay，也不能说成成功识别了 Swift。两个模式都遵守预算、可卖股票及同一撮合规则，最后 40 tick 尝试退出。", "",
              "| 处理 | 角色 | 收益中位数 | 亏损局数 | 平均绝对损益（元） | 平均/最差最大回撤 |", "|---|---|---:|---:|---:|---:|"]
    for key in ["relay","clock","cash","blind","coral"]:
        for owner,label in [("target","Swift"),("predator","对手")]:
            r=cases[key][owner]
            lines.append(f"| {NAMES[key]} | {label} | {r['median_return_pct']:+.2f}% | {r['losses']}/80 | {r['mean_pnl_yuan']:+,.0f} | {r['mean_drawdown_pct']:.1f}% / {r['worst_drawdown_pct']:.1f}% |")
    examples=[r for r in result["worlds"] if r["case"] in ["relay","clock"]
              and r["target"]["return_pct"]<r["predator"]["return_pct"]<0]
    lines += ["", "部分局数是否出现了用户提出的双方都亏、狩猎者少亏，可以由下列实例直接复查。按种子及顺序排列取前三例，属于事后展示，不代表整体成功：", "",
              "| 路线 | 种子 | 注册顺序 | Swift 收益 | 对手收益 |", "|---|---:|---|---:|---:|"]
    for r in sorted(examples,key=lambda r:(r["seed"],r["order"],r["case"]))[:3]:
        lines.append(f"| {NAMES[r['case']]} | {r['seed']} | {r['order']} | {r['target']['return_pct']:+.2f}% | {r['predator']['return_pct']:+.2f}% |")
    lines += ["", "回撤按每次决策前的标记权益和最终结算计算，不是流动性枯竭时保证可实现的清仓损失。十倍资金意味着同样百分比损失的金额约为十倍，不能省略绝对损益。", "",
              "对手的风险暴露与费用进一步分开；下表费用均为相对初始权益的比例，已经计入前面的总收益：", "",
              "| 路线 | 平均持仓总值/权益 | 交易费 | 闲置费 | 利息 |", "|---|---:|---:|---:|---:|"]
    for key in ["relay","clock","cash","blind","coral"]:
        r=cases[key]["predator"]
        lines.append(f"| {NAMES[key]} | {r['mean_gross_leverage']:.2f} | {r['trade_fees_pct']:.2f}% | {r['idle_fees_pct']:.2f}% | {r['interest_pct']:.2f}% |")
    lines += ["", "这能区分成交成本与持仓风险，但不能把剩余亏损全部归为对 Swift 的攻击成本；行情变化、分红和对其他账户的交易也包含在内。", "",
              "## 相对于同资金对照的目标变化", "",
              "以下都是同种子、同注册顺序的差，再按种子聚类。负数表示 Swift 在该路线下收益更低。更换对手会改变成交和价格路径；这些差不是一段固定行情的回放。", "",
              "| 路线 | 对照 | Swift 收益变化（百分点） | 95%区间 | 对手自身收益变化（百分点） |", "|---|---|---:|---:|---:|"]
    for r in result["comparisons"]:
        if r["case"] not in ["relay","clock"]:
            continue
        lo,hi=r["target_change_ci95"]
        lines.append(f"| {NAMES[r['case']]} | {NAMES[r['reference']]} | {r['target_return_change_pp']:+.2f} | [{lo:+.2f}, {hi:+.2f}] | {r['predator_return_change_pp']:+.2f} |")
    pressure=next(r for r in result["comparisons"] if r["case"]=="clock" and r["reference"]=="cash")
    lo,hi=pressure["predator_change_ci95"]
    lines += ["", f"机械扰动相对持现金对照，让 Swift 平均少赚 {-pressure['target_return_change_pp']:.2f} 个百分点，对手自身平均多亏 {-pressure['predator_return_change_pp']:.2f} 个百分点（自身变化95%区间 [{lo:+.2f}, {hi:+.2f}]）。这表明收益压制与最终相对胜负应分开：压制幅度较大，并不等于对手赢了，也不是这笔损失转入了对手账户。这里的额外损失是对照差，不是能单独结算的攻击费用。", ""]
    lines += ["", "持现金对照提供少承担市场风险的参照，固定暴露提供不依据方向信号交易的参照，Coral 代表另一种成熟策略。它们的持仓和换手也会不同，不能只归因于一个信号开关。Coral 使用其原有估值信息，信息集与 Falcon 不同，因此它是机会成本参考，不是严格同信息消融。", "",
              "## 注册顺序与直接成交", "",
              "| 处理 | 注册顺序 | Swift 平均收益 | 对手平均收益 | 对手胜局 |", "|---|---|---:|---:|---:|"]
    for r in result["by_order"]:
        if r["case"] not in ["relay","clock"]:
            continue
        order="对手先加入" if r["order"]=="predator_first" else "Swift 先加入"
        lines.append(f"| {NAMES[r['case']]} | {order} | {r['target']['mean_return_pct']:+.2f}% | {r['predator']['mean_return_pct']:+.2f}% | {r['wins']}/40 |")
    lines += ["", "| 处理 | Swift 成交额中直接与对手成交的比例 | Swift 换手倍数 | 对手换手倍数 |", "|---|---:|---:|---:|"]
    for key in ["relay","clock","coral"]:
        r=cases[key]
        lines.append(f"| {NAMES[key]} | {r['target']['direct_counterparty_notional_share']:.2%} | {r['target']['turnover']:.1f} | {r['predator']['turnover']:.1f} |")
    markout=load("swift-hunt-markout.json")
    lines += ["", "成交后的方向性价格变化作为补充诊断：买入后价格上升、卖出后价格下降记为正；以成交金额加权，在每局内计算后跨局等权平均。", "",
              "| 处理 | Swift 成交后约20 tick 的有符号中价变化 | 可匹配覆盖率 |", "|---|---:|---:|"]
    for r in markout["combined"]:
        lines.append(f"| {NAMES[r['case']]} | {r['signed_mid_markout20_bps']:+.1f} bp | {r['coverage']:.1%} |")
    lines += ["", "使用成交后 20–30 tick 内最早可用双边盘口中价；这是事后标记变化，包含正常派息等价格变化，不是已实现交易利润或独立因果证据。原始 JSON 中的机械阶段分组还受到本地时钟差影响，只作近似描述。", ""]
    lines += ["", "Swift、relay 和 clock 使用 IOC，不留下供彼此成交的挂单。若直接成交比例为零，收益变化来自其他参与者和价格、盘口、成交反馈，不能称为直接从 Swift 手中赚走了这些钱。身份仅用于本段事后归因，bot 接口不包含成交对手身份。", "",
              "## 全部开发路线", "",
              "开发集只有 8 个种子，交换顺序。下表保留失败路线，不与冻结验收混合计分。", "",
              "| 阶段 | 路线 | Swift 平均收益 | 对手平均收益 | 对手收益差 | 胜局 |", "|---|---|---:|---:|---:|---:|"]
    for stage,rpt in [("第一轮",dev),("反馈实验",feedback)]:
        for r in rpt["combined"]:
            if "predator" not in r:
                continue
            lines.append(f"| {stage} | {NAMES[r['case']]} | {r['target']['mean_return_pct']:+.2f}% | {r['predator']['mean_return_pct']:+.2f}% | {r['mean_advantage_pp']:+.2f} | {r['wins']}/{r['runs']} |")
    audits={key:sum(r["audit_totals"][key] for r in [dev,feedback,result]) for key in result["audit_totals"]}
    lines += ["", "## 为什么知道代码仍不等于低成本获胜", "",
              "以下是结合实现与实验的解释，尚未逐项做独立因果隔离：", "",
              "- 公开信号只能推测方向。Swift 是否已经成交、持有多少、此刻看到的盘口，都不是狩猎者可见的状态。",
              "- Swift 使用最优价 IOC。对手把报价推走后，Swift 的旧限价可能直接不成交，并不会必然追着更差价格买。主动推价者先承担成交费用和库存风险。",
              "- 反向持仓不是免费攻击。relay 的持仓和亏损显示，它在目标退出的阶段会自己承担较大的股票风险；强行扫单也可能把机会送给其他估值策略。",
              "- 一个十倍资金账户与十个独立小账户不同。独立账户有各自的观测、决策时刻、库存与后续订单；合成一个大账户不保留这些特征，因此不能直接套用此前十个 Swift 的拥挤结果。", "",
              "## 审计与边界", "",
              f"- {total} 局共 {audits['audits']} 次账务审计；全体账户跳过 {audits['all_skipped']} 次，bot 错误 {audits['all_errors']} 次，停用 {audits['all_disabled']} 次。",
              "- 目标 Swift 代码、参数和市场引擎未修改。资金倍率在每局自动核对，权益轨迹必须覆盖全部 3,000 次观测。",
              "- 41 项 Rust 测试、3 项统计检验、格式检查及严格 Clippy 通过；Falcon 的所有模式检验身份、seed、新闻和私有终值输入不改变动作。",
              "- 原生多线程撮合存在路径不确定性，同种子重复也可能改变结果。两次开发单独 Swift 参照分别约 +96.3% 和 +117.0%，所以结果必须看批量与对照，不能据单局断言。",
              "- 当前结果只约束这七条针对性路线和一个定时扰动对照，不能证明任何单一对手都无法攻击 Swift。没有改变市场规则、利用目标身份或在验收集继续调参。", "",
              "验收中的拒单与强平单列，不能把交易拒绝混同 bot 执行错误：", "",
              "| 处理 | Swift 拒单 / 强平 | 对手拒单 / 强平 |", "|---|---:|---:|"]
    for key in ["relay","clock","cash","blind","coral"]:
        r=cases[key]
        lines.append(f"| {NAMES[key]} | {r['target']['rejects']} / {r['target']['liquidations']} | {r['predator']['rejects']} / {r['predator']['liquidations']} |")
    lines += ["", "## 复查和复现", "",
              "实现与实验模式见 [Falcon 源码](../crates/bots/src/falcon.rs)。默认 mode=0 是第一轮 pursuit；本次冻结主要候选明确使用 mode=6，定时对照使用 mode=7。不要把默认模式的成绩与 relay 混在一起。", "",
              "    cargo build --release --bin hunt-eval",
              "    python tools/swift_hunt.py prepare-validation --stage replay --candidate relay --first-seed 13001 --seeds 20",
              "    python tools/arena_gate.py --cwd . -- python tools/swift_hunt.py run --stage replay",
              "    python tools/swift_hunt_report.py --stages replay --output runs/swift-hunt-replay.json", "",
              "replay 目录必须是新目录；此命令会重新记录当前二进制和源码哈希。历史可执行文件保存在 runs/swift-hunt-20261018，原版 Swift 验收用的旧可执行文件也已归档。Windows 若 sccache 不可用，在构建子进程里将 RUSTC_WRAPPER 设为空。", "",
              "- [Swift 总研究档案](swift-research.md)",
              "- [预定方案与开发后冻结选择](swift-hunt-protocol.md)",
              "- [全部冻结验收数据](swift-hunt-results.json)",
              "- [第一轮开发数据](swift-hunt-development.json)",
              "- [反馈开发数据](swift-hunt-feedback-development.json)",
              "- [A批冻结清单](swift-hunt-validation-a-plan.json) / [B批冻结清单](swift-hunt-validation-b-plan.json)", ""]
    (ROOT/"docs/swift-hunt-results.md").write_text("\n".join(lines),encoding="utf-8")
    print(conclusion,"runs",total,"audits",audits)


if __name__=="__main__":
    main()
