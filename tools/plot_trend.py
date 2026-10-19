"""Create a comparison from saved runs; requires ReportLab and pypdfium2, entirely local."""
import csv
from pathlib import Path

from reportlab.graphics import renderSVG, renderPDF
from reportlab.graphics.charts.lineplots import LinePlot
from reportlab.graphics.shapes import Drawing, Line, String
from reportlab.lib import colors

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "docs/assets/v7-trend-seed27003.svg"


def series(arm, stock, field):
    path = ROOT / "runs/v7-trend-study" / arm / "seed-27003/bars.csv"
    points = []
    previous = None
    with path.open() as handle:
        for row in csv.DictReader(handle):
            if row["stock"] != stock:
                continue
            raw = row[field]
            value = float(raw) if raw else previous
            if value is None:
                continue
            previous = value
            if field == "close":
                value /= 100
            points.append((int(row["tick"]), value))
    return points


def main():
    d = Drawing(1120, 730)
    d.add(String(55, 697, "Same world, different growth process - seed 27003", fontSize=22, fillColor=colors.HexColor("#182239")))
    palette = [colors.HexColor(x) for x in ["#a0a5b0", "#da7854", "#237bba"]]
    arms = ["off", "pulses", "trend"]
    for i, label in enumerate(["Ordinary news only", "Old eight-node pulse", "New irregular trend"]):
        x = 55 + i * 330
        d.add(Line(x, 669, x + 26, 669, strokeColor=palette[i], strokeWidth=2))
        d.add(String(x + 34, 664, label, fontSize=12))
    for index, (stock, field, label) in enumerate([
        ("E2", "dividend_cents_per_payment", "E2 dividends (cents/share/tick)"),
        ("E1", "dividend_cents_per_payment", "E1 dividends (cents/share/tick)"),
        ("E2", "close", "E2 executed price (yuan)"),
        ("E1", "close", "E1 executed price (yuan)"),
    ]):
        x, y = 65 + (index % 2) * 550, 395 - (index // 2) * 310
        plot = LinePlot()
        plot.x = x; plot.y = y; plot.width = 450; plot.height = 210
        plot.data = [series(arm, stock, field) for arm in arms]
        plot.joinedLines = 1
        plot.xValueAxis.valueMin = 0; plot.xValueAxis.valueMax = 3000
        plot.xValueAxis.valueSteps = list(range(0, 3001, 500))
        plot.xValueAxis.labels.fontSize = 9
        plot.yValueAxis.valueMin = 0
        plot.yValueAxis.valueMax = max(v for data in plot.data for _, v in data) * 1.12
        plot.yValueAxis.labels.fontSize = 9
        plot.yValueAxis.visibleGrid = 1
        plot.yValueAxis.gridStrokeColor = colors.HexColor("#e7eaf0")
        for i, color in enumerate(palette):
            plot.lines[i].strokeColor = color
            plot.lines[i].strokeWidth = 1.4
        d.add(plot)
        d.add(String(x, y + 230, label, fontSize=15, fillColor=colors.HexColor("#182239")))
        d.add(String(x + 205, y - 35, "tick", fontSize=10))
    d.add(String(55, 20, "Each arm is a separate 4-thread match. Ordinary news is matched; execution paths can differ. E2 is the selected stock.", fontSize=10))
    OUT.parent.mkdir(exist_ok=True)
    renderSVG.drawToFile(d, str(OUT))
    # Rasterize in memory for viewers without SVG support; no browser/network needed.
    import pypdfium2
    with pypdfium2.PdfDocument(renderPDF.drawToString(d)) as pdf:
        pdf[0].render(scale=1.5).to_pil().save(OUT.with_suffix(".png"))
    print(OUT)


if __name__ == "__main__":
    main()
