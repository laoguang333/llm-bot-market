"""Fit Keel's dragon classifier (crates/bots/src/keel_growth.rs) on generated worlds.

Research only: the labels come from hidden world truth, the features from public news.

    cargo build --release --bin world-dump
    ./target/release/world-dump scenarios/baseline.toml 3000 100000 runs/worlds-train.jsonl
    python3 tools/fit_keel_growth.py runs/worlds-train.jsonl

Prints one coefficient row per checkpoint (every 250 ticks from 3500), in the order of
`COEF` in keel_growth.rs.
"""

import json
import sys

import numpy as np

TICKS = 10000
DIVERGENCE_PUBLIC = 3333 + 60  # divergence tick + public news stage
SAMPLE = 50
LOOKBACK = 30  # samples, i.e. 1500 ticks
CHECKPOINTS = list(range(3500, 9501, 250))
L2 = 5.0


def features(world):
    """Public dividend features of every stock, sampled every SAMPLE ticks."""
    init = np.array(world["initial"], float)
    level = init.copy()
    changes = []
    for layer, _first, public, _effective, deltas in world["events"]:
        for stock, delta in deltas:
            changes.append((public, layer, stock, delta))
    changes.sort(key=lambda c: c[0])
    proj = init.copy()
    since_div = np.zeros(10)
    since_start = np.zeros(10)
    ups = np.zeros(10)
    count = np.zeros(10)
    history = []
    rows = []
    j = 0
    for k in range(TICKS // SAMPLE):
        t = k * SAMPLE
        while j < len(changes) and changes[j][0] <= t:
            public, layer, s, delta = changes[j]
            before = proj[s]
            proj[s] += delta
            if layer == "Stock":
                step = np.log(max(proj[s], 1) / max(before, 1))
                since_start[s] += step
                if public > DIVERGENCE_PUBLIC:
                    since_div[s] += step
                    count[s] += 1
                    ups[s] += proj[s] > before
            j += 1
        L = np.log(np.maximum(proj, 1) / np.maximum(init, 1))
        history.append(L.copy())
        old = history[k - LOOKBACK] if k >= LOOKBACK else 0.0
        top = np.sort(L)
        other = np.where(L >= top[-1], top[-2], top[-1])
        rows.append(
            np.stack(
                [
                    np.ones(10),
                    L,
                    np.maximum(L - 1, 0),
                    L - other,
                    since_div.copy(),
                    since_start.copy(),
                    L - old,
                    (ups + 1) / (count + 2) - 0.5,
                ],
                axis=-1,
            )
        )
    return rows


def logit(X, y, l2):
    b = np.zeros(X.shape[1])
    for _ in range(40):
        p = 1 / (1 + np.exp(-X @ b))
        g = X.T @ (y - p) - l2 * b
        H = (X * (p * (1 - p))[:, None]).T @ X + l2 * np.eye(X.shape[1])
        b += np.linalg.solve(H, g)
    return b


def main():
    xs = {t: [] for t in CHECKPOINTS}
    ys = {t: [] for t in CHECKPOINTS}
    for line in open(sys.argv[1]):
        world = json.loads(line)
        rows = features(world)
        label = np.array([float(i == world["selected"]) for i in range(10)])
        for t in CHECKPOINTS:
            xs[t].append(rows[t // SAMPLE])
            ys[t].append(label)
    for t in CHECKPOINTS:
        X = np.concatenate(xs[t])
        y = np.concatenate(ys[t])
        b = logit(X, y, L2)
        p = 1 / (1 + np.exp(-X @ b))
        print(
            f"{t}: [{', '.join(f'{c:.4f}' for c in b)}]"
            f"  # mean p dragon {p[y == 1].mean():.2f}, others {p[y == 0].mean():.3f}"
        )


if __name__ == "__main__":
    main()
