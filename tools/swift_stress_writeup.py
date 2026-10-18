"""Render all stress conditions and audit totals as a reviewable Markdown report."""
import json
from pathlib import Path
import statistics as stats

ROOT = Path(__file__).resolve().parents[1]


def main():
    report = json.loads((ROOT / "docs/swift-stress-results.json").read_text(encoding="utf-8"))
    follow = json.loads((ROOT / "docs/swift-crowding-results.json").read_text(encoding="utf-8"))
    assert report["complete"]
    cases = {r["case"]: r for r in report["combined"]}
    batches = {r["id"]: r for r in report["batches"]}
    pairs = {r["case"]: r for r in report["paired_vs_v5"]}
    ws = report["worlds"]
    extra = [w for r in follow["results"] for w in r["worlds"]]
    audits = sum(w["audits"] for w in ws+extra)
    skips = sum(w["all_accounts_skipped"] for w in ws)+sum(w["all_skipped"] for w in extra)
    errors = sum(w["all_accounts_errors"] for w in ws)+sum(w["all_errors"] for w in extra)
    disabled = sum(w["all_accounts_disabled"] for w in ws)+sum(w["all_disabled"] for w in extra)
    lines = [
        "# Swift 跨模拟市场压力测试", "",
        f"完成 **{len(ws)+len(extra)} 局运行**：27 类主实验、35 个批次共 700 局，另做 80 局拥挤效应诊断。Swift 默认代码与参数没有按结果调整。", "",
        "**Swift 有明确的适用边界，不能在所有模拟市场里维持优势。** 原版 v5 的成绩得到新种子复核；同策略拥挤、本金放大、降低决策频率、减少消息冲击和移除主要流动性提供者，均会明显改变表现。", "",
        "![压力测试概览](swift-stress-overview.png)", "",
        "## 实验方法", "",
        "- 原策略冻结于提交 5289b76，本轮没有修改 crates/bots 或 crates/market。测试程序只记录每次决策前的权益；降频实验才合并成交数据并减少策略调用。",
        "- 7001–7020 运行所有 27 类；预先指定的 8 类重点环境再用 8001–8020 复核。各条件样本量为 20 或 40，不能把全部运行当作 700 个独立样本。",
        "- 相同种子用于配对，但更改规则或对手后价格内生变化，不能当作同一路径回放。",
        "- 资金对照同时同比缩放现金和股票；纯现金开局对照使用对应 v5 世界的相同初始权益。",
        "- 去盘口、去动量、关闭信号退出、取消杠杆和持现金均标为消融/对照，没有据此替换正式 Swift。手续费变化适用于所有账户。",
        "- 决策每 5/20 tick 一次是降频测试，不是网络延迟测试，tick 不能换算为现实秒数。",
        "- 生成器没有独立的牛熊漂移开关。低波动和无消息场景不直接称为横盘市，另按实际价格净变化做事后分组。", "",
        "## 全部条件", "",
        "过线门槛沿用 v5：平均百分位至少 90，且至少 80% 的局数进入前四分位。更换对手后名次含义也变化，需同时看绝对收益与亏损局数。", "",
        "| 条件 | 局数 | 平均排名 | 前25% | 平均收益 | 中位收益 | 亏损局数 | 平均/最差最大回撤 | 过线 |",
        "|---|---:|---:|---:|---:|---:|---:|---:|---|",
    ]
    for r in report["combined"]:
        lines.append(f"| {r['label']} | {r['worlds']} | {r['mean_percentile']:.2f} | {r['top25_share']:.1%} | "
                     f"{r['mean_return_pct']:+.1f}% | {r['median_return_pct']:+.1f}% | {r['loss_worlds']}/{r['worlds']} | "
                     f"{r['mean_drawdown_pct']:.1f}% / {r['max_drawdown_pct']:.1f}% | {'是' if r['gate_pass'] else '否'} |")
    lines += ["", "收益是本局游戏收益，不是年化。正常分红、费用、利息和最终结算均包含在内。回撤从初始权益、每次决策前的标记权益和最终结算计算，不等于流动性枯竭时能实现的清仓价值。", "",
              "## 第二批复核", "", "| 条件 | 第一批排名 / 收益 | 第二批排名 / 收益 |", "|---|---:|---:|"]
    for r in report["batches"]:
        if r["id"].startswith("b-"):
            a = batches["a-"+r["case"]]
            lines.append(f"| {r['label']} | {a['mean_percentile']:.2f} / {a['mean_return_pct']:+.1f}% | {r['mean_percentile']:.2f} / {r['mean_return_pct']:+.1f}% |")
    lines += ["", "## 哪部分规则更关键", "",
              "仅比较 7001–7020。区间按配对种子差异做 3,000 次 bootstrap，包含市场因策略改变产生的反馈。", "",
              "| 对照 | 平均排名 | 平均收益 | 相对默认策略的收益差（百分点） | 差异95%区间 |", "|---|---:|---:|---:|---:|"]
    default = batches["a-v5"]
    lines.append(f"| 默认策略 | {default['mean_percentile']:.2f} | {default['mean_return_pct']:+.1f}% | — | — |")
    for key in ["flow_only", "no_depth", "no_momentum", "no_signal_exit", "unlevered", "cash_control"]:
        r, p = cases[key], pairs[key]
        lo, hi = p["return_difference_ci95"]
        lines.append(f"| {r['label']} | {r['mean_percentile']:.2f} | {r['mean_return_pct']:+.1f}% | {p['return_difference_pp']:+.1f} | [{lo:+.1f}, {hi:+.1f}] |")
    lines += ["", "主动成交方向单独仍能过线；关闭负向信号清仓会大幅恶化表现。盘口与动量是辅助项，当前证据不支持把每一项都描述为不可缺少。关闭信号退出时，仓位数量调整和最后 20 tick 清仓仍保留，并非禁止所有卖出。", "",
              "## 容量", "", "同样只比较第一批配对世界。放大现金和股票会改变市场供需，属于有市场影响的容量测试。", "",
              "| 资金规模 | 平均初始权益（元） | 平均净利润（元） | 平均收益率 |", "|---|---:|---:|---:|"]
    for key in ["capital01", "v5", "capital10", "capital100"]:
        selected = [w for w in ws if w["case"] == key and w["batch"].startswith("a-")]
        lines.append(f"| {cases[key]['label']} | {stats.mean(w['initial_equity_cents'] for w in selected)/100:,.0f} | "
                     f"{stats.mean(w['final_equity_cents']-w['initial_equity_cents'] for w in selected)/100:+,.0f} | {stats.mean(w['return_pct'] for w in selected):+.1f}% |")
    lines += ["", "小资金提高的是收益率与进出灵活性，不等于绝对利润更高：本组十分之一本金的平均利润仍低于标准本金。", ""]
    crowded = cases["crowded"]
    lines += ["", "## 拥挤与顺序", "",
              f"11 个 Swift 的等权平均收益为 **{crowded['swift_cohort_mean_return_pct']:+.2f}%**。以下同时列出其他副本，避免只看最后加入的挑战者。", "",
              "| Swift 顺序 | 平均收益率 | 亏损局数 |", "|---|---:|---:|"]
    for owner, value in crowded["swift_cohort_by_position"].items():
        lines.append(f"| {owner} | {value['mean_return_pct']:+.2f}% | {value['loss_worlds']}/{crowded['worlds']} |")
    lines += ["", "同一策略在不同位置的表现有明显差异，不能忽略先后获取短期机会的影响。尚未分离线程调度、排队顺序、共同追单和自身交易反馈的各自贡献。", "",
              "发现首批拥挤失败后，追加以下诊断实验，用新种子 9001–9020；这部分不冒充最初预注册实验。三种新增对手处理的账户数和初始资金相同，使用冻结 Swift 和原 llm-market 可执行文件。", "",
              "| 追加对照 | 平均排名 | 平均收益 | 中位收益 | 亏损局数 |", "|---|---:|---:|---:|---:|"]
    labels = {"v5": "不增加账户", "swift10": "增加10个默认Swift", "coral10": "增加10个默认Coral", "cash10": "增加10个卖出初始股票后持现金的账户"}
    for r in follow["results"]:
        lines.append(f"| {labels[r['key']]} | {r['mean_percentile']:.2f} | {r['mean_return_pct']:+.1f}% | {r['median_return_pct']:+.1f}% | {r['loss_worlds']}/20 |")
    lines += ["", "增加同款 Swift 的损害在新种子下再次出现，增加等量 Coral 或持现金账户则没有出现同样的崩塌。这支持同策略竞争是关键条件；但没有把执行顺序、信号共同反应和自身成交反馈分别控制，不能指定其中一个为唯一原因。", "",
              "## 市场状态", "",
              "不满足原 v5 全部质量检查的场景也保留。无消息场景自然减少估值穿越，不能仅凭这一项认定市场失效。", "",
              "| 条件 | 双边盘口覆盖 | 成交覆盖 | 中位价差的跨局均值 | 薄侧深度/典型成交 |", "|---|---:|---:|---:|---:|"]
    for r in report["combined"]:
        if r["group"] == "market":
            q = r["quality"]
            lines.append(f"| {r['label']} | {q['two_sided_share']:.1%} | {q['trade_coverage']:.1%} | {q['median_spread']:.3%} | {q['depth_2pct_in_trades']:.1f} |")
    lines += ["", "价格净变化的事后分组，仅计市场环境组；重复种子不作独立样本显著性推断：", ""]
    regime_names = {"price_down_5_to_30pct": "价格净降 5%–30%", "price_down_more_than_30pct": "价格净降超过 30%",
                    "price_change_within_5pct": "价格净变化在 ±5% 内", "price_up_more_than_5pct": "价格净涨超过 5%"}
    for tag, r in report["descriptive_price_regimes"].items():
        lines.append(f"- {regime_names.get(tag, tag)}：{r['world_runs']} 次运行，平均排名 {r['mean_percentile']:.2f}，中位收益 {r['median_return_pct']:+.1f}%，亏损 {r['loss_worlds']} 次。")
    lines += ["", "这些分组使用各股票首末双边盘口中价涨跌幅的中位数。本次没有净上涨或净变化 ±5% 内的市场组样本，不能声称已覆盖完整牛熊横盘周期。游戏股票按有限剩余分红定价，随着分红派发，剩余现金流期限缩短；净价格下降不能直接等同现实熊市。", ""]
    normal = cases["v5"]
    lines += ["## 收益构成", "",
              f"原版 v5 的平均总收益 {normal['mean_return_pct']:+.1f}%，其中已到账普通及组合分红相当于初始权益的 {normal['mean_dividend_return_pct']:.1f}%；将这些分红从总损益中扣除，剩余 {normal['mean_net_ex_dividend_return_pct']:+.1f}%。平均累计成交额为初始权益的 {normal['mean_turnover_multiple']:.1f} 倍，交易费、闲置费和利息合计相当于初始权益的 {normal['mean_fees_pct_initial']:.1f}%。",
              "这只是同一场运行的账务分解。分红也会影响价格、再投资和对手行为，不能据此预测取消分红后的收益。收益并不全是短线买卖价差。", ""]
    repeat = pairs["v5_repeat"]
    lines += ["", "## 验证与限制", "",
              f"- 共 **{audits} 次账务审计**；全体账户跳过 **{skips}** 次、bot 错误 **{errors}** 次、停用 **{disabled}** 次。",
              "- cargo fmt --check、严格 Clippy 和全部 36 项 Rust 测试通过。逐局权益记录必须完整覆盖 3,000 次观测；源码、场景及两个实验可执行文件的冻结哈希在实验后重新核对一致。",
              f"- 同种子重复的默认 v5，平均排名差 {repeat['percentile_difference']:+.2f}，平均收益差 {repeat['return_difference_pp']:+.2f} 个百分点，反映多线程路径不确定性。",
              "- 95%区间为描述性 bootstrap 区间，未做多重比较校正。平均盈利不代表每局盈利，排名过线也不是实盘盈利证据。",
              "- 所有环境仍来自同一分红驱动、不能做空的模拟器；没有真实交易所延迟、完整撤单事件流或实盘逐笔回放，不能据此断言适用于所有牛熊市或现实品种。",
              "- 减去分红后的损益仅是账务分解。取消分红也会改变价格和其他策略，因此不能把这一列当作无分红世界的收益。", "",
              "## 复现与产物", "",
              "    cargo build --release --bin swift-eval",
              "    python tools/swift_stress.py prepare --out runs/swift-stress-replay",
              "    python tools/arena_gate.py --cwd . -- python tools/swift_stress.py run --out runs/swift-stress-replay",
              "    python tools/swift_stress_report.py --out runs/swift-stress-replay",
              "    python tools/swift_stress_plot.py", "",
              "使用新的输出目录，避免覆盖历史结果。脚本冻结配置、源码和二进制哈希；run 可跳过已完成批次，遇到未完成批次则保留现场并停止。统计和作图脚本不参与交易。作图另需 matplotlib；可用 python -m pip install --target runs/swift-analysis-deps matplotlib 安装到独立目录。Windows 若全局 sccache 不可用，构建时将子进程环境 RUSTC_WRAPPER 设为空。", "",
              "- [主实验预定方案](swift-stress-plan.json)",
              "- [全部逐局结果与区间](swift-stress-results.json)",
              "- [追加拥挤诊断](swift-crowding-results.json)",
              "- [可导出统计图](swift-stress-overview.svg)",
              "- 原始输出：runs/swift-stress-20261018 与 runs/swift-crowding-20261018。", ""]
    (ROOT / "docs/swift-stress.md").write_text("\n".join(lines), encoding="utf-8")
    print(f"Wrote report: {len(ws)+len(extra)} runs, {audits} audits, skips={skips}, errors={errors}, disabled={disabled}")


if __name__ == "__main__":
    main()
