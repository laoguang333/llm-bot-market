"""Bot processes: one OS process per bot, JSON lines over stdin/stdout.

Isolation here is about fairness and robustness, not security: a bot cannot
see engine memory or other bots, cannot block the market past its time
budget, and a crash only takes itself out. Resource limits (CPU seconds,
address space) are set with setrlimit. Run untrusted code inside a container.
"""

from __future__ import annotations

import json
import os
import resource
import selectors
import subprocess
import sys
import time
from pathlib import Path

CHILD = Path(__file__).with_name("bot_child.py")


class BotProcess:
    def __init__(self, idx: int, bot_file: Path, log_path: Path, limits: dict):
        self.idx = idx
        self.bot_file = bot_file
        self.alive = True
        self.busy_tick: int | None = None  # tick of an outstanding request
        self.busy_since: float = 0.0
        self.buf = b""
        self.dead_reason = ""
        mem = int(limits.get("memory_mb", 1024)) * 1024 * 1024
        cpu = int(limits.get("cpu_seconds", 600))

        def preexec():
            resource.setrlimit(resource.RLIMIT_AS, (mem, mem))
            resource.setrlimit(resource.RLIMIT_CPU, (cpu, cpu + 5))
            os.setsid()

        self.log = open(log_path, "wb")
        cmd = [sys.executable, "-I", "-u", str(CHILD), str(bot_file)]
        self.proc = subprocess.Popen(
            cmd, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=self.log,
            cwd=str(bot_file.parent), preexec_fn=preexec, bufsize=0,
            env={"PATH": os.environ.get("PATH", ""), "PYTHONHASHSEED": "0"},
        )
        os.set_blocking(self.proc.stdout.fileno(), False)

    def send(self, msg: dict, tick: int | None) -> bool:
        if not self.alive:
            return False
        try:
            self.proc.stdin.write((json.dumps(msg, separators=(",", ":")) + "\n").encode())
        except (BrokenPipeError, OSError):
            self.kill("stdin closed")
            return False
        self.busy_tick = tick
        self.busy_since = time.perf_counter()
        return True

    def read_lines(self) -> list[dict]:
        out = []
        try:
            chunk = os.read(self.proc.stdout.fileno(), 1 << 20)
        except BlockingIOError:
            return out
        except OSError:
            chunk = b""
        if not chunk:
            self.kill("exited" if self.proc.poll() is not None else "stdout closed")
            return out
        self.buf += chunk
        while b"\n" in self.buf:
            line, self.buf = self.buf.split(b"\n", 1)
            try:
                out.append(json.loads(line))
            except json.JSONDecodeError:
                out.append({"error": "malformed reply"})
        return out

    def kill(self, reason: str) -> None:
        if self.alive:
            self.alive = False
            self.dead_reason = reason
        try:
            self.proc.kill()
        except OSError:
            pass

    def close(self) -> None:
        try:
            self.proc.stdin.close()
        except OSError:
            pass
        try:
            self.proc.wait(timeout=2)
        except subprocess.TimeoutExpired:
            self.kill("did not exit")
            self.proc.wait()
        self.proc.stdout.close()
        self.log.close()


class BotPool:
    def __init__(self, bots: list[BotProcess]):
        self.bots = bots
        self.sel = selectors.DefaultSelector()
        for b in bots:
            self.sel.register(b.proc.stdout, selectors.EVENT_READ, b)

    def collect(self, tick: int | None, waiting: set[int], timeout_s: float) -> dict[int, dict]:
        """Wait until every bot in `waiting` answered request `tick`, or timeout.

        Replies to older requests (late bots) are consumed and dropped; the bot
        becomes free again. Returns {bot idx: reply}.
        """
        replies: dict[int, dict] = {}
        deadline = time.perf_counter() + timeout_s
        pending = set(waiting)
        while True:
            remaining = deadline - time.perf_counter()
            if not pending or remaining <= 0:
                break
            for key, _ in self.sel.select(remaining):
                b: BotProcess = key.data
                lines = b.read_lines()
                if not b.alive:
                    self.sel.unregister(key.fileobj)
                    pending.discard(b.idx)
                    continue
                for msg in lines:
                    if msg.get("tick") == b.busy_tick:
                        req = b.busy_tick
                        b.busy_tick = None
                        if req == tick and b.idx in pending:
                            replies[b.idx] = msg
                            pending.discard(b.idx)
        return replies

    def close(self) -> None:
        self.sel.close()
        for b in self.bots:
            b.close()

    def drain_late(self) -> None:
        """Pick up replies from late bots without waiting."""
        for key, _ in self.sel.select(0):
            b: BotProcess = key.data
            for msg in b.read_lines():
                if msg.get("tick") == b.busy_tick:
                    b.busy_tick = None
            if not b.alive:
                self.sel.unregister(key.fileobj)
