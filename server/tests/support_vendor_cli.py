"""An invented Claude Code and Codex sign-in, run as the adapter by the sign-in tests.

The hub starts an agent's adapter with the vendor's sign-in after it
(`--cli auth login …` for Claude Code, `cli login --device-auth` for Codex);
this script answers the way those tools do, as read from claude-agent-acp
0.94.0 and codex-acp 2.2.2, with invented addresses, codes and an invented
token. No vendor is reached.

Flags before the sign-in's own arguments: `--record PATH` writes the
arguments and the home it was given as JSON lines; `--gate PATH` makes the
Codex sign-in wait until that file exists, as the real one waits for the
user; `--deny` makes it fail the way an account without device codes does.
"""

import json
import os
import sys
import time

# Invented: what a vendor's tool would write as the account's credential.
# The tests prove it never leaves the agent's folder.
TOKEN = "sk-inventado-0123456789abcdefTOKENDEPRUEBA"
CLAUDE_URL = "https://claude.example.test/oauth/authorize?code=true&state=estado-inventado"
CODEX_URL = "https://auth.example.test/codex/device"
PASTED = "codigo-inventado#estado-inventado"


def flag(arguments, name):
    if name in arguments:
        index = arguments.index(name)
        value = arguments[index + 1]
        del arguments[index : index + 2]
        return value
    return None


def main():
    arguments = sys.argv[1:]
    record = flag(arguments, "--record")
    gate = flag(arguments, "--gate")
    deny = "--deny" in arguments
    if deny:
        arguments.remove("--deny")
    home = os.environ.get("CLAUDE_CONFIG_DIR") or os.environ.get("CODEX_HOME")
    if record:
        with open(record, "a", encoding="utf-8") as handle:
            handle.write(
                json.dumps({"arguments": arguments, "home": home, "pid": os.getpid()}) + "\n"
            )
    if arguments[:3] == ["--cli", "auth", "login"]:
        claude_login(home)
    elif arguments[:3] == ["--cli", "auth", "logout"]:
        remove(os.path.join(home, ".credentials.json"))
        print("Successfully logged out from your Anthropic account.")
    elif arguments[:3] == ["cli", "login", "--device-auth"]:
        codex_login(home, gate, deny)
    elif arguments[:2] == ["cli", "logout"]:
        remove(os.path.join(home, "auth.json"))
        print("Successfully logged out", file=sys.stderr)
    else:
        sys.exit(2)


def claude_login(home):
    print("Opening browser to sign in…")
    print(f"If the browser didn't open, visit: {CLAUDE_URL}")
    print("Paste code here if prompted > ", end="", flush=True)
    pasted = sys.stdin.readline().strip()
    if pasted != PASTED:
        print("Login failed: Authentication failed: Invalid authorization code", file=sys.stderr)
        sys.exit(1)
    write(os.path.join(home, ".credentials.json"), {"claudeAiOauth": {"accessToken": TOKEN}})
    print("Login successful.")


def codex_login(home, gate, deny):
    # Codex signs the account it had out before anything else.
    remove(os.path.join(home, "auth.json"))
    print("\nWelcome to Codex [v\x1b[90m0.160.1\x1b[0m]")
    print("1. Open this link in your browser and sign in to your account")
    print(f"   \x1b[94m{CODEX_URL}\x1b[0m\n")
    print("2. Enter this one-time code \x1b[90m(expires in 15 minutes)\x1b[0m")
    print("   \x1b[94mABCD-1234\x1b[0m", flush=True)
    if deny:
        print(
            "Error logging in with device code: device code login is not enabled for this "
            "Codex server. Use the browser login or verify the server URL.",
            file=sys.stderr,
        )
        sys.exit(1)
    while gate is None or not os.path.exists(gate):
        time.sleep(0.05)
    write(os.path.join(home, "auth.json"), {"tokens": {"access_token": TOKEN}})
    print("Successfully logged in", file=sys.stderr)


def write(path, value):
    descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    with os.fdopen(descriptor, "w", encoding="utf-8") as handle:
        json.dump(value, handle)


def remove(path):
    if os.path.exists(path):
        os.remove(path)


if __name__ == "__main__":
    main()
