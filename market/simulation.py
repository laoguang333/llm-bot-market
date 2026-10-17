"""Run loop: spawn bots, step the market, write results."""

from __future__ import annotations

import csv
import json
import random
import time
import tomllib
from pathlib import Path

from .exchange import Account, Exchange
from .sandbox import BotPool, BotProcess
from .world import World

START_TICK = -1
END_TICK = -2


def load_config(path: Path) -> dict:
    with open(path, "rb") as f:
        cfg = tomllib.load(f)
    cfg["_dir"] = str(path.parent.resolve())
    return cfg


def build_accounts(cfg: dict, stocks: list[str]) -> tuple[list[Account], list[dict]]:
    accounts, specs = [], []
    base = Path(cfg["_dir"])
    for group in cfg["bots"]:
        bot_file = (base / group["file"]).resolve()
        if not bot_file.is_file():
            raise FileNotFoundError(bot_file)
        owner = group.get("owner", bot_file.stem)
        for i in range(int(group.get("count", 1))):
            idx = len(accounts)
            shares = {s: int(group.get("shares", {}).get(s, 0)) for s in stocks}
            name = f"{owner}/{bot_file.stem}#{i}"
            accounts.append(Account(idx, name, owner, int(group.get("cash", 0)), shares))
            specs.append({"file": bot_file, "strategy": bot_file.stem, "params": group.get("params", {})})
    return accounts, specs


