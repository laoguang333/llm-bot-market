"""Report all capital treatments, including controls and unsuccessful candidates."""
import argparse
import json
from pathlib import Path
import statistics as stats

from swift_capital import BASE, ROOT, MODES, STRUCTURES
from swift_hunt_report import aggregate, cluster_ci

LABELS = {"clock": "定时买卖", "relay": "信号反馈", "pursuit": "提前同向", "cash": "卖出持现金", "blind": "固定仓位"}
ALLOCATIONS = {"base": "共同起点", "reserve": "仅增加备用本金", "scaled": "同时放大交易", "cash_active": "新增现金并放大交易"}


def effect(rows, reference):
    lookup = {(r["seed"], r["order"]): r for r in reference}
    pairs = []
    for row in rows:
        base = lookup.get((row["seed"], row["order"]))
        if base is None:
            continue
        pairs.append({"seed": row["seed"], **{
            f"{owner}_{metric}": row[owner][metric] - base[owner][metric]
            for owner in ["target", "predator"] for metric in ["return_pct", "pnl_yuan"]}})
    if not pairs:
        return None
    result = {"runs": len(pairs), "independent_seeds": len({r["seed"] for r in pairs})}
    for key in pairs[0]:
        if key == "seed":
            continue
        result[key] = stats.mean(r[key] for r in pairs)
        result[key + "_ci95"], _ = cluster_ci(pairs, lambda r: r[key])
    return result


def summarize_case(rows):
    result = aggregate(rows)
    result["market_quality_passes"] = sum(r["quality_pass"] for r in rows)
    names = {c["name"] for r in rows for c in r.get("quality_checks", [])}
    result["quality_checks"] = {}
    for name in sorted(names):
        checks = [c for r in rows for c in r.get("quality_checks", []) if c["name"] == name]
        values = [c["value"] for c in checks if c["value"] is not None]
        result["quality_checks"][name] = {"passes": sum(c["pass"] for c in checks),
                                          "mean": stats.mean(values) if values else None,
                                          "threshold": checks[0]["threshold"]}
    if rows[0]["predator"]:
        result["win_rate_ci95"], _ = cluster_ci(rows, lambda r: float(r["predator"]["return_pct"] > r["target"]["return_pct"]))
        result["target_loss_rate_ci95"], _ = cluster_ci(rows, lambda r: float(r["target"]["return_pct"] < 0))
        result["mean_wealth_relative_to_background"] = stats.mean(r["capital_setup"]["predator_initial_equity"] / r["capital_setup"]["background_initial_equity"] for r in rows)
        result["mean_initial_stock_share"] = stats.mean(r["predator_initial_stock_share"] for r in rows)
        for owner in ["target", "predator"]:
            result[owner]["mean_initial_equity_yuan"] = stats.mean(r[owner]["initial_equity"] / 100 for r in rows)
            result[owner]["mean_turnover_yuan"] = stats.mean(r[owner]["stats"]["volume_notional"] / 100 for r in rows)
    return result


