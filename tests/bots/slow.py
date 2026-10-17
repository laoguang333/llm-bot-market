import time


class Bot:
    def __init__(self, start):
        self.n = 0

    def on_tick(self, obs):
        self.n += 1
        if self.n == 2:
            time.sleep(0.5)  # miss a few ticks once
        return []
