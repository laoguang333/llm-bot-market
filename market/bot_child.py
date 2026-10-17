"""Bot-side harness. Runs inside the bot's own process.

Loads the bot file, then loops: read one JSON request from stdin, call the
bot, write one JSON reply. The real stdout is reserved for the protocol, so
anything the bot prints goes to its log (stderr).
"""

import importlib.util
import json
import sys
import traceback


def main() -> None:
    proto_out = sys.stdout
    sys.stdout = sys.stderr
    path = sys.argv[1]
    spec = importlib.util.spec_from_file_location("bot_module", path)
    module = importlib.util.module_from_spec(spec)
    bot = None
    load_error = None
    try:
        spec.loader.exec_module(module)
    except Exception:
        load_error = traceback.format_exc()

    def reply(obj):
        proto_out.write(json.dumps(obj, separators=(",", ":")) + "\n")
        proto_out.flush()

    for line in sys.stdin:
        msg = json.loads(line)
        kind = msg.get("type")
        tick = msg.get("tick")
        try:
            if kind == "start":
                if load_error:
                    raise RuntimeError(load_error)
                bot = module.Bot(msg)
                reply({"tick": tick, "ok": True})
            elif kind == "tick":
                actions = bot.on_tick(msg) or []
                reply({"tick": tick, "actions": list(actions)})
            elif kind == "end":
                if hasattr(bot, "on_end"):
                    bot.on_end(msg)
                reply({"tick": tick, "ok": True})
                break
        except Exception:
            err = traceback.format_exc()
            sys.stderr.write(err)
            reply({"tick": tick, "actions": [], "error": err[-2000:]})


if __name__ == "__main__":
    main()