class Simulation:
    def __init__(self, cfg: dict, out_dir: Path):
        self.cfg = cfg
        self.out = out_dir
        self.seed = int(cfg.get("seed", 1))
        self.ticks = int(cfg.get("ticks", 2000))
        mkt = cfg.get("market", {})
        self.depth = int(mkt.get("book_depth", 10))
        self.tape_limit = int(mkt.get("tape_limit", 200))
        sb = cfg.get("sandbox", {})
        self.step_timeout = float(sb.get("step_timeout_ms", 250)) / 1000
        self.start_timeout = float(sb.get("start_timeout_ms", 10000)) / 1000
        self.max_late_ticks = int(sb.get("max_late_ticks", 100))
        self.limits = sb
        scoring = cfg.get("scoring", {})
        self.final_mark = scoring.get("final_mark", "vwap")
        self.vwap_ticks = int(scoring.get("vwap_ticks", 200))
        self.discount_rate = float(scoring.get("discount_rate", 0.01))

        self.stock_cfg = cfg["stocks"]
        self.stocks = [s["name"] for s in self.stock_cfg]
        accounts, self.specs = build_accounts(cfg, self.stocks)
        self.ex = Exchange(cfg, self.stocks, accounts)
        self.world = World(cfg.get("world", {}), self.stock_cfg, len(accounts), self.ticks, self.seed)
        self.rng = random.Random(self.seed)
        self.tape_seen = [{s: 0 for s in self.stocks} for _ in accounts]
        self.last_price = {s: int(c.get("initial_price", 0)) or None for s, c in zip(self.stocks, self.stock_cfg)}
        self.bars: list[dict] = []
        self.late_ticks = [0] * len(accounts)
        self.disabled = [False] * len(accounts)

    # ---- bots ----------------------------------------------------------
    def start_bots(self) -> None:
        logs = self.out / "logs"
        logs.mkdir(parents=True, exist_ok=True)
        procs = []
        for acct, spec in zip(self.ex.accounts, self.specs):
            safe = acct.name.replace("/", "_").replace("#", "_")
            procs.append(BotProcess(acct.idx, spec["file"], logs / f"{safe}.log", self.limits))
        self.pool = BotPool(procs)
        rules = self.public_rules()
        for acct, spec, p in zip(self.ex.accounts, self.specs, procs):
            p.send({"type": "start", "tick": START_TICK, "account": acct.name, "owner": acct.owner,
                    "seed": self.rng.randrange(2**31), "params": spec["params"], "rules": rules,
                    "cash": acct.cash, "shares": dict(acct.shares)}, START_TICK)
        replies = self.pool.collect(START_TICK, {p.idx for p in procs}, self.start_timeout)
        for p in procs:
            r = replies.get(p.idx)
            if r is None or r.get("error") or not r.get("ok"):
                reason = (r or {}).get("error", "no start reply")
                self.disable(p.idx, f"start failed: {reason[-300:]}")

    def disable(self, idx: int, reason: str) -> None:
        if self.disabled[idx]:
            return
        self.disabled[idx] = True
        p = self.pool.bots[idx]
        p.kill(reason)
        p.dead_reason = reason
        acct = self.ex.accounts[idx]
        for oid in list(acct.open_orders):
            self.ex.cancel_order(acct, oid)

    def public_rules(self) -> dict:
        ex = self.ex
        return {
            "stocks": [{"name": s["name"], "dividend": s["dividend"], "initial_price": s.get("initial_price")}
                       for s in self.stock_cfg],
            "ticks_total": self.ticks,
            "price_unit": "cents (integers)",
            "dividend_period": self.world.dividend_period,
            "trade_fee_bps": ex.trade_fee_bps,
            "carry_period": ex.carry_period,
            "carry_rate": ex.carry_rate_ppm / 1_000_000,
            "credit_ratio": ex.credit_ratio_ppm / 1_000_000,
            "credit_side": ex.credit_side,
            "stp_scope": ex.stp_scope,
            "max_actions_per_tick": ex.max_actions,
            "max_open_orders": ex.max_open_orders,
            "step_timeout_ms": round(self.step_timeout * 1000),
            "book_depth": self.depth,
            "final_mark": self.final_mark,
            "vwap_ticks": self.vwap_ticks,
            "discount_rate": self.discount_rate,
        }

    def observation(self, acct: Account) -> dict:
        ex = self.ex
        t = ex.tick
        fee, credit, due = ex.carry_estimate(acct)
        period_end = (t // ex.carry_period + 1) * ex.carry_period - 1
        next_div = (t // self.world.dividend_period + 1) * self.world.dividend_period
        market, trades = {}, {}
        seen = self.tape_seen[acct.idx]
        for s in self.stocks:
            book = ex.books[s]
            market[s] = {"bids": book.depth("buy", self.depth), "asks": book.depth("sell", self.depth),
                         "last": self.last_price[s], "dividend": self.world.dividend[s],
                         "next_dividend_tick": next_div}
            tape = ex.tape[s]
            start = max(seen[s], len(tape) - self.tape_limit)
            trades[s] = [list(x) for x in tape[start:]]
            seen[s] = len(tape)
        obs = {
            "type": "tick", "tick": t, "ticks_total": self.ticks,
            "account": {
                "cash": acct.cash, "cash_available": acct.cash_available(),
                "positions": {s: {"shares": acct.shares.get(s, 0), "available": acct.shares_available(s)}
                              for s in self.stocks},
                "carry": {"period_end_tick": period_end, "fee_estimate": fee,
                          "credit_estimate": credit, "due_estimate": due, "debt": acct.fee_debt},
            },
            "open_orders": [{"order_id": o.order_id, "stock": o.stock, "side": o.side, "price": o.price,
                             "qty": o.qty, "placed_tick": o.placed_tick} for o in acct.open_orders.values()],
            "market": market, "trades": trades,
        }
        obs.update(acct.inbox)
        acct.inbox = {k: [] for k in acct.inbox}
        return obs

    # ---- main loop -----------------------------------------------------
    def step(self, t: int) -> None:
        ex = self.ex
        ex.tick = t
        self.world.apply_effects(t)
        for ev, informed, public in self.world.releases(t):
            view = ev.public_view(public)
            for idx in informed:
                ex.accounts[idx].inbox["news"].append(view)
        if t > 0 and t % self.world.dividend_period == 0:
            ex.pay_dividends(self.world.dividend)

        tape_start = {s: len(ex.tape[s]) for s in self.stocks}
        self.pool.drain_late()
        waiting = set()
        for acct, p in zip(ex.accounts, self.pool.bots):
            if self.disabled[acct.idx]:
                continue
            if not p.alive:
                self.disable(acct.idx, p.dead_reason or "process died")
                continue
            if p.busy_tick is not None:
                self.late_ticks[acct.idx] += 1
                acct.stats["timeouts"] += 1
                if self.late_ticks[acct.idx] > self.max_late_ticks:
                    self.disable(acct.idx, f"unresponsive for {self.max_late_ticks} ticks")
                continue
            if p.send(self.observation(acct), t):
                waiting.add(acct.idx)
        replies = self.pool.collect(t, waiting, self.step_timeout)
        for idx in waiting - replies.keys():
            ex.accounts[idx].stats["timeouts"] += 1
        order = sorted(replies)
        self.rng.shuffle(order)
        for idx in order:
            self.late_ticks[idx] = 0
            r = replies[idx]
            acct = ex.accounts[idx]
            if r.get("error"):
                acct.stats["errors"] += 1
            ex.apply_actions(acct, r.get("actions", []))
        ex.end_of_tick()
        self.record_bar(t, tape_start)

    def record_bar(self, t: int, tape_start: dict) -> None:
        bar = {"tick": t}
        for s in self.stocks:
            trades = self.ex.tape[s][tape_start[s]:]
            book = self.ex.books[s]
            if trades:
                prices = [p for _, p, _, _ in trades]
                vol = sum(q for _, _, q, _ in trades)
                notional = sum(p * q for _, p, q, _ in trades)
                self.last_price[s] = prices[-1]
                o, h, l, c = prices[0], max(prices), min(prices), prices[-1]
            else:
                vol = notional = 0
                o = h = l = c = None
            bar[s] = {"open": o, "high": h, "low": l, "close": c, "volume": vol, "notional": notional,
                      "bid": book.best("buy"), "ask": book.best("sell"), "last": self.last_price[s],
                      "dividend": self.world.dividend[s]}
        self.bars.append(bar)

    def marks(self) -> dict[str, float]:
        out = {}
        for s in self.stocks:
            fundamental = self.world.dividend[s] / self.discount_rate
            if self.final_mark == "fundamental":
                out[s] = fundamental
                continue
            window = self.bars[-self.vwap_ticks:]
            vol = sum(b[s]["volume"] for b in window)
            if vol:
                out[s] = sum(b[s]["notional"] for b in window) / vol
            else:
                out[s] = self.last_price[s] if self.last_price[s] else fundamental
        return out

    def run(self) -> dict:
        self.out.mkdir(parents=True, exist_ok=False)
        t0 = time.perf_counter()
        initial = [(a.cash, dict(a.shares)) for a in self.ex.accounts]
        self.start_bots()
        try:
            for t in range(self.ticks):
                self.step(t)
                if t % 100 == 0 or t == self.ticks - 1:
                    self.ex.check_invariants()
            wall = time.perf_counter() - t0
            results = self.results(initial, wall)
            for p in self.pool.bots:
                if p.alive and not self.disabled[p.idx]:
                    p.busy_tick = None
                    p.send({"type": "end", "tick": END_TICK, "results": results["accounts"][p.idx]}, END_TICK)
            self.pool.collect(END_TICK, {p.idx for p in self.pool.bots if p.alive}, 2.0)
        finally:
            self.pool.close()
        self.write(results)
        return results

    def results(self, initial, wall: float) -> dict:
        marks = self.marks()
        rows = []
        for acct, spec, (cash0, shares0) in zip(self.ex.accounts, self.specs, initial):
            equity = acct.cash + sum(acct.shares.get(s, 0) * marks[s] for s in self.stocks)
            hold = cash0 + sum(shares0.get(s, 0) * marks[s] for s in self.stocks)
            rows.append({
                "account": acct.name, "owner": acct.owner, "strategy": spec["strategy"],
                "cash": acct.cash, "shares": dict(acct.shares), "equity": round(equity),
                "start_value_at_final_marks": round(hold), "pnl_vs_start": round(equity - hold),
                "disabled": self.pool.bots[acct.idx].dead_reason if self.disabled[acct.idx] else "",
                **acct.stats,
            })
        by_strategy: dict[str, dict] = {}
        for r in rows:
            g = by_strategy.setdefault(r["strategy"], {"accounts": 0, "pnl_vs_start": 0, "volume_notional": 0,
                                                       "carry_fees": 0, "dividends": 0, "trade_fees": 0})
            g["accounts"] += 1
            for k in ("pnl_vs_start", "volume_notional", "carry_fees", "dividends", "trade_fees"):
                g[k] += r[k]
        dividend_paid = sum(r["dividends"] for r in rows)
        fees = sum(r["carry_fees"] + r["trade_fees"] for r in rows)
        return {
            "seed": self.seed, "ticks": self.ticks, "accounts_n": len(rows), "wall_seconds": round(wall, 2),
            "final_marks": {s: round(m, 2) for s, m in marks.items()}, "discount_rate": self.discount_rate,
            "final_dividend": dict(self.world.dividend),
            "news_events": len(self.world.events),
            "treasury": self.ex.treasury, "dividends_paid": dividend_paid, "fees_collected": fees,
            "by_strategy": by_strategy, "accounts": rows,
        }

    def write(self, results: dict) -> None:
        (self.out / "summary.json").write_text(json.dumps(results, indent=2, ensure_ascii=False))
        with open(self.out / "bars.csv", "w", newline="") as f:
            w = csv.writer(f)
            w.writerow(["tick", "stock", "open", "high", "low", "close", "volume", "notional", "bid", "ask",
                        "last", "dividend"])
            for b in self.bars:
                for s in self.stocks:
                    x = b[s]
                    w.writerow([b["tick"], s, x["open"], x["high"], x["low"], x["close"], x["volume"],
                                x["notional"], x["bid"], x["ask"], x["last"], x["dividend"]])
        with open(self.out / "trades.csv", "w", newline="") as f:
            w = csv.writer(f)
            w.writerow(["tick", "stock", "price", "qty", "aggressor"])
            for s in self.stocks:
                for row in self.ex.tape[s]:
                    w.writerow([row[0], s, *row[1:]])
        with open(self.out / "news.csv", "w", newline="") as f:
            w = csv.writer(f)
            w.writerow(["id", "stock", "dividend_change", "first_reveal", "public_tick", "effective"])
            for ev in self.world.events:
                w.writerow([ev.event_id, ev.stock, ev.delta, ev.first_reveal, ev.stages[-1][0], ev.effective])
