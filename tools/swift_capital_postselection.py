"""Descriptive pooling of two post-selection batches; never include the matrix."""
import json
from swift_capital_report import ROOT, summarize_case, fmt_ci
from swift_stress import sha


def main():
    paths = [ROOT / "docs/swift-capital-confirmation.json", ROOT / "docs/swift-capital-cash-results.json"]
    batches = [json.loads(p.read_text(encoding="utf-8")) for p in paths]
    assert not ({r["seed"] for r in batches[0]["worlds"]} & {r["seed"] for r in batches[1]["worlds"]})
    cases = ["relay-scaled-100", "relay-scaled-300"]
    combined = [{"case": case, **summarize_case([r for b in batches for r in b["worlds"] if r["case"] == case])} for case in cases]
    data = {"sources": {p.relative_to(ROOT).as_posix(): sha(p) for p in paths},
            "note": "Post hoc descriptive pooling, not a new pre-registered test. Excludes all matrix/development seeds. Both batches were fixed without retuning the tested scaled treatments.",
            "combined": combined}
    (ROOT / "docs/swift-capital-postselection.json").write_text(json.dumps(data, ensure_ascii=False, indent=2)+"\n", encoding="utf-8")
    lines = ["# 资金实验：两批选择后数据的补充合并", "",
             "这是报告阶段的描述性汇总，不是新预注册检验。只合并 16001–16020 的独立确认和 17001–17020 的机制核验；不纳入用来挑选策略的 14000/15000 段矩阵。机制核验在确认批结果出来之前写定，scaled 100/300 的策略行为与资产配置未再调节。", "",
             "第一批确认的收益差区间跨零，第二批核验的收益差区间为正；两批结果均保留。下表各配置共 40 个独立世界、80 局（两种顺序），区间仍按种子聚类。", "",
             "| 配置 | Swift 收益 | 猎手收益 | 猎手减 Swift / 95%区间 | 猎手胜局 | Swift / 猎手平均金额损益（元） |", "|---|---:|---:|---:|---:|---:|"]
    for r in combined:
        lines.append(f"| {r['case']} | {r['target']['mean_return_pct']:+.2f}% | {r['predator']['mean_return_pct']:+.2f}% | {r['mean_advantage_pp']:+.2f} / {fmt_ci(r['advantage_ci95'])} | {r['wins']}/{r['runs']} | {r['target']['mean_pnl_yuan']:+,.0f} / {r['predator']['mean_pnl_yuan']:+,.0f} |")
    lines += ["", "在这些后续世界中，大库存大本金的猎手通常能百分比少亏；这不是盈利策略，且绝对亏损大得多。猎手 100/300 倍权益分别约为原背景总权益 1.30/3.90 倍，分别持有全市场约 51%/76% 的初始股票。", "",
              "两档 relay 各 80 局均未通过全部市场质量阈值，双边盘口覆盖率均值约 61%/60%。对照市场与无猎手市场相对完整，不能把压低流动性的攻击结果解释为一般正常市场中的稳定套利。账务审计通过、所有账户零跳过，并不意味着市场质量良好。", "",
              "只增加现金、不给大量初始库存的结果仍较弱，详见[现金机制核验](swift-capital-cash-results.md)。完整原批次分别见[资金报告](swift-capital-results.md)和[核验数据](swift-capital-cash-results.json)；[合并摘要及来源哈希](swift-capital-postselection.json)。", ""]
    (ROOT / "docs/swift-capital-postselection.md").write_text("\n".join(lines), encoding="utf-8")


if __name__ == "__main__":
    main()
