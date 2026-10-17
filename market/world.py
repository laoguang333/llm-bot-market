"""Hidden world: dividend path and staged news disclosure.

Dividends only change through news events. Every event is generated up front
from the seed (the hidden truth) and reaches accounts in stages: a random
subset learns it first, the subset widens, then it becomes public, and only
later does the dividend actually change. Bots never see this module's state;
the engine hands them just the events they have been told about.
"""

from __future__ import annotations

import math
import random
from dataclasses import dataclass, field


@dataclass
class NewsEvent:
    event_id: int
    stock: str
    delta: int  # change in per-share dividend, cents
    first_reveal: int
    effective: int
    # tick -> cumulative fraction of accounts informed at that tick
    stages: list[tuple[int, float]]
    order: list[int] = field(default_factory=list)  # account reveal order

    def public_view(self, public: bool) -> dict:
        return {
            "id": self.event_id,
            "stock": self.stock,
            "dividend_change": self.delta,
            "effective_tick": self.effective,
            "first_reveal_tick": self.first_reveal,
            "public": public,
        }


class World:
    def __init__(self, cfg: dict, stocks: list[dict], n_accounts: int, ticks: int, seed: int):
        rng = random.Random(seed ^ 0x5EED)
        news = cfg.get("news", {})
        self.dividend_period = int(cfg.get("dividend_period", 50))
        rate = float(news.get("events_per_1000_ticks", 4.0))
        sd = float(news.get("change_sd", 0.15))
        lead = int(news.get("lead_ticks", 120))
        stage_offsets = [(int(o), float(f)) for o, f in news.get("stages", [[0, 0.05], [40, 0.25], [80, 1.0]])]
        if not stage_offsets or stage_offsets[-1][1] < 1.0:
            raise ValueError("news.stages must end with fraction 1.0 (public)")
        if stage_offsets[-1][0] > lead:
            raise ValueError("news.stages must turn public no later than lead_ticks")

        self.dividend = {s["name"]: int(s["dividend"]) for s in stocks}
        self.events: list[NewsEvent] = []
        eid = 0
        for s in stocks:
            name = s["name"]
            d = int(s["dividend"])
            t = 0.0
            while rate > 0:
                t += rng.expovariate(rate / 1000.0)
                reveal = int(t)
                if reveal >= ticks:
                    break
                x = rng.gauss(0.0, sd)
                delta = max(round(d * x), 1 - d)
                if delta == 0:
                    continue
                d += delta
                order = list(range(n_accounts))
                rng.shuffle(order)
                self.events.append(NewsEvent(
                    event_id=eid, stock=name, delta=delta, first_reveal=reveal,
                    effective=reveal + lead,
                    stages=[(reveal + o, f) for o, f in stage_offsets],
                    order=order,
                ))
                eid += 1
        self.n_accounts = n_accounts
        self._by_stage_tick: dict[int, list[tuple[NewsEvent, int, int]]] = {}
        for ev in self.events:
            prev = 0
            for tick, frac in ev.stages:
                upto = n_accounts if frac >= 1.0 else min(n_accounts, math.ceil(frac * n_accounts))
                if upto > prev:
                    self._by_stage_tick.setdefault(tick, []).append((ev, prev, upto))
                    prev = upto
        self._by_effective: dict[int, list[NewsEvent]] = {}
        for ev in self.events:
            self._by_effective.setdefault(ev.effective, []).append(ev)

    def releases(self, tick: int):
        """Yield (event, newly informed accounts, is_public) for this tick."""
        for ev, lo, hi in self._by_stage_tick.get(tick, []):
            yield ev, ev.order[lo:hi], hi >= self.n_accounts

    def apply_effects(self, tick: int) -> list[NewsEvent]:
        evs = self._by_effective.get(tick, [])
        for ev in evs:
            self.dividend[ev.stock] += ev.delta
        return evs
