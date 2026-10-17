"""PNG report from a finished run directory (needs matplotlib)."""

from __future__ import annotations

import csv
import json
from pathlib import Path

import matplotlib

matplotlib.use("Agg")
import matplotlib.pyplot as plt  # noqa: E402

PRICE = "#2a78d6"
REFERENCE = "#8a8984"
UP, DOWN = "#1baf7a", "#e34948"
INK, MUTED, GRID = "#0b0b0b", "#52514e", "#e6e5e1"


def _style(ax):
    ax.grid(axis="y", color=GRID, linewidth=0.8)
    for side in ("top", "right"):
        ax.spines[side].set_visible(False)
    for side in ("left", "bottom"):
        ax.spines[side].set_color(GRID)
    ax.tick_params(colors=MUTED, labelsize=8)


def write_report(run_dir: Path) -> Path:
    summary = json.loads((run_dir / "summary.json").read_text())
    bars: dict[str, dict[str, list]] = {}
    with open(run_dir / "bars.csv") as f:
        for r in csv.DictReader(f):
            b = bars.setdefault(r["stock"], {"tick": [], "last": [], "volume": [], "dividend": []})
            b["tick"].append(int(r["tick"]))
            b["last"].append(float(r["last"]) / 100 if r["last"] else float("nan"))
            b["volume"].append(int(r["volume"]))
            b["dividend"].append(int(r["dividend"]))
    news = list(csv.DictReader(open(run_dir / "news.csv")))
    rate = summary["discount_rate"]
    stocks = list(bars)
    fig = plt.figure(figsize=(12, 3.2 * len(stocks) + 3.2), dpi=120)
    gs = fig.add_gridspec(2 * len(stocks) + 1, 1, height_ratios=[3, 1] * len(stocks) + [2.6], hspace=0.55)
    marks = summary["final_marks"]
    for i, s in enumerate(stocks):
        b = bars[s]
        ax = fig.add_subplot(gs[2 * i])
        _style(ax)
        ax.plot(b["tick"], b["last"], color=PRICE, linewidth=1.4, label="last trade price")
        ax.step(b["tick"], [d / rate / 100 for d in b["dividend"]], where="post", color=REFERENCE,
                linewidth=1.2, linestyle="--", label=f"dividend / {rate:g} (reference)")
        for ev in news:
            if ev["stock"] != s:
                continue
            c = UP if int(ev["dividend_change"]) > 0 else DOWN
            ax.axvspan(int(ev["first_reveal"]), int(ev["public_tick"]), color=c, alpha=0.10, linewidth=0)
            ax.axvline(int(ev["effective"]), color=c, linewidth=0.8, alpha=0.6)
        ax.set_title(f"{s}  ·  final mark {marks[s] / 100:.2f}", loc="left", fontsize=10, color=INK)
        ax.set_ylabel("price", fontsize=8, color=MUTED)
        ax.legend(loc="upper left", fontsize=7, frameon=False)
        ax.tick_params(labelbottom=False)
        axv = fig.add_subplot(gs[2 * i + 1], sharex=ax)
        _style(axv)
        axv.bar(b["tick"], b["volume"], width=1.0, color=PRICE, linewidth=0)
        axv.set_ylabel("volume", fontsize=8, color=MUTED)
    ax = fig.add_subplot(gs[-1])
    _style(ax)
    ax.grid(axis="y", visible=False)
    ax.grid(axis="x", color=GRID, linewidth=0.8)
    strat = sorted(summary["by_strategy"].items(), key=lambda kv: kv[1]["pnl_vs_start"] / kv[1]["accounts"])
    names = [f"{k} (n={v['accounts']})" for k, v in strat]
    vals = [v["pnl_vs_start"] / v["accounts"] / 100 for _, v in strat]
    ax.barh(names, vals, color=PRICE, height=0.6)
    for y, v in enumerate(vals):
        ax.text(v, y, f" {v:,.0f}", va="center", fontsize=8, color=INK)
    ax.axvline(0, color=MUTED, linewidth=0.8)
    ax.set_title("average P&L per account vs. initial holdings at final marks", loc="left", fontsize=10, color=INK)
    fig.text(0.01, 0.005, "shaded: news known to a growing subset of bots until public; vertical line: dividend change "
             "takes effect (green up, red down)", fontsize=7, color=MUTED)
    out = run_dir / "report.png"
    fig.savefig(out, bbox_inches="tight")
    plt.close(fig)
    return out
