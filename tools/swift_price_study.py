"""Price-only exploratory statistics. Never reads dividend/news/account columns.

Run with a Python containing NumPy. Historical replay is exploratory only: live
challenge results include the entrant's market impact and asynchronous scheduling.
"""
import csv
from pathlib import Path

import numpy as np


def main():
    rows, labels = [], []
    names = ["return10", "return30", "return100", "return300", "spread",
             "depth", "last_mid", "flow10", "flow30", "flow100", "volume_ratio"]
    for seed in range(1, 21):
        root = Path(f"runs/baseline-v5-20261018/seed-{seed}")
        data = {}
        for r in csv.DictReader((root / "bars.csv").open()):
            s, t = r["stock"], int(r["tick"])
            if s not in data:
                data[s] = np.zeros((3001, 6))
            if t <= 3000:
                data[s][t, :2] = [float(r["close"] or (data[s][t-1, 0] if t else 0)), float(r["volume"])]
        for r in csv.DictReader((root / "trades.csv").open()):
            t = int(r["tick"])
            if t <= 3000:
                data[r["stock"]][t, 2] += int(r["qty"]) * (1 if r["aggressor"].lower() == "buy" else -1)
        books = {}
        for r in csv.DictReader((root / "books.csv").open()):
            if not r["bid"] or not r["ask"]:
                continue
            s, t = r["stock"], int(r["tick"])
            bid, ask = int(r["bid"]), int(r["ask"])
            books[s, t] = (bid, ask, int(r["bid_depth_2pct"]), int(r["ask_depth_2pct"]))
        for s, a in data.items():
            c = np.cumsum(a[:, 1:3], axis=0)
            for t in range(310, 2801, 10):
                if (s, t) not in books or any((s, t+h) not in books for h in [20, 50, 100]):
                    continue
                bid, ask, bd, ad = books[s, t]
                mid = (bid+ask)/2
                if mid <= 0 or any(a[t-h, 0] <= 0 for h in [10, 30, 100, 300]):
                    continue
                vs = [c[t]-c[t-h] for h in [10, 30, 100]]
                x = [mid / a[t-h, 0]-1 for h in [10, 30, 100, 300]]
                x += [(ask-bid)/mid, (bd-ad)/max(1, bd+ad), a[t, 0]/mid-1]
                x += [f/max(1, v) for v, f in vs]
                x += [vs[0][0]*10/max(1, vs[2][0])]
                rows.append(x)
                labels.append([((books[s, t+h][0]+books[s, t+h][1])/2)/mid-1 for h in [20, 50, 100]])
    x, y = np.array(rows), np.clip(np.array(labels), -.3, .3)
    print("samples", len(x), "future return20/50/100", y.mean(axis=0))
    for j, name in enumerate(names):
        q = np.quantile(x[:, j], [.1, .3, .5, .7, .9])
        bins = np.searchsorted(q, x[:, j])
        print(name, "cuts", np.round(q, 4).tolist())
        print([(int(np.sum(bins == k)), np.round(y[bins == k].mean(axis=0)*100, 2).tolist())
               for k in range(6) if np.any(bins == k)])


if __name__ == "__main__":
    main()
