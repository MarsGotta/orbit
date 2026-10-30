"""Mac-side CLI dispatch; the Mac boundary is supplied by mac.py."""

from __future__ import annotations

import json

from harness_copy import language_scope
from mac_controls import PARTS, MacFailure
from mac_drafts_panel import COMMANDS as DRAFTS_AND_PANEL


def _bound(arguments: list[str]) -> tuple[list[str], float]:
    if "--max" not in arguments:
        raise MacFailure("--max is missing")
    position = arguments.index("--max")
    return arguments[:position] + arguments[position + 2 :], float(arguments[position + 1])


def main(argv: list[str], mac) -> int:
    selected = "es"
    if "--language" in argv:
        position = argv.index("--language")
        selected = argv[position + 1] if position + 1 < len(argv) else ""
        argv = argv[:position] + argv[position + 2 :]
    try:
        with language_scope(selected):
            return _main(argv, mac)
    except ValueError as cause:
        print(json.dumps({"error": str(cause)}))
        return 1


def _main(argv: list[str], mac) -> int:
    try:
        if not argv:
            raise MacFailure("the subcommand is missing")
        command, rest = argv[0], argv[1:]
        if command == "prepare":
            _, max_s = _bound(rest)
            output = mac.prepare(max_s)
        elif command == "pair" and len(rest) == 3:
            output = mac.pair(*rest)
        elif command == "wait":
            rest, max_s = _bound(rest)
            output = mac.wait(rest[0], rest[1] if len(rest) > 1 else None, max_s)
        elif command == "read" and rest:
            output = mac.read(rest[0], rest[1] if len(rest) > 1 else None)
        elif command == "say" and len(rest) == 1 and rest[0] in PARTS:
            output = mac.say(rest[0])
        elif command == "listening-choice" and not rest:
            output = mac.listening_choice()
        elif command == "voice-note" and not rest:
            output = mac.voice_note()
        elif command == "command" and len(rest) == 1 and rest[0] in ("call_start", "call_stop"):
            output = mac.command(rest[0])
        elif command == "restore":
            _, max_s = _bound(rest)
            output = mac.restore(max_s)
        elif command == "remote-audio" and len(rest) == 1:
            output = mac.remote_audio(rest[0])
        elif command == "native" and 1 <= len(rest) <= 2:
            output = mac.native(rest[0], rest[1] if len(rest) > 1 else None)
        elif command == "logs":
            output = mac.logs()
        elif command == "switch-input" and len(rest) <= 1:
            output = mac.switch_input(rest[0] if rest else None)
        elif command == "activate":
            output = mac.activate()
        elif command in DRAFTS_AND_PANEL:
            output = DRAFTS_AND_PANEL[command](mac, rest)
        elif command == "bridge-call" and len(rest) == 1:
            output = mac.bridge_call(json.loads(rest[0]))
        elif command == "bridge-menus" and not rest:
            output = mac.bridge_menus()
        elif command == "engine-start":
            _, max_s = _bound(rest)
            output = mac.engine_start(max_s)
        elif command == "engine-call" and len(rest) == 2:
            output = mac.engine_call(rest[0], json.loads(rest[1]))
        elif command == "engine-stop":
            _, max_s = _bound(rest)
            output = mac.engine_stop(max_s)
        else:
            raise MacFailure(f"unknown subcommand: {' '.join(argv)[:80]}")
    except (MacFailure, OSError, ValueError, KeyError) as cause:
        print(json.dumps({"error": str(cause) or type(cause).__name__}, ensure_ascii=False))
        return 1
    print(json.dumps(output, ensure_ascii=False))
    return 0