def build(stages, partial=False):
    rows, plans = [], []
    for stage in stages:
        plan = json.loads((BASE / stage / "plan.json").read_text(encoding="utf-8"))
        plans.append({k: v for k, v in plan.items() if k != "jobs"})
        for job in plan["jobs"]:
            path = BASE / stage / job["id"] / "done.json"
            if not path.exists():
                if partial:
                    continue
                raise SystemExit(f"Missing {path}")
            data = json.loads(path.read_text(encoding="utf-8"))
            for row in data["worlds"]:
                summary = json.loads((path.parent / f"seed-{row['seed']}" / "summary.json").read_text(encoding="utf-8"))
                predator = next((a for a in summary["accounts"] if a["owner"] == "predator"), None)
                stock_share = (sum(predator["initial_shares"]) / sum(sum(a["initial_shares"]) for a in summary["accounts"])) if predator else None
                rows.append({"stage": stage, **row, "quality_checks": summary["quality"]["checks"],
                             "predator_initial_stock_share": stock_share})
    cases = list(dict.fromkeys(r["case"] for r in rows))
    combined, by_stage, by_order, comparisons, structure_effects, cash_effects = [], [], [], [], [], []
    for case in cases:
        subset = [r for r in rows if r["case"] == case]
        meta = {k: subset[0][k] for k in ["case", "mode", "multiple", "allocation"]}
        combined.append({**meta, **summarize_case(subset)})
        for stage in stages:
            chosen = [r for r in subset if r["stage"] == stage]
            if chosen:
                by_stage.append({**meta, "stage": stage, **summarize_case(chosen)})
        for order in sorted({r["order"] for r in subset}):
            by_order.append({**meta, "order": order, **summarize_case([r for r in subset if r["order"] == order])})
        if case == "alone":
            continue
        for control in ["cash", "blind"]:
            allocation = "reserve" if meta["allocation"] == "cash_active" and control == "cash" else meta["allocation"]
            ref = f"{control}-{allocation}-{meta['multiple']}"
            if case != ref:
                delta = effect(subset, [r for r in rows if r["case"] == ref])
                if delta:
                    comparisons.append({"case": case, "reference": ref, **delta})
        if meta["allocation"] == "scaled":
            ref = f"{meta['mode']}-reserve-{meta['multiple']}"
            delta = effect(subset, [r for r in rows if r["case"] == ref])
            if delta:
                structure_effects.append({"case": case, "reference": ref, **delta})
        if meta["allocation"] == "cash_active":
            ref = f"{meta['mode']}-scaled-{meta['multiple']}"
            delta = effect(subset, [r for r in rows if r["case"] == ref])
            if delta:
                cash_effects.append({"case": case, "reference": ref, **delta})
    stable, harmful, eligible = [], [], []
    for r in combined:
        if r["mode"] not in ["clock", "relay", "pursuit"]:
            continue
        batches = [b for b in by_stage if b["case"] == r["case"]]
        if all(b["mean_advantage_pp"] > 0 and b["wins"] > b["runs"] / 2 for b in batches) and r["advantage_ci95"][0] > 0:
            stable.append(r["case"])
        if all(b["target"]["mean_return_pct"] < 0 for b in batches) and r["target"]["return_ci95"][1] < 0:
            harmful.append(r["case"])
        if r["mean_advantage_pp"] > 0 or r["target"]["mean_return_pct"] < 0:
            eligible.append(r)
    selected = [r["case"] for r in sorted(eligible, key=lambda r: (-r["mean_advantage_pp"], r["case"]))[:3]]
    return {"stages": stages, "plans": plans, "runs": len(rows), "combined": combined,
            "by_stage": by_stage, "by_order": by_order, "comparisons": comparisons,
            "scaled_minus_reserve": structure_effects, "cash_active_minus_scaled": cash_effects, "stable_relative": stable,
            "stable_target_loss": harmful, "confirmation_selection": selected,
            "worlds": rows, "audit_totals": {k: sum(r[k] for r in rows) for k in ["audits", "all_skipped", "all_errors", "all_disabled"]},
            "notes": ["Two registration orders are clustered by world seed; they are not independent worlds.",
                      "Actual executions and prices differ across treatments and repeated runs.",
                      "Returns include dividends, terminal settlement and fees. No annualization.",
                      "Intervals are descriptive and not corrected for all candidate comparisons."]}


def fmt_ci(ci):
    return f"[{ci[0]:+.2f}, {ci[1]:+.2f}]"


