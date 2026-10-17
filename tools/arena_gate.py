"""Serialize benchmark processes across worktrees to avoid CPU contention.

Usage: python tools/arena_gate.py --cwd WORKTREE -- python tools/bench_bot.py NAME
Use this same absolute script path from each worktree so the lock is shared.
"""

import argparse
import os
from pathlib import Path
import subprocess
import time


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cwd", required=True)
    parser.add_argument("command", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    command = args.command[1:] if args.command[:1] == ["--"] else args.command
    if not command:
        parser.error("a command is required after --")
    lock = Path(__file__).resolve().parents[1] / "runs" / "arena-benchmark.lock"
    lock.parent.mkdir(parents=True, exist_ok=True)
    with lock.open("a+b") as handle:
        handle.seek(0, os.SEEK_END)
        if handle.tell() == 0:
            handle.write(b"0")
            handle.flush()
        print(f"Waiting for benchmark slot: {args.cwd}", flush=True)
        if os.name == "nt":
            import msvcrt

            while True:
                try:
                    handle.seek(0)
                    msvcrt.locking(handle.fileno(), msvcrt.LK_NBLCK, 1)
                    break
                except OSError:
                    time.sleep(0.25)
        else:
            import fcntl

            fcntl.flock(handle, fcntl.LOCK_EX)
        print(f"Benchmark slot acquired: {args.cwd}", flush=True)
        env = os.environ.copy()
        env["RUSTC_WRAPPER"] = ""
        try:
            result = subprocess.run(command, cwd=args.cwd, env=env)
        finally:
            if os.name == "nt":
                handle.seek(0)
                msvcrt.locking(handle.fileno(), msvcrt.LK_UNLCK, 1)
            else:
                fcntl.flock(handle, fcntl.LOCK_UN)
    raise SystemExit(result.returncode)


if __name__ == "__main__":
    main()
