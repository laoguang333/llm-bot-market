"""Read frozen v16 tapes to distinguish liquidity provision and inventory loss.

No bot source is inspected. Run serially through arena_gate.py.
"""

from collections import defaultdict
import csv
import hashlib
import json
import os
from pathlib import Path
import statistics
import time
import tomllib

ROOT = Path(__file__).resolve().parents[1]
TARGETS = ["grid", "coral"]
HORIZONS = [10, 50, 200]


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def mean_present(values):
    values = [v for v in values if v is not None]
    return statistics.mean(values) if values else None


def analyze(folder, config):
    summary = json.loads((folder / "summary.json").read_text(encoding="utf-8"))
    assert summary["dividend_period"] == 1, "Carry integration requires per-tick dividends"
    stocks = [s["name"] for s in config["stocks"]]
    index = {name: i for i, name in enumerate(stocks)}
    ticks = summary["ticks"]
    mid = [[None] * ticks for _ in stocks]
    carry = [[0.0] * (ticks + 1) for _ in stocks]
    with (folder / "books.csv").open(encoding="utf-8", newline="") as f:
        for row in csv.DictReader(f):
            if row["bid"] and row["ask"]:
                mid[index[row["stock"]]][int(row["tick"])] = (
                    int(row["bid"]) + int(row["ask"])) / 2
    with (folder / "bars.csv").open(encoding="utf-8", newline="") as f:
        for row in csv.DictReader(f):
            s, t = index[row["stock"]], int(row["tick"])
            carry[s][t + 1] = carry[s][t] + float(row["dividend_cents_per_payment"])
    by_name = {a["account"]: a["strategy"] for a in summary["accounts"]}
    groups = {g: [a for a in summary["accounts"] if a["strategy"] == g] for g in TARGETS}
    totals = {g: defaultdict(int) for g in TARGETS}
    # Each accumulator: notional, quantity, fills, followed by covered notional,
    # price-only signed P&L, and carry-adjusted signed P&L per horizon.
    fills = defaultdict(lambda: [0.0] * (3 + 3 * len(HORIZONS)))
    phases = defaultdict(lambda: [0.0] * 4)
    market_notional = 0
    with (folder / "trades.csv").open(encoding="utf-8", newline="") as f:
        reader = csv.reader(f)
        next(reader)
        for row in reader:
            t, s, price, qty = int(row[0]), index[row[1]], int(row[2]), int(row[3])
            taker_group, maker_group = by_name[row[5]], by_name[row[6]]
            notional = price * qty
            market_notional += notional
            for g, other, role, buy in [
                (taker_group, maker_group, "taker", row[4] == "buy"),
                (maker_group, taker_group, "maker", row[4] == "sell"),
            ]:
                if g not in totals:
                    continue
                direction = "buy" if buy else "sell"
                sign = 1 if buy else -1
                total = totals[g]
                total[direction] += notional
                total[direction + "_qty"] += qty
                total[role] += notional
                total["fills"] += 1
                if g == other:
                    total["internal_notional"] += notional
                keys = [(g, role + "_" + direction, "all")]
                if g != other:
                    keys.append((g, role + "_" + direction, "external"))
                selected = [fills[key] for key in keys]
                for acc in selected:
                    acc[0] += notional
                    acc[1] += qty
                    acc[2] += 1
                phase = "opening" if t < 1000 else "middle" if t < 9000 else "closing"
                phase_acc = phases[g, phase]
                phase_acc[0] += notional
                phase_acc[1] += notional * (-1 if buy else 1)
                for h_index, h in enumerate(HORIZONS):
                    # Next recorded 10-tick book sample: actual lag h..h+9.
                    end = ((t + h + 9) // 10) * 10
                    if end >= ticks or mid[s][end] is None:
                        continue
                    price_pnl = sign * qty * (mid[s][end] - price)
                    # Hypothetical carry from t+1 through end; per-share
                    # approximation excludes position rounding and financing.
                    carry_pnl = sign * qty * (carry[s][end + 1] - carry[s][t + 1])
                    for acc in selected:
                        offset = 3 + h_index * 3
                        acc[offset] += notional
                        acc[offset + 1] += price_pnl
                        acc[offset + 2] += price_pnl + carry_pnl
                    if h == 50:
                        phase_acc[2] += notional
                        phase_acc[3] += price_pnl + carry_pnl
    result = {}
    for g, accounts in groups.items():
        tot = totals[g]
        initial = sum(a["initial_equity"] for a in accounts)
        initial_cash = sum(a["initial_cash"] for a in accounts)
        terminal = sum(a["terminal_value"] for a in accounts)
        final_cash = sum(a["cash"] for a in accounts)
        stat = lambda k: sum(a["stats"][k] for a in accounts)
        expected_cash = (initial_cash + tot["sell"] - tot["buy"] + stat("dividends")
                         + stat("combo_dividends") - stat("trade_fees")
                         - stat("idle_cash_fees") - stat("interest") + stat("bad_debt"))
        assert expected_cash == final_cash, (g, folder, expected_cash - final_cash)
        assert tot["buy"] + tot["sell"] == stat("volume_notional")
        initial_qty = sum(sum(a["initial_shares"]) for a in accounts)
        final_qty = sum(sum(a["shares"]) for a in accounts)
        assert initial_qty + tot["buy_qty"] - tot["sell_qty"] - final_qty == stat("burned_shares")
        residual = tot["sell"] - tot["buy"] + terminal - (initial - initial_cash)
        net = final_cash + terminal - initial
        assert net == (residual + stat("dividends") + stat("combo_dividends")
                       - stat("trade_fees") - stat("idle_cash_fees")
                       - stat("interest") + stat("bad_debt"))
        metrics = {
            "initial_stock_share": (initial - initial_cash) / initial,
            "net_return": net / initial,
            "trading_inventory_terminal_residual": residual / initial,
            "dividends": stat("dividends") / initial,
            "combo_dividends": stat("combo_dividends") / initial,
            "trade_fees": stat("trade_fees") / initial,
            "idle_cash_fees": stat("idle_cash_fees") / initial,
            "idle_stock_fees_embedded_in_residual": stat("idle_stock_fees") / initial,
            "interest": stat("interest") / initial,
            "bad_debt": stat("bad_debt") / initial,
            "buy_notional": tot["buy"] / initial,
            "sell_notional": tot["sell"] / initial,
            "terminal_value": terminal / initial,
            "final_cash": final_cash / initial,
            "net_bought_shares_per_initial": (tot["buy_qty"] - tot["sell_qty"]) / initial_qty,
            "final_share_count_per_initial": final_qty / initial_qty,
            "burned_share_count_per_initial": stat("burned_shares") / initial_qty,
            "maker_fraction_of_own_notional": tot["maker"] / (tot["buy"] + tot["sell"]),
            "share_of_market_maker_notional": tot["maker"] / market_notional,
            "share_of_market_taker_notional": tot["taker"] / market_notional,
            "share_of_market_maker_notional_external": sum(
                v[0] for (name, role, scope), v in fills.items()
                if name == g and role.startswith("maker") and scope == "external") / market_notional,
            "internal_fraction_of_own_notional": tot["internal_notional"] / (tot["buy"] + tot["sell"]),
        }
        markouts = {}
        for (name, role, scope), acc in fills.items():
            if name != g:
                continue
            entry = {"notional": acc[0], "qty": acc[1], "fills": acc[2], "horizons": {}}
            for h_index, h in enumerate(HORIZONS):
                offset = 3 + h_index * 3
                covered = acc[offset]
                entry["horizons"][str(h)] = {
                    "coverage": covered / acc[0],
                    "price_bps": acc[offset + 1] / covered * 10000 if covered else None,
                    "carry_adjusted_bps": acc[offset + 2] / covered * 10000 if covered else None,
                }
            markouts[role + "_" + scope] = entry
        phase_result = {}
        for (name, phase), acc in phases.items():
            if name == g:
                phase_result[phase] = {
                    "volume_fraction": acc[0] / (tot["buy"] + tot["sell"]),
                    "net_sale_cash_per_initial": acc[1] / initial,
                    "h50_coverage": acc[2] / acc[0],
                    "h50_carry_adjusted_bps": acc[3] / acc[2] * 10000 if acc[2] else None,
                }
        result[g] = {"metrics": metrics, "markouts": markouts, "phases": phase_result}
    return result


def main():
    if os.name == "nt":
        from build_release import windows_build_budget
        budget = windows_build_budget()
    else:
        budget = os.cpu_count()
    survey = json.loads((ROOT / "docs/ecology-v16-results.json").read_text(encoding="utf-8"))
    config = tomllib.loads((ROOT / "scenarios/baseline-v16.toml").read_text(encoding="utf-8"))
    baseline = [r for r in survey["runs"] if r["arm"].startswith("baseline")]
    started = time.perf_counter()
    records = []
    cache = ROOT / "runs/grid-coral-analysis"
    cache.mkdir(parents=True, exist_ok=True)
    for i, run in enumerate(baseline):
        folder = ROOT / "runs/ecology-v16" / f"{run['arm']}-{run['seed']}"
        assert digest(folder / "summary.json") == run["summary_sha256"]
        inputs = {name: digest(folder / name) for name in
                  ["summary.json", "trades.csv", "books.csv", "bars.csv"]}
        cache_path = cache / f"{run['arm']}-{run['seed']}.json"
        saved = json.loads(cache_path.read_text(encoding="utf-8")) if cache_path.exists() else {}
        if saved.get("method_version") == 1 and saved.get("inputs") == inputs:
            result = saved["groups"]
        else:
            result = analyze(folder, config)
            cache_path.write_text(json.dumps(
                {"method_version": 1, "inputs": inputs, "groups": result}, indent=2),
                encoding="utf-8")
        records.append({"seed": run["seed"], "arm": run["arm"], "groups": result})
        print(f"Analyzed {i+1}/{len(baseline)} {run['arm']} seed={run['seed']}", flush=True)
    aggregate = {}
    for g in TARGETS:
        values = [r["groups"][g] for r in records]
        avg = mean_present
        roles = sorted({role for v in values for role in v["markouts"]})
        aggregate[g] = {
            "metrics": {k: avg(v["metrics"][k] for v in values) for k in values[0]["metrics"]},
            "markouts": {role: {h: {
                **{metric: avg(v["markouts"][role]["horizons"][h][metric]
                               for v in values if role in v["markouts"])
                   for metric in ["coverage", "price_bps", "carry_adjusted_bps"]},
                "runs_with_fills": sum(role in v["markouts"] for v in values),
                "runs_with_coverage": sum(
                    role in v["markouts"] and v["markouts"][role]["horizons"][h]["coverage"] > 0
                    for v in values),
                } for h in ["10", "50", "200"]} for role in roles},
            "phases": {phase: {
                k: avg(v["phases"].get(phase, {}).get(
                    k, 0 if k in ["volume_fraction", "net_sale_cash_per_initial"] else None)
                    for v in values)
                for k in ["volume_fraction", "net_sale_cash_per_initial",
                          "h50_coverage", "h50_carry_adjusted_bps"]}
                for phase in ["opening", "middle", "closing"]},
        }
    output = {
        "method": "20 frozen worlds, two repetitions each; equal run/world weight. "
                  "Notional-weighted fill markouts within run. Next 10-tick midpoint snapshot; "
                  "two-sided books only, no imputation. Carry is per-share theoretical income. "
                  "Markout is execution quality, not isolated causal adverse selection or net P&L.",
        "source_survey_sha256": digest(ROOT / "docs/ecology-v16-results.json"),
        "cpu_budget": budget, "elapsed_seconds": time.perf_counter() - started,
        "aggregate": aggregate, "runs": records,
    }
    (ROOT / "docs/grid-coral-v16-results.json").write_text(
        json.dumps(output, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    print(json.dumps(aggregate, ensure_ascii=False, indent=2), flush=True)


if __name__ == "__main__":
    main()
