"""Report entire Swift groups, paired controls and count effects without selection."""
import argparse
import json
from pathlib import Path
import statistics as stats

from swift_crowding_hunt import BASE, ROOT
from swift_hunt_report import cluster_ci


def estimate(rows, value):
    return {"mean": stats.mean(value(r) for r in rows), "ci95": cluster_ci(rows, value)[0]}


def summarize(rows):
    result = {"runs": len(rows), "independent_seeds": len({r["seed"] for r in rows}),
              **{k: rows[0][k] for k in ["case", "swift_count", "mode", "multiple"]}}
    for key in ["return_pct", "median_return_pct", "best_return_pct", "worst_return_pct",
                "first_return_pct", "last_return_pct", "pnl_yuan", "drawdown_pct"]:
        result["group_"+key] = estimate(rows, lambda r: r["group"][key])
    result["group_loss_games"] = sum(r["group"]["return_pct"] < 0 for r in rows)
    result["all_swifts_loss_games"] = sum(r["group"]["loss_count"] == r["swift_count"] for r in rows)
    result["swift_loss_count"] = sum(r["group"]["loss_count"] for r in rows)
    result["positions"] = [{"position": i, **estimate(rows, lambda r: r["swifts"][i]["return_pct"])}
                           for i in range(result["swift_count"])]
    result["swift_rejects"] = sum(a["stats"]["rejects"] for r in rows for a in r["swifts"])
    result["swift_liquidations"] = sum(a["stats"]["liquidations"] for r in rows for a in r["swifts"])
    result["quality_pass_games"] = sum(r["quality_pass"] for r in rows)
    names = sorted({c["name"] for r in rows for c in r["quality_checks"]})
    result["quality_checks"] = {}
    for name in names:
        checks = [c for r in rows for c in r["quality_checks"] if c["name"] == name]
        values = [c["value"] for c in checks if c["value"] is not None]
        result["quality_checks"][name] = {"passes": sum(c["pass"] for c in checks),
                                          "mean": stats.mean(values) if values else None,
                                          "threshold": checks[0]["threshold"]}
    if rows[0]["predator"]:
        for key in ["return_pct", "pnl_yuan", "drawdown_pct"]:
            result["predator_"+key] = estimate(rows, lambda r: r["predator"][key])
        result["advantage_pp"] = estimate(rows, lambda r: r["group"]["advantage_pp"])
        result["beaten_share"] = estimate(rows, lambda r: r["group"]["beaten_share"])
        for key in ["predator_beats_group", "predator_beats_all", "both_lose", "both_lose_predator_wins"]:
            result[key] = sum(r["group"][key] for r in rows)
        result["group_win_rate"] = estimate(rows, lambda r: float(r["group"]["predator_beats_group"]))
        result["predator_profit_games"] = sum(r["predator"]["return_pct"] > 0 for r in rows)
        result["predator_initial_equity_yuan"] = stats.mean(r["predator"]["initial_equity"] / 100 for r in rows)
        for fee in ["trade_fees", "idle_fees", "interest"]:
            result["predator_"+fee+"_pct"] = stats.mean(r["predator"]["stats"][fee] / r["predator"]["initial_equity"] * 100 for r in rows)
        for key in ["rejects", "liquidations"]:
            result["predator_"+key] = sum(r["predator"]["stats"][key] for r in rows)
        result["direct_predator_swift_notional_yuan"] = sum(r["direct_predator_swift_notional"] for r in rows) / 100
    return result


def paired(rows, refs, values):
    lookup = {(r["seed"], r["order"]): r for r in refs}
    assert len(lookup) == len(refs)
    assert {(r["seed"], r["order"]) for r in rows} == set(lookup)
    pairs = [{"seed": r["seed"], "order": r["order"],
              **{k: value(r) - value(lookup[r["seed"], r["order"]]) for k, value in values.items()}}
             for r in rows]
    return {"runs": len(pairs), "independent_seeds": len({r["seed"] for r in pairs}),
            **{k: estimate(pairs, lambda r: r[k]) for k in values}}, pairs


