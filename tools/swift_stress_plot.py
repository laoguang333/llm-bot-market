"""Export a standalone research figure from the frozen stress results.

Requires matplotlib. An optional isolated install in runs/swift-analysis-deps
is discovered automatically; no global Python environment is modified.
"""
import argparse
import json
import os
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[1]
local_deps = ROOT / "runs/swift-analysis-deps"
if local_deps.exists():
    sys.path.insert(0, str(local_deps))
os.environ.setdefault("MPLCONFIGDIR", str(ROOT / "runs/swift-mpl-cache"))
import matplotlib
matplotlib.use("Agg")
import matplotlib.pyplot as plt
from matplotlib import font_manager
import numpy as np


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--input", type=Path, default=ROOT / "docs/swift-stress-results.json")
    parser.add_argument("--out", type=Path, default=ROOT / "docs/swift-stress-overview.png")
    args = parser.parse_args()
    report = json.loads(args.input.read_text(encoding="utf-8"))
    font = Path("C:/Windows/Fonts/msyh.ttc")
    if font.exists():
        font_manager.fontManager.addfont(str(font))
        plt.rcParams["font.family"] = font_manager.FontProperties(fname=str(font)).get_name()
    plt.rcParams.update({"font.size": 11, "axes.unicode_minus": False,
                         "axes.spines.top": False, "axes.spines.right": False,
                         "axes.spines.left": False, "axes.spines.bottom": False,
                         "savefig.facecolor": "#f7f9fc", "figure.facecolor": "#f7f9fc",
                         "axes.facecolor": "#f7f9fc", "text.color": "#172b4d",
                         "axes.labelcolor": "#42526e", "xtick.color": "#42526e",
                         "ytick.color": "#172b4d"})
    by_case = {x["case"]: x for x in report["combined"]}
    selected = ["v5", "calm", "no_news", "turbulent", "public_news", "no_grid", "strong_crowd",
                "crowded", "fee50", "capital01", "capital10", "capital100", "cadence5", "cadence20",
                "flow_only", "no_signal_exit"]
    rows = [by_case[k] for k in selected if k in by_case]
    y = np.arange(len(rows))
    fig, axes = plt.subplots(1, 3, figsize=(15.8, max(6.5, len(rows)*.43+2.1)),
                             sharey=True, gridspec_kw={"width_ratios": [1.05, 1.45, .9]})
    colors = ["#007f73" if r["gate_pass"] else "#bd4545" for r in rows]
    for ax in axes:
        ax.set_ylim(len(rows)-.35, -.65)
        ax.set_yticks(y)
        ax.tick_params(axis="both", length=0, pad=8)
        ax.grid(axis="x", color="#dce3eb", linewidth=.8, zorder=0)
        for i in range(len(rows)):
            if i % 2 == 0:
                ax.axhspan(i-.45, i+.45, color="#edf1f6", zorder=0)
    for i, (row, color) in enumerate(zip(rows, colors)):
        for ax, mean_key, ci_key in [(axes[0], "mean_percentile", "percentile_mean_ci95"),
                                     (axes[1], "mean_return_pct", "return_mean_ci95")]:
            mean = row[mean_key]
            lo, hi = row[ci_key]
            ax.errorbar(mean, i, xerr=[[max(0, mean-lo)], [max(0, hi-mean)]],
                        fmt="o", color=color, capsize=3, linewidth=1.8, markersize=6, zorder=3)
        axes[2].plot([row["mean_drawdown_pct"], row["max_drawdown_pct"]], [i, i],
                     color="#a8b4c5", linewidth=2, zorder=2)
        axes[2].scatter(row["mean_drawdown_pct"], i, s=32, color="#435a7a", zorder=3)
        axes[2].scatter(row["max_drawdown_pct"], i, s=28, marker="x", color="#bd4545", zorder=3)
    axes[0].set_yticklabels([f"{r['label']}  n={r['worlds']}" for r in rows])
    axes[0].set_xlim(0, 107)
    axes[0].axvline(90, linestyle="--", linewidth=1.3, color="#737f91")
    axes[0].set_title("平均排名百分位", loc="left", pad=18, fontweight="bold")
    axes[0].set_xlabel("虚线：90 分门槛")
    axes[1].axvline(0, color="#737f91", linewidth=1.2)
    axes[1].set_title("平均游戏收益率（%）", loc="left", pad=18, fontweight="bold")
    axes[1].set_xlabel("包含分红、费用与最终结算")
    axes[2].set_xlim(left=0)
    axes[2].set_title("最大回撤（%）", loc="left", pad=18, fontweight="bold")
    axes[2].set_xlabel("● 各局平均    × 最差一局")
    fig.suptitle("Swift 的优势有边界", x=.035, y=.985, ha="left", fontsize=23, fontweight="bold")
    fig.text(.035, .925, f"固定默认策略的压力测试与消融对照 · {report['world_runs']} 局主实验 · 未按结果调参",
             fontsize=12, color="#42526e")
    fig.text(.035, .025, "横线为均值的 95% bootstrap 区间；绿色同时满足排名与前四分位门槛。回撤按观测标记权益计算。\n"
             "各组样本为 20 或 40 个种子；“只保留成交信号 / 关闭信号清仓”为消融实验。所有市场均来自同一模拟器。",
             fontsize=10, color="#56657d")
    fig.subplots_adjust(left=.27, right=.975, top=.845, bottom=.12, wspace=.22)
    args.out.parent.mkdir(parents=True, exist_ok=True)
    fig.savefig(args.out, dpi=160)
    svg = args.out.with_suffix(".svg")
    fig.savefig(svg)
    svg.write_text("\n".join(line.rstrip() for line in svg.read_text(encoding="utf-8").splitlines())+"\n", encoding="utf-8")
    print(args.out)


if __name__ == "__main__":
    main()