def write_markdown(data, path):
    total = data["runs"]
    rows = data["combined"]
    by = {r["case"]: r for r in rows}
    independent = len({r["seed"] for r in data["worlds"]})
    text = ["# Swift 狩猎者资金优势实验结果", "", f"矩阵完成 {total:,} 局、{independent} 个新种子；所有配置和两种注册顺序完整报告。", "",
            "以下首先是矩阵内筛选，不能替代独立确认；最终判断请同时阅读文末的追加确认，以及[新增现金机制核验](swift-capital-cash-results.md)。多目标问题另见[多 Swift 实验](swift-crowding-hunt-results.md)。", "",
            f"达到批次内相对胜出门槛：{'、'.join(data['stable_relative']) or '无'}。",
            f"达到使 Swift 平均收益稳定为负的门槛：{'、'.join(data['stable_target_loss']) or '无'}。", "",
            "这里的门槛和追加确认规则见[冻结方案](swift-capital-protocol.md)。两种注册顺序按种子聚类；不能把同一个种子的两局当作两个独立市场。", "",
            "![资金规模实验](swift-capital-overview.png)", "",
            "## 本金、仓位和费用", "",
            "Swift 不变。对手初始权益恰好为 Swift 的 10、30、100 或 300 倍。备用组保持每只股票 200 股和 10 倍权益的固定交易额度，新增部分全为现金；放大组按倍率增加初始股票、交易额度与 clock / relay 的每次股数。10 倍为共同起点，仅运行一次。", "",
            "所有本金在同一个账户内，从开局承担费用并计入收益率分母。有效仓位额度为固定初始分配与当时实际权益的较小值；不随盈利增加。收益包含分红、终值及全部费用，不是纯价差利润或年化。", "",
            "clock 定时交易，relay 推测 Swift 开关，pursuit 用更早的阈值同向进出；cash 尝试卖出开局股票后持现金，blind 保持固定仓位目标。后两项是对照，不包装成识别目标的策略。", "",
            "## 全部配置", "",
            "| 结构 | 倍率 | 策略 | Swift 平均收益 | 对手平均收益 | 收益率差 | 对手胜局 | Swift 亏损局 | 双亏时对手少亏 |",
            "|---|---:|---|---:|---:|---:|---:|---:|---:|"]
    if "alone" in by:
        a = by["alone"]
        text.append(f"| 无新增对手 | — | alone | {a['target']['mean_return_pct']:+.2f}% | — | — | — | {a['target']['losses']}/{a['runs']} | — |")
    for r in rows:
        if r["mode"] is None:
            continue
        text.append(f"| {ALLOCATIONS[r['allocation']]} | {r['multiple']} | {LABELS[r['mode']]} | {r['target']['mean_return_pct']:+.2f}% | {r['predator']['mean_return_pct']:+.2f}% | {r['mean_advantage_pp']:+.2f} | {r['wins']}/{r['runs']} | {r['target']['losses']}/{r['runs']} | {r['both_lose_predator_wins']}/{r['both_lose']} |")
    text += ["", "收益率差为对手减 Swift，单位为百分点；最后一列是事后双亏子集，不是总体胜率。", "",
             "## 相对于整个市场的规模", "", "| 结构 | 倍率 | 每只股票初始股数 | 占全市场初始股票数量 | 对手权益 / 原背景权益 |", "|---|---:|---:|---:|---:|"]
    for allocation, multiple in STRUCTURES:
        key = f"cash-{allocation}-{multiple}"
        if key in by:
            r = by[key]
            shares = 200 if allocation in ["base", "reserve"] else 20 * multiple
            text.append(f"| {ALLOCATIONS[allocation]} | {multiple} | {shares} | {r['mean_initial_stock_share']*100:.1f}% | {r['mean_wealth_relative_to_background']*100:.1f}% |")
    text += ["", "放大股票端会改变流通供给；若卖出持现金对照也使 Swift 亏损，便不能把全部效果解释为利用 Swift 参数的优势。备用组保持股票数量不变，回答的是新增现金的作用。", "",
             "## 批次与不确定性", "", "| 配置 | 收益率差95%区间 | Swift 收益95%区间 | 各批差值 / 胜局 |", "|---|---:|---:|---|"]
    for r in rows:
        if r["mode"] not in ["clock", "relay", "pursuit"]:
            continue
        batch = "; ".join(f"{b['stage']}: {b['mean_advantage_pp']:+.2f} / {b['wins']}/{b['runs']}" for b in data["by_stage"] if b["case"] == r["case"])
        text.append(f"| {r['case']} | {fmt_ci(r['advantage_ci95'])} | {fmt_ci(r['target']['return_ci95'])} | {batch} |")
    text += ["", "区间为按种子聚类的 3,000 次 bootstrap，先在种子内平均两种顺序。未对全部配置作多重比较校正；若有候选进入独立确认，确认结果另报，不能用矩阵内择优结果替代。", "",
             "## 相对于同资金对照", "", "| 配置 | 对照 | Swift 收益变化 / 95%区间 | 对手收益变化 | 对手绝对损益变化（元） |", "|---|---|---:|---:|---:|"]
    for r in data["comparisons"]:
        if by[r["case"]]["mode"] not in ["clock", "relay", "pursuit"]:
            continue
        text.append(f"| {r['case']} | {r['reference']} | {r['target_return_pct']:+.2f} / {fmt_ci(r['target_return_pct_ci95'])} | {r['predator_return_pct']:+.2f} | {r['predator_pnl_yuan']:+,.0f} |")
    text += ["", "配对使用相同种子与注册顺序，但交易会改变市场路径。现金对照会卖初始库存；放大组增加股票供给和分红权，因此目标亏损可能并非信号识别的效果。损益差是对照结果，不是能单独结算的攻击费用。", "",
             "## 绝对损益、交易量和费用", "", "| 配置 | 对手初始权益（元） | 占背景总权益 | Swift 损益（元） | 对手损益（元） | 对手总成交额（元） | 交易费% / 闲置费% / 利息% |", "|---|---:|---:|---:|---:|---:|---:|"]
    for r in rows:
        if r["mode"] is None:
            continue
        p, t = r["predator"], r["target"]
        text.append(f"| {r['case']} | {p['mean_initial_equity_yuan']:,.0f} | {100*r['mean_wealth_relative_to_background']:.1f}% | {t['mean_pnl_yuan']:+,.0f} | {p['mean_pnl_yuan']:+,.0f} | {p['mean_turnover_yuan']:,.0f} | {p['trade_fees_pct']:.2f} / {p['idle_fees_pct']:.2f} / {p['interest_pct']:.2f} |")
    text += ["", "百分比少亏可以伴随金额多亏。把本金放大但限制投入，也会稀释损益百分比；这符合本次规则，但不能据此说策略更善于交易。全部费用已包含于收益率。", "",
             "## 回撤、执行与质量", "", "| 配置 | Swift 平均 / 最差回撤% | 对手平均 / 最差回撤% | 拒单 Swift / 对手 | 强平 Swift / 对手 | 市场质量通过局数 |", "|---|---:|---:|---:|---:|---:|"]
    for r in rows:
        if r["mode"] is None:
            continue
        t, p = r["target"], r["predator"]
        text.append(f"| {r['case']} | {t['mean_drawdown_pct']:.1f} / {t['worst_drawdown_pct']:.1f} | {p['mean_drawdown_pct']:.1f} / {p['worst_drawdown_pct']:.1f} | {t['rejects']} / {p['rejects']} | {t['liquidations']} / {p['liquidations']} | {r['market_quality_passes']}/{r['runs']} |")
    a = data["audit_totals"]
    text += ["", f"共 {a['audits']:,} 次账务审计；全账户跳过 {a['all_skipped']}、错误 {a['all_errors']}、停用 {a['all_disabled']}。回撤用决策前标记权益和最终结算计算，不保证能按标记价格清仓。", "",
             "市场质量通过率独立于账务正确性；检查双边盘口、价差、深度、价格偏离等。原版背景本身也可能不通过全部质量阈值。所有世界均保留，不按质量结果筛选有利样本；逐项检查值写入 JSON。", "",
             "原始各世界、分顺序统计、放大交易减备用本金的配对结果、费用和初始资产记录见 [JSON](swift-capital-results.json)。原始撮合记录保存在被 git 忽略的 runs/swift-capital-20261018；冻结计划保存源码、配置和二进制哈希。", "",
             "## 追加确认选择", "", "按冻结规则进入追加确认的配置：" + ("、".join(data["confirmation_selection"]) or "无；没有策略达到触发条件。"), ""]
    path.write_text("\n".join(text), encoding="utf-8")