def build(stage):
    plan = json.loads((BASE / stage / "plan.json").read_text(encoding="utf-8"))
    rows = []
    for job in plan["jobs"]:
        path = BASE / stage / job["id"] / "done.json"
        data = json.loads(path.read_text(encoding="utf-8"))
        assert data["job"] == job and len(data["worlds"]) == job["seeds"]
        rows += data["worlds"]
    groups = {key: [r for r in rows if r["case"] == key] for key in dict.fromkeys(r["case"] for r in rows)}
    combined = [summarize(group) for group in groups.values()]
    by_order = [{"order": order, **summarize([r for r in group if r["order"] == order])}
                for group in groups.values() for order in sorted({r["order"] for r in group})]
    values = {"group_return_change_pp": lambda r: r["group"]["return_pct"],
              "predator_return_change_pp": lambda r: r["predator"]["return_pct"],
              "advantage_change_pp": lambda r: r["group"]["advantage_pp"]}
    controls, counts, difference_in_differences, alone = [], [], [], []
    for key, group in groups.items():
        n, mode, m = group[0]["swift_count"], group[0]["mode"], group[0]["multiple"]
        if mode == "relay":
            ref = f"n{n}-cash-{m}"
            delta, pairs = paired(group, groups[ref], values)
            controls.append({"case": key, "reference": ref, **delta})
            if n != 1:
                _, one_pairs = paired(groups[f"n1-relay-{m}"], groups[f"n1-cash-{m}"], values)
                diff, _ = paired(pairs, one_pairs, {k: lambda r, k=k: r[k] for k in values})
                difference_in_differences.append({"case": key, "reference": f"n1-relay-{m}", **diff})
        if n != 1:
            ref = f"n1-{mode}-{m}" if mode else "n1-alone"
            use_values = values if mode else {"group_return_change_pp": values["group_return_change_pp"]}
            delta, _ = paired(group, groups[ref], use_values)
            (counts if mode else alone).append({"case": key, "reference": ref, **delta})
    clear = [r["case"] for r in combined if r["mode"] == "relay"
             and r["advantage_pp"]["ci95"][0] > 0 and r["predator_beats_group"] > r["runs"] / 2
             and all(b["advantage_pp"]["mean"] > 0 for b in by_order if b["case"] == r["case"])]
    return {"stage": stage, "plan": plan, "runs": len(rows), "combined": combined,
            "by_order": by_order, "relay_minus_cash": controls, "count_minus_one": counts,
            "alone_count_minus_one": alone, "count_effect_minus_cash_count_effect": difference_in_differences,
            "clear_relative_advantage": clear, "worlds": rows,
            "audit_totals": {k: sum(r[k] for r in rows) for k in ["audits", "all_skipped", "all_errors", "all_disabled"]}}


def fmt(x):
    return f"{x['mean']:+.2f} [{x['ci95'][0]:+.2f}, {x['ci95'][1]:+.2f}]"


