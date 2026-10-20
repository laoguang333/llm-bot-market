"""Standalone, exportable comparison of crowding and hunter effects."""
import json
from swift_capital_plot import ROOT, plt, font_manager
from pathlib import Path


def main():
    data = json.loads((ROOT / "docs/swift-crowding-hunt-results.json").read_text(encoding="utf-8"))
    by = {r["case"]: r for r in data["combined"]}
    font = Path("C:/Windows/Fonts/msyh.ttc")
    font_manager.fontManager.addfont(str(font))
    plt.rcParams.update({"font.family": font_manager.FontProperties(fname=str(font)).get_name(),
                         "font.size": 11, "axes.unicode_minus": False,
                         "axes.spines.top": False, "axes.spines.right": False,
                         "figure.facecolor": "#f6f8fb", "axes.facecolor": "#f6f8fb",
                         "savefig.facecolor": "#f6f8fb", "text.color": "#1c2b43"})
    fig, axes = plt.subplots(2, 2, figsize=(14, 10), sharex=True, sharey="row")
    n_values = [1, 3, 5, 10]
    for j, multiple in enumerate([10, 100]):
        for mode, metric, label, color, style in [
            ("alone", "group_return_pct", "Swift：无猎手", "#85909f", "--"),
            ("cash", "group_return_pct", "Swift：清仓对照", "#087f8c", "--"),
            ("relay", "group_return_pct", "Swift：信号猎手", "#b44b40", "-"),
            ("relay", "predator_return_pct", "信号猎手自身", "#6950b3", "-")]:
            available = [(i, by[key][metric]) for i, n in enumerate(n_values)
                         if (key := f"n{n}-{mode}" + (f"-{multiple}" if mode != "alone" else "")) in by]
            x_values = [i for i, _ in available]
            points = [point for _, point in available]
            ax = axes[0, j]
            ax.plot(x_values, [p["mean"] for p in points], marker="o", label=label, color=color, linestyle=style, lw=2)
            if mode == "relay":
                ax.fill_between(x_values, [p["ci95"][0] for p in points], [p["ci95"][1] for p in points], color=color, alpha=.10)
        ax = axes[1, j]
        for mode, color, label in [("relay", "#b44b40", "信号猎手相对收益"), ("cash", "#087f8c", "清仓对照相对收益")]:
            available = [(i, by[key]["advantage_pp"]) for i, n in enumerate(n_values)
                         if (key := f"n{n}-{mode}-{multiple}") in by]
            x_values = [i for i, _ in available]
            points = [point for _, point in available]
            ax.plot(x_values, [p["mean"] for p in points], color=color, marker="o", linestyle="-" if mode == "relay" else "--", lw=2, label=label)
            ax.fill_between(x_values, [p["ci95"][0] for p in points], [p["ci95"][1] for p in points], color=color, alpha=.10)
        axes[0, j].set_title(f"猎手本金：单个 Swift 的 {multiple} 倍", loc="left", fontweight="bold", pad=16)
        axes[0, j].set_ylabel("平均收益率（%）")
        axes[1, j].set_ylabel("猎手收益 − Swift 全组（百分点）")
        axes[1, j].set_xlabel("Swift 数量（每个的资金与策略不变）")
        for ax in axes[:, j]:
            ax.axhline(0, lw=1, color="#303c50")
            ax.set_xticks(range(4), [str(n) for n in n_values])
            ax.grid(axis="y", alpha=.18)
            ax.tick_params(length=0, pad=7)
    h, l = axes[0, 0].get_legend_handles_labels()
    fig.legend(h, l, loc="upper left", bbox_to_anchor=(.06, .90), ncol=4, frameon=False)
    axes[1, 1].legend(loc="lower right", frameon=False, fontsize=10)
    fig.suptitle("更多 Swift，会让单一猎手更轻松吗？", x=.06, y=.98, ha="left", fontsize=22, fontweight="bold")
    stopped_note = "（已中止，空白为未完成）" if data["status"] == "stopped_by_user" else ""
    fig.text(.06, .923, f"{data['runs']} 局{stopped_note} · 20 个全新世界 · 两种注册顺序 · 猎手本金固定", color="#526277", fontsize=12)
    fig.text(.06, .035, "阴影为按世界种子聚类的 95% 区间；所有费用、分红和终值均计入收益。\n"
             "比较对象为 Swift 全组总权益收益率。猎手股票始终每标的 200 股；增加的权益为现金。", fontsize=10, color="#526277")
    fig.subplots_adjust(left=.08, right=.97, top=.80, bottom=.13, wspace=.17, hspace=.21)
    path = ROOT / "docs/swift-crowding-hunt-overview.png"
    fig.savefig(path, dpi=160)
    svg = path.with_suffix(".svg")
    fig.savefig(svg)
    svg.write_text("\n".join(line.rstrip() for line in svg.read_text(encoding="utf-8").splitlines())+"\n", encoding="utf-8")
    print(path)


if __name__ == "__main__":
    main()