def append_confirmation(matrix, confirmation, path):
    selected = matrix["confirmation_selection"]
    by = {r["case"]: r for r in confirmation["combined"]}
    assert all(case in by for case in selected)
    old_seeds = {r["seed"] for r in matrix["worlds"]}
    new_seeds = {r["seed"] for r in confirmation["worlds"]}
    assert not old_seeds & new_seeds
    lines = ["", "## 独立新市场确认", "",
             f"依据矩阵选择规则冻结 {len(selected)} 个候选后，追加 {confirmation['runs']} 局，使用 {len(new_seeds)} 个未参与选择的新种子。对照与候选一同运行，参数和代码不改；这一批单独报告，不并回矩阵冒充未选择样本。", "",
             "独立确认达到平均相对收益优势门槛：" + ("、".join(confirmation['stable_relative']) or "无") + "。",
             "独立确认使 Swift 平均收益显著为负：" + ("、".join(confirmation['stable_target_loss']) or "无") + "。",
             "胜局比例高与平均收益领先确定是两件事；少数 Swift 大幅盈利局可以使平均收益差区间跨零。不能把多数局少亏写成已经证明稳定的期望优势。", "",
             "| 候选 | Swift 平均收益 / 95%区间 | 对手平均收益 | 收益率差 / 95%区间 | 对手胜局 | Swift 亏损局 | 双亏时对手少亏 |",
             "|---|---:|---:|---:|---:|---:|---:|"]
    for case in selected:
        r = by[case]
        lines.append(f"| {case} | {r['target']['mean_return_pct']:+.2f}% / {fmt_ci(r['target']['return_ci95'])} | {r['predator']['mean_return_pct']:+.2f}% | {r['mean_advantage_pp']:+.2f} / {fmt_ci(r['advantage_ci95'])} | {r['wins']}/{r['runs']} | {r['target']['losses']}/{r['runs']} | {r['both_lose_predator_wins']}/{r['both_lose']} |")
    lines += ["", "| 候选 | 对照 | Swift 收益变化 / 95%区间 | 对手收益变化 | 对手绝对损益变化（元） |", "|---|---|---:|---:|---:|"]
    for r in confirmation["comparisons"]:
        if r["case"] in selected:
            lines.append(f"| {r['case']} | {r['reference']} | {r['target_return_pct']:+.2f} / {fmt_ci(r['target_return_pct_ci95'])} | {r['predator_return_pct']:+.2f} | {r['predator_pnl_yuan']:+,.0f} |")
    lines += ["", "| 配置 | Swift 收益 | 对手收益 | 对手胜局 | Swift 损益（元） | 对手损益（元） | 交易费% / 闲置费% / 利息% |", "|---|---:|---:|---:|---:|---:|---:|"]
    for r in confirmation["combined"]:
        if r["mode"] is None:
            continue
        t, p = r["target"], r["predator"]
        lines.append(f"| {r['case']} | {t['mean_return_pct']:+.2f}% | {p['mean_return_pct']:+.2f}% | {r['wins']}/{r['runs']} | {t['mean_pnl_yuan']:+,.0f} | {p['mean_pnl_yuan']:+,.0f} | {p['trade_fees_pct']:.2f} / {p['idle_fees_pct']:.2f} / {p['interest_pct']:.2f} |")
    a = confirmation["audit_totals"]
    rejects = sum(r[o]["rejects"] for r in confirmation["combined"] for o in ["target", "predator"] if o in r)
    liquidations = sum(r[o]["liquidations"] for r in confirmation["combined"] for o in ["target", "predator"] if o in r)
    lines += ["", f"确认批次共 {a['audits']} 次账务审计，全账户跳过 {a['all_skipped']}、错误 {a['all_errors']}、停用 {a['all_disabled']}；目标与对手共拒单 {rejects}、强平 {liquidations}。",
              "逐世界数据、双顺序结果、费用与市场质量见[独立确认 JSON](swift-capital-confirmation.json)。", ""]
    with path.open("a", encoding="utf-8") as f:
        f.write("\n".join(lines))


if __name__ == "__main__":
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--stages", nargs="+", default=["matrix-a", "matrix-b"])
    p.add_argument("--partial", action="store_true")
    p.add_argument("--output", type=Path)
    p.add_argument("--markdown", type=Path)
    p.add_argument("--confirmation", type=Path)
    a = p.parse_args()
    data = build(a.stages, a.partial)
    if a.output:
        a.output.write_text(json.dumps(data, ensure_ascii=False, indent=2)+"\n", encoding="utf-8")
    if a.markdown:
        write_markdown(data, a.markdown)
        if a.confirmation:
            append_confirmation(data, json.loads(a.confirmation.read_text(encoding="utf-8")), a.markdown)
    for r in data["combined"]:
        print(r["case"], r["runs"], round(r["target"]["mean_return_pct"], 2),
              round(r.get("predator", {}).get("mean_return_pct", 0), 2), r.get("wins"),
              round(r.get("mean_advantage_pp", 0), 2))
    print("confirmation selection", data["confirmation_selection"])
    print("audit", data["audit_totals"])