def write_markdown(data, path):
    lines = ["# 多 Swift 是否更容易狩猎", "", f"完成冻结的 {data['runs']} 局；全部数量、两种顺序和对照均保留。方案见[实验协议](swift-crowding-hunt-protocol.md)。",
             "", "![数量与狩猎效果](swift-crowding-hunt-overview.png)", "",
             "## 直接结果", "", "所有收益含费用、分红与终值。猎手本金始终相对于单个 Swift；增加 Swift 数量不会增加猎手本金。两种顺序合并按世界种子聚类，40 局是 20 个独立种子。", "",
             "| Swift 数 | 猎手 | 本金倍数 | Swift 组收益% | 猎手收益% | 胜过组均值 | 胜过全部 | 平均胜过个体比例 | 双亏少亏 |", "|---:|---|---:|---:|---:|---:|---:|---:|---:|"]
    for r in data["combined"]:
        if r["mode"] is None:
            lines.append(f"| {r['swift_count']} | 无 | — | {r['group_return_pct']['mean']:+.2f} | — | — | — | — | — |")
        else:
            lines.append(f"| {r['swift_count']} | {r['mode']} | {r['multiple']} | {r['group_return_pct']['mean']:+.2f} | {r['predator_return_pct']['mean']:+.2f} | {r['predator_beats_group']}/{r['runs']} | {r['predator_beats_all']}/{r['runs']} | {r['beaten_share']['mean']*100:.1f}% | {r['both_lose_predator_wins']}/{r['both_lose']} |")
    lines += ["", "满足预先约定的明确相对优势门槛：" + ("、".join(data["clear_relative_advantage"]) or "无") + "。这只指本实验范围，不代表任何市场都有效。", "",
              "## 收益分布和顺序", "", "| 配置 | 猎手减 Swift 组，百分点 / 95%区间 | Swift 组收益 / 95%区间 | Swift 中位 / 最好 / 最差均值 | 首位 / 末位均值 | 所有 Swift 均亏局数 |", "|---|---:|---:|---:|---:|---:|"]
    for r in data["combined"]:
        gap = fmt(r["advantage_pp"]) if r["mode"] else "—"
        lines.append(f"| {r['case']} | {gap} | {fmt(r['group_return_pct'])} | {r['group_median_return_pct']['mean']:+.2f} / {r['group_best_return_pct']['mean']:+.2f} / {r['group_worst_return_pct']['mean']:+.2f} | {r['group_first_return_pct']['mean']:+.2f} / {r['group_last_return_pct']['mean']:+.2f} | {r['all_swifts_loss_games']}/{r['runs']} |")
    lines += ["", "组中位数/最好/最差先在各局内取值，再跨局平均；不是拿跨局平均收益重新排序。完整逐位置与两顺序数据见 JSON。以下不把首位和末位混成同一种执行机会。", "",
              "| 配置 | 顺序 | Swift 组收益% | 猎手收益% | 胜过组均值 |", "|---|---|---:|---:|---:|"]
    for r in data["by_order"]:
        if r["mode"] == "relay":
            lines.append(f"| {r['case']} | {r['order']} | {r['group_return_pct']['mean']:+.2f} | {r['predator_return_pct']['mean']:+.2f} | {r['predator_beats_group']}/{r['runs']} |")
    for key, title in [("relay_minus_cash", "relay 相对同资金清仓对照"), ("count_minus_one", "增加 Swift 相对只有一个"), ("count_effect_minus_cash_count_effect", "数量效应扣除 cash 的数量效应")]:
        lines += ["", f"## {title}", "", "| 配置 | 参照 | Swift 组变化 / 95%区间 | 猎手变化 / 95%区间 | 相对收益变化 / 95%区间 |", "|---|---|---:|---:|---:|"]
        for r in data[key]:
            lines.append(f"| {r['case']} | {r['reference']} | {fmt(r['group_return_change_pp'])} | {fmt(r['predator_return_change_pp'])} | {fmt(r['advantage_change_pp'])} |")
    lines += ["", "变化单位都是百分点。按同种子与同注册顺序配对，但行动会改变之后的市场路径。最后一表为差分之差；正的相对收益变化表示 relay 的增量优势随人数增大。它不能消除增加小账户同时增加市场资本/股票的影响。", "",
              "## 无猎手时的自身拥挤", "", "| 数量对比 | Swift 组收益变化 / 95%区间 |", "|---|---:|"]
    for r in data["alone_count_minus_one"]:
        lines.append(f"| {r['case']} − {r['reference']} | {fmt(r['group_return_change_pp'])} |")
    lines += ["", "## 金额、回撤、执行与市场质量", "", "| 配置 | 猎手初始权益（元） | Swift 全组损益（元） | 猎手损益（元） | Swift 组 / 猎手平均回撤% | 猎手交易费% / 闲置费% | 市场质量通过 |", "|---|---:|---:|---:|---:|---:|---:|"]
    for r in data["combined"]:
        if r["mode"]:
            lines.append(f"| {r['case']} | {r['predator_initial_equity_yuan']:,.0f} | {r['group_pnl_yuan']['mean']:+,.0f} | {r['predator_pnl_yuan']['mean']:+,.0f} | {r['group_drawdown_pct']['mean']:.1f} / {r['predator_drawdown_pct']['mean']:.1f} | {r['predator_trade_fees_pct']:.2f} / {r['predator_idle_fees_pct']:.2f} | {r['quality_pass_games']}/{r['runs']} |")
    a = data["audit_totals"]
    rejects = sum(r["swift_rejects"] + r.get("predator_rejects", 0) for r in data["combined"])
    liquidations = sum(r["swift_liquidations"] + r.get("predator_liquidations", 0) for r in data["combined"])
    direct = sum(r.get("direct_predator_swift_notional_yuan", 0) for r in data["combined"])
    lines += ["", f"账务审计 {a['audits']} 次；所有账户跳过 {a['all_skipped']}、错误 {a['all_errors']}、停用 {a['all_disabled']}。所有 Swift 与猎手拒单 {rejects}、强平 {liquidations}；猎手与全部 Swift 的直接成交金额合计 {direct:,.2f} 元。",
              "", "市场质量门槛与账务正确性分开；所有失败世界保留。区间按种子聚类 bootstrap 3000 次，未作多重比较校正。回撤是观察到的标记权益回撤，不保证可按该价格退出。百分比少亏不等于金额少亏，也不等于猎手赚钱。",
              "", "数据：[完整 JSON](swift-crowding-hunt-results.json)；[冻结计划](swift-crowding-hunt-plan.json)。原始撮合与权益曲线在 runs/swift-crowding-hunt-20261018，未提交大体积逐笔文件。", ""]
    path.write_text("\n".join(lines), encoding="utf-8")


if __name__ == "__main__":
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--stage", default="validation")
    p.add_argument("--output", type=Path, default=ROOT / "docs/swift-crowding-hunt-results.json")
    p.add_argument("--markdown", type=Path, default=ROOT / "docs/swift-crowding-hunt-results.md")
    a = p.parse_args()
    data = build(a.stage)
    a.output.write_text(json.dumps(data, ensure_ascii=False, indent=2)+"\n", encoding="utf-8")
    write_markdown(data, a.markdown)
    for r in data["combined"]:
        print(r["case"], round(r["group_return_pct"]["mean"], 2),
              round(r.get("predator_return_pct", {}).get("mean", 0), 2),
              r.get("predator_beats_group"), r.get("advantage_pp"))
    print("clear_relative_advantage", data["clear_relative_advantage"])
    print("audits", data["audit_totals"])
