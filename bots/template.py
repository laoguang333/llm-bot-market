"""Minimal bot skeleton. Copy this file to start a new strategy.

The engine runs this file in its own process and calls:
  Bot(start)        once, with the rules, your account name, seed and params
  bot.on_tick(obs)  every tick you are not late; return a list of actions
  bot.on_end(msg)   once at the end, with your results (optional)

See docs/BOT_API.md for every field. Only the standard library is available.
"""

import random


class Bot:
    def __init__(self, start):
        self.rules = start["rules"]
        self.params = start["params"]
        self.rng = random.Random(start["seed"])

    def on_tick(self, obs):
        actions = []
        for stock, m in obs["market"].items():
            if m["bids"] and m["asks"]:
                pass  # look at the book, decide, append actions
        return actions
