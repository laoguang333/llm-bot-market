"""Standalone research figure for the complete capital matrix."""
import json
import os
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "runs/swift-analysis-deps"))
os.environ.setdefault("MPLCONFIGDIR", str(ROOT / "runs/swift-mpl-cache"))
import matplotlib
matplotlib.use("Agg")
import matplotlib.pyplot as plt
from matplotlib import font_manager
from swift_capital_report import LABELS


def main():
    data = json.loads((ROOT / "docs/swift-capital-results.json").read_text(encoding="utf-8"))
    font = Path("C:/Windows/Fonts/msyh.ttc")
    font_manager.fontManager.addfont(str(font))
    plt.rcParams.update({"font.family": font_manager.FontProperties(fname=str(font)).get_name(),
                         "font.size": 11, "axes.unicode_minus": False,
                         "axes.spines.top": False, "axes.spines.right": False,
                         "figure.facecolor": "#f6f8fb", "axes.facecolor": "#f6f8fb",
                         "savefig.facecolor": "#f6f8fb", "text.color": "#1c2b43"})
    by = {r["case"]: r for r in data["combined"]}
    fig, axes = plt.subplots(2, 2, figsize=(14, 10), sharex=True, sharey="row")
    colors = {"clock": "#087f8c", "relay": "#b44b40", "pursuit": "#6950b3", "cash": "#7b8493", "blind": "#a9acb3"}
    for j, allocation in enumerate(["reserve", "scaled"]):
        for i, (owner, label) in enumerate([("target", "Swift 平均收益率（%）"), ("advantage", "对手收益率 − Swift（百分点）")]):
            ax = axes[i, j]
            ax.axhline(0, color="#303c50", lw=1)
            ax.grid(axis="y", alpha=.18)
            for mode in colors:
                points = [by[f"{mode}-{'base' if m == 10 else allocation}-{m}"] for m in [10, 30, 100, 300]]
                means = [r["mean_advantage_pp"] if owner == "advantage" else r[owner]["mean_return_pct"] for r in points]
                intervals = [r["advantage_ci95"] if owner == "advantage" else r[owner]["return_ci95"] for r in points]
                lo = [ci[0] for ci in intervals]
                hi = [ci[1] for ci in intervals]
                ax.plot(range(4), means, marker="o" if mode not in ["cash", "blind"] else None,
                        color=colors[mode], linestyle="--" if mode in ["cash", "blind"] else "-",
                        lw=2, label=LABELS[mode])
                if mode not in ["cash", "blind"]:
                    ax.fill_between(range(4), lo, hi, color=colors[mode], alpha=.10)
            ax.set_ylabel(label)
            ax.set_xticks(range(4), ["10 倍", "30 倍", "100 倍", "300 倍"])
            ax.tick_params(length=0, pad=7)
            if i == 0:
                ax.set_title("增加备用本金，交易额度不变" if allocation == "reserve" else "本金、初始股票、交易额度一起增加", loc="left", fontweight="bold", pad=16)
            if i == 1:
                ax.set_xlabel("对手初始权益 / Swift 初始权益")
    handles, labels = axes[0, 0].get_legend_handles_labels()
    fig.legend(handles, labels, loc="upper left", bbox_to_anchor=(.06, .90), ncol=5, frameon=False)
    fig.suptitle("更有钱，能否让单一狩猎者胜过 Swift？", x=.06, y=.98, ha="left", fontsize=22, fontweight="bold")
    fig.text(.06, .923, "固定 Swift 与 v5 规则 · 20 个新种子 × 两种顺序 · 1,420 局完整矩阵", fontsize=12, color="#526277")
    fig.text(.06, .035, "阴影：按种子聚类的 95% 区间。10 倍是两组共同起点，只测一次。所有费用、分红和终值计入收益。\n"
             "上图小于 0：Swift 平均亏损。下图大于 0：对手平均收益率超过 Swift。放大组同时增加股票供给。", fontsize=10, color="#526277")
    fig.subplots_adjust(left=.08, right=.97, top=.80, bottom=.13, wspace=.17, hspace=.21)
    path = ROOT / "docs/swift-capital-overview.png"
    fig.savefig(path, dpi=160)
    svg = path.with_suffix(".svg")
    fig.savefig(svg)
    svg.write_text("\n".join(line.rstrip() for line in svg.read_text(encoding="utf-8").splitlines())+"\n", encoding="utf-8")
    print(path)


if __name__ == "__main__":
    main()
