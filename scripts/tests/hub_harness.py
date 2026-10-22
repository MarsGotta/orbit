"""A sandbox for the production hub scripts: temporary base, fake systemd, fake hub.

Every run of a hub script from a test goes through here. Importing this module also marks the
whole test process with ORBIT_HUB_UNDER_TEST, so a hub script started from the
same run — through here or not — refuses to act unless its base, systemctl,
journalctl, ps, hermes, hub URL and Deimos profile are all redirected (lib.sh
checks it).

Nothing here reaches the machine's services. systemctl, journalctl and hermes
are stand-ins that write what they were asked into a log and answer from a JSON
file; ps prints a synthetic process listing, never this machine's; the hub is an
HTTP server in this process on an ephemeral loopback port; the base, the
repository and the Deimos profile live in a temporary directory.
uv is the real one, offline, reading its own cache: it is what builds a release
in production and the thing most worth exercising.
"""

import http.server
import json
import os
import re
import shutil
import sqlite3
import stat
import subprocess
import sys
import tempfile
import threading
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
HUB_SCRIPTS = ROOT / "scripts" / "hub"
LIB = HUB_SCRIPTS / "lib.sh"

# The process-wide mark. Set at import, before any test runs, so every child of
# this test process inherits it.
os.environ["ORBIT_HUB_UNDER_TEST"] = "1"

UNCONFIGURED_BASE = "/unconfigured-orbit-base"
HUB_UNIT = "orbit-hub.service"
SERVE_UNIT = "hermes-serve-orbit.service"
# The unit Deimos's gateway had until October 2026. Hermes now runs one gateway
# per host; the scripts must not ask systemd for that retired per-profile unit.
RETIRED_GATEWAY_UNIT = "hermes-gateway-deimos.service"
# How the gateway is restarted now, as restarts() names it.
GATEWAY_RESTART = "hermes -p default gateway restart"
GATEWAY_RESTART_ARGV = ["-p", "default", "gateway", "restart"]
GATEWAY_STATUS_ARGV = ["-p", "default", "gateway", "status"]
RESTART_ORDER = (HUB_UNIT, SERVE_UNIT, GATEWAY_RESTART)

# The synthetic process listing (see FAKE_PS), shaped like the one measured on
# the server on 2026-10-15: the host gateway is a `… gateway run` process of
# this user, outside systemd, and the Deimos profile's orbit-mcp is its child,
# launched through the release's shebang, so its interpreter names the release.
USER_MANAGER_PID = 100
GATEWAY_PID = 200
CONTAINER_GATEWAY_PID = 400
SERVE_PID = 500
FIRST_TRANSLATOR_PID = 300
GATEWAY_ARGS = "/home/fake/hermes/python3 -I -c import os, re, sys; from hermes_cli.main import main gateway run"
CONTAINER_GATEWAY_ARGS = (
    "/opt/hermes/.venv/bin/python3 /opt/hermes/.venv/bin/hermes gateway run --replace"
)
SERVE_ARGS = "/home/fake/hermes/venv/bin/python -m hermes_cli.main --profile deimos serve --isolated --port 8643"
DEVELOPMENT_TRANSLATOR_ARGS = "/home/example/projects/orbit/server/.venv/bin/python /home/example/projects/orbit/server/.venv/bin/orbit-mcp"
# Historical: what `hermes -p deimos gateway status` printed on the server on 2026-10-15,
# exit 0, while the host gateway (profile default) was serving Deimos: its
# word for Deimos is untrue, which is why activation does not rely on it.
DEIMOS_STATUS_OUTPUT = (
    "✗ Gateway is not running\n\nTo start:\n  hermes gateway run      # Run in foreground\n\n"
    "Other profiles:\n  ✓ default          — PID 200"
)
# Synthetic default status; acceptance still depends on its new translator.
HOST_STATUS_OUTPUT = "✓ Gateway is running — PID 200"
# Hermes's refusal as recorded on 2026-10-20 (its omitted middle was not
# recorded). It must never replace the translator or report success.
DEIMOS_RESTART_OUTPUT = (
    "The host gateway already serves profile 'deimos' … "
    "Manage the host gateway instead: hermes -p default gateway restart"
)
# What the operator's `hermes -p deimos gateway restart` answered on 2026-10-15; the
# restart had worked all the same.
PENDING_RESTART_OUTPUT = (
    "Profile 'deimos' restart was not confirmed: host operation is still pending."
)


def translator_args(base, version):
    """An orbit-mcp of `version` as ps shows it: the release's python, then current's script."""
    return f"{base}/releases/{version}/.venv/bin/python {base}/current/.venv/bin/orbit-mcp"


# What the unit said before hub releases existed, for the tests that refuse it.
DEVELOPMENT_EXEC_START = (
    "/home/example/.local/bin/uv run --frozen orbit-hub --config"
    " /srv/orbit/stacks/orbit-hub/etc/orbit.toml serve --host 127.0.0.1 --port 8700"
)
DEVELOPMENT_WORKING_DIRECTORY = "/home/example/projects/orbit/server"

FAKE_MARK = "ORBIT_HUB_FAKE_"
linux_only = unittest.skipUnless(
    sys.platform.startswith("linux"),
    "the hub scripts are the Linux server's: they use systemd, flock and mv -T",
)

FAKE_SYSTEMCTL = r"""#!/usr/bin/env python3
# ORBIT_HUB_FAKE_SYSTEMCTL — written by scripts/tests/hub_harness.py.
# It never reaches systemd: it records what it was asked and answers from units.json.
import json, os, sys

STATE = @STATE@


def record(entry):
    with open(os.path.join(STATE, "events.jsonl"), "a", encoding="utf-8") as handle:
        handle.write(json.dumps(entry) + "\n")


arguments = sys.argv[1:]
record({
    "tool": "systemctl",
    "argv": arguments,
    "env": {key: os.environ.get(key) for key in ("XDG_RUNTIME_DIR", "DBUS_SESSION_BUS_ADDRESS")},
})
if not arguments or arguments[0] != "--user":
    print("fake systemctl: it only serves --user", file=sys.stderr)
    sys.exit(64)
arguments = arguments[1:]
verb = arguments[0] if arguments else ""
positional, properties, value_only = [], [], False
index = 1
while index < len(arguments):
    argument = arguments[index]
    if argument in ("-p", "--property") and index + 1 < len(arguments):
        properties.append(arguments[index + 1])
        index += 2
        continue
    if argument.startswith("--property="):
        properties.append(argument.split("=", 1)[1])
    elif argument == "--value":
        value_only = True
    elif not argument.startswith("-"):
        positional.append(argument)
    index += 1

path = os.path.join(STATE, "units.json")
with open(path, encoding="utf-8") as handle:
    units = json.load(handle)
unit = positional[0] if positional else ""
# A unit not in units.json does not exist, as hermes-gateway-deimos.service no
# longer did on 2026-10-15: restarting it fails the way systemd failed then.
known = unit in units
state = units.setdefault(unit, {})
active = state.get("ActiveState", "active" if known else "inactive")

if verb == "daemon-reload":
    sys.exit(0)
if verb == "show":
    for name in properties:
        value = state.get(name, "")
        print(value if value_only else f"{name}={value}")
    sys.exit(0)
if verb == "restart" and not known:
    print(f"Failed to restart {unit}: Unit {unit} not found.", file=sys.stderr)
    sys.exit(5)
if verb == "restart":
    code = state.get("restart_exit", 0)
    if state.get("restart_output"):
        print(state["restart_output"], file=sys.stderr)
    if code == 0:
        state.update(state.get("after_restart", {}))
        with open(path, "w", encoding="utf-8") as handle:
            json.dump(units, handle)
    sys.exit(code)
if verb in ("stop", "start"):
    # What activation does around putting a copy of the database back.
    if state.get(f"{verb}_exit"):
        print(f"fake systemctl: {verb} failed", file=sys.stderr)
        sys.exit(state[f"{verb}_exit"])
    state["ActiveState"] = "inactive" if verb == "stop" else "active"
    with open(path, "w", encoding="utf-8") as handle:
        json.dump(units, handle)
    sys.exit(0)
if verb == "is-active":
    print(active)
    sys.exit(0 if active == "active" else 3)
if verb == "status":
    print(f"● {unit} - fake status for the tests")
    print(f"     Active: {active} ({state.get('SubState', 'running')})")
    sys.exit(0 if active == "active" else 3)
print(f"fake systemctl: unexpected verb: {verb}", file=sys.stderr)
sys.exit(64)
"""

FAKE_JOURNALCTL = r"""#!/usr/bin/env python3
# ORBIT_HUB_FAKE_JOURNALCTL — written by scripts/tests/hub_harness.py.
import json, os, sys

STATE = @STATE@
arguments = sys.argv[1:]
with open(os.path.join(STATE, "events.jsonl"), "a", encoding="utf-8") as handle:
    handle.write(json.dumps({"tool": "journalctl", "argv": arguments}) + "\n")
unit = arguments[arguments.index("-u") + 1] if "-u" in arguments[:-1] else "?"
print(f"-- fake journal of {unit} --")
print(f"{unit}: fake journal line for the tests")
"""

FAKE_PS = r"""#!/usr/bin/env python3
# ORBIT_HUB_FAKE_PS — written by scripts/tests/hub_harness.py.
# It never reads this machine's processes: it prints processes.json, in the
# `pid ppid args` shape of `ps -o pid=,ppid=,args=`. An entry with "from_call"
# only shows from that call on, which is how a respawn that takes a while is
# simulated without a clock.
import json, os, sys

STATE = @STATE@
path = os.path.join(STATE, "processes.json")
with open(path, encoding="utf-8") as handle:
    table = json.load(handle)
table["calls"] = table.get("calls", 0) + 1
with open(path, "w", encoding="utf-8") as handle:
    json.dump(table, handle)
with open(os.path.join(STATE, "events.jsonl"), "a", encoding="utf-8") as handle:
    handle.write(json.dumps({"tool": "ps", "argv": sys.argv[1:], "call": table["calls"]}) + "\n")
for process in table["processes"]:
    if process.get("from_call", 0) <= table["calls"]:
        print(f"{process['pid']:>7} {process['ppid']:>7} {process['args']}")
"""

FAKE_HERMES = r"""#!/usr/bin/env python3
# ORBIT_HUB_FAKE_HERMES — written by scripts/tests/hub_harness.py.
# It never reaches Hermes: it records what it was asked and answers from
# hermes.json. On `gateway restart` it does what the real one did on 2026-10-15:
# the gateway keeps its PID and its orbit-mcp child is replaced by a new one,
# whose interpreter is the release `current` points to.
import json, os, sys

STATE = @STATE@
arguments = sys.argv[1:]
with open(os.path.join(STATE, "events.jsonl"), "a", encoding="utf-8") as handle:
    handle.write(json.dumps({"tool": "hermes", "argv": arguments, "env": {
        key: os.environ.get(key) for key in ("XDG_RUNTIME_DIR", "DBUS_SESSION_BUS_ADDRESS")
    }}) + "\n")
with open(os.path.join(STATE, "hermes.json"), encoding="utf-8") as handle:
    config = json.load(handle)
if arguments == ["-p", "deimos", "gateway", "restart"]:
    print(@DEIMOS_RESTART_OUTPUT@)
    sys.exit(1)
if arguments == ["-p", "deimos", "gateway", "status"]:
    print(@DEIMOS_STATUS_OUTPUT@)
    sys.exit(0)
if arguments[:3] != ["-p", "default", "gateway"] or len(arguments) != 4:
    print(f"fake hermes: unexpected command: {arguments}", file=sys.stderr)
    sys.exit(64)
verb = arguments[3]
if verb == "restart":
    if config.get("respawn", True):
        base = os.environ["ORBIT_HUB_BASE"]
        version = config.get("respawn_version") or os.path.basename(os.readlink(os.path.join(base, "current")))
        path = os.path.join(STATE, "processes.json")
        with open(path, encoding="utf-8") as handle:
            table = json.load(handle)
        gateway = config["gateway_pid"]
        kept = [p for p in table["processes"] if not (p["ppid"] == gateway and "orbit-mcp" in p["args"])]
        kept.append({
            "pid": max(p["pid"] for p in table["processes"]) + 1,
            "ppid": gateway,
            "args": f"{base}/releases/{version}/.venv/bin/python {base}/current/.venv/bin/orbit-mcp",
            "from_call": table.get("calls", 0) + 1 + config.get("respawn_after_calls", 0),
        })
        table["processes"] = kept
        with open(path, "w", encoding="utf-8") as handle:
            json.dump(table, handle)
    if config.get("restart_output"):
        print(config["restart_output"])
    sys.exit(config.get("restart_exit", 0))
if verb == "status":
    print(config.get("status_output", ""))
    sys.exit(config.get("status_exit", 0))
print(f"fake hermes: unexpected verb: {verb}", file=sys.stderr)
sys.exit(64)
"""


def find_uv():
    """The uv that builds releases here, or None."""
    found = shutil.which("uv")
    if found:
        return found
    fallback = Path.home() / ".local" / "bin" / "uv"
    return str(fallback) if fallback.is_file() and os.access(fallback, os.X_OK) else None


def systemd_exec(command):
    """How `systemctl show -p ExecStart --value` renders a command line."""
    return (
        f"{{ path={command.split()[0]} ; argv[]={command} ; ignore_errors=no ;"
        " start_time=[n/a] ; stop_time=[n/a] ; pid=0 ; code=(null) ; status=0/0 }"
    )


def release_fields(path):
    """The key=value lines of a RELEASE file."""
    fields = {}
    for line in Path(path).read_text(encoding="utf-8").splitlines():
        key, separator, value = line.partition("=")
        if separator:
            fields[key] = value
    return fields


def lock_version(text):
    """The version uv.lock records for the orbit-hub project itself."""
    found = re.search(r'(?m)^name = "orbit-hub"\nversion = "([^"]+)"', text)
    return found.group(1) if found else None


def project_version(text):
    found = re.search(r'(?m)^version = "(\d+\.\d+\.\d+)"', text)
    return found.group(1) if found else None


class FakeHub:
    """/health and /openapi.json on loopback, answering like the hub does.

    The version it reports is read from <base>/current/RELEASE at request time,
    which is what a real hub restarted from `current` would report, unless a
    test pins another one to prove activation notices.
    """

    def __init__(self, sandbox):
        self.sandbox = sandbox
        self.answering = True
        self.version = None
        self.describes_itself = True
        hub = self

        class Handler(http.server.BaseHTTPRequestHandler):
            def log_message(self, *_arguments):
                pass

            def do_GET(self):
                if not hub.answering:
                    status, body = 503, b'{"detail":"arrancando"}'
                elif self.path == "/health":
                    status, body = 200, b'{"ok":true,"protocol":1}'
                elif self.path == "/openapi.json" and not hub.describes_itself:
                    status, body = 404, b'{"detail":"Not Found"}'
                elif self.path == "/openapi.json":
                    status = 200
                    body = json.dumps(
                        {
                            "openapi": "3.1.0",
                            "info": {
                                "title": "Orbit Hub",
                                "version": hub.reported_version(),
                            },
                            "paths": {},
                        }
                    ).encode()
                else:
                    status, body = 404, b"{}"
                hub.sandbox.record({"tool": "hub", "path": self.path, "status": status})
                self.send_response(status)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)

        self.server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.url = f"http://127.0.0.1:{self.server.server_address[1]}"
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()

    def reported_version(self):
        if self.version is not None:
            return self.version
        try:
            return release_fields(self.sandbox.base / "current" / "RELEASE").get(
                "version", "desconocida"
            )
        except OSError:
            return "desconocida"

    def close(self):
        self.server.shutdown()
        self.server.server_close()
        self.thread.join(timeout=10)


class HubSandbox:
    """One temporary world for the hub scripts. Always close() it."""

    def __init__(self):
        self.root = Path(tempfile.mkdtemp(prefix="orbit-hub-sandbox-"))
        self.base = self.root / "base"
        self.bin = self.root / "bin"
        self.state = self.root / "fake"
        self.repo = self.root / "repo"
        self.profile = self.root / "deimos-config.yaml"
        self.gitconfig = self.root / "gitconfig"
        for directory in (self.bin, self.state, self.repo):
            directory.mkdir()
        self.gitconfig.write_text("", encoding="utf-8")
        (self.state / "events.jsonl").write_text("", encoding="utf-8")
        self._events_lock = threading.Lock()
        state_literal = repr(str(self.state))
        self._write_executable(
            self.bin / "systemctl", FAKE_SYSTEMCTL.replace("@STATE@", state_literal)
        )
        self._write_executable(
            self.bin / "journalctl", FAKE_JOURNALCTL.replace("@STATE@", state_literal)
        )
        self._write_executable(self.bin / "ps", FAKE_PS.replace("@STATE@", state_literal))
        self.hermes = self.bin / "hermes"
        self._write_executable(
            self.hermes,
            FAKE_HERMES.replace("@STATE@", state_literal)
            .replace("@DEIMOS_RESTART_OUTPUT@", repr(DEIMOS_RESTART_OUTPUT))
            .replace("@DEIMOS_STATUS_OUTPUT@", repr(DEIMOS_STATUS_OUTPUT)),
        )
        # The hub's configuration as its unit names it: only data_dir and
        # key_file are read (activation copies the database). No database
        # exists until a test makes one (make_database).
        self.data_dir = self.root / "data"
        self.key_file = self.root / "storage.key"
        (self.root / "hub.toml").write_text(
            f'data_dir = "{self.data_dir}"\nkey_file = "{self.key_file}"\n', encoding="utf-8"
        )
        self.write_units(self.default_units())
        self.write_processes(self.default_processes())
        self.write_hermes({})
        self.profile.write_text(self.profile_text(current=True), encoding="utf-8")
        self.hub = FakeHub(self)

    # -- lifecycle ---------------------------------------------------------

    def close(self):
        self.hub.close()
        # Published releases are read-only on purpose; give the owner write back
        # so the temporary tree can go.
        for directory, subdirectories, files in os.walk(self.root):
            for name in subdirectories + files:
                path = Path(directory) / name
                if not path.is_symlink():
                    path.chmod(path.stat().st_mode | stat.S_IWUSR)
        self.root.chmod(self.root.stat().st_mode | stat.S_IWUSR)
        shutil.rmtree(self.root, ignore_errors=True)

    @staticmethod
    def _write_executable(path, text):
        path.write_text(text, encoding="utf-8")
        path.chmod(0o755)

    # -- the fake systemd --------------------------------------------------

    def default_units(self):
        rendered = subprocess.run(
            ["bash", str(HUB_SCRIPTS / "render-unit.sh")],
            env={
                "PATH": os.environ["PATH"],
                "ORBIT_HUB_BASE": str(self.base),
                "ORBIT_HUB_CONFIG": str(self.root / "hub.toml"),
                "ORBIT_HUB_ENV_FILE": str(self.root / "hub.env"),
            },
            check=True,
            capture_output=True,
            text=True,
            timeout=10,
        ).stdout
        unit = dict(
            line.split("=", 1)
            for line in rendered.splitlines()
            if "=" in line and not line.startswith("#")
        )
        return {
            HUB_UNIT: {
                "ExecStart": systemd_exec(unit["ExecStart"]),
                "WorkingDirectory": unit["WorkingDirectory"],
                "NeedDaemonReload": "no",
                "ActiveState": "active",
                "SubState": "running",
            },
            SERVE_UNIT: {"ActiveState": "active", "SubState": "running"},
        }

    def write_units(self, units):
        (self.state / "units.json").write_text(json.dumps(units), encoding="utf-8")

    def set_unit(self, unit, **properties):
        units = json.loads((self.state / "units.json").read_text(encoding="utf-8"))
        units.setdefault(unit, {}).update(properties)
        self.write_units(units)

    # -- the hub's database ------------------------------------------------

    def make_database(self, rows=("Acta de la Cooperativa Brezo",)):
        """A synthetic hub database and its key: plain SQLite, no hub code."""
        self.data_dir.mkdir(mode=0o700, exist_ok=True)
        self.key_file.write_bytes(bytes(range(32)))
        self.key_file.chmod(0o600)
        database = sqlite3.connect(self.data_dir / "hub.sqlite3", isolation_level=None)
        try:
            database.execute("PRAGMA journal_mode=WAL")
            database.execute("CREATE TABLE IF NOT EXISTS notes (id INTEGER PRIMARY KEY, body TEXT)")
            for row in rows:
                database.execute("INSERT INTO notes(body) VALUES (?)", (row,))
        finally:
            database.close()

    def migrate_database(self):
        """What a newer release's first start does: a schema the older one never had."""
        database = sqlite3.connect(self.data_dir / "hub.sqlite3", isolation_level=None)
        try:
            database.execute("ALTER TABLE notes ADD COLUMN synthetic_future TEXT")
        finally:
            database.close()

    def database_rows(self, path=None):
        database = sqlite3.connect(path or self.data_dir / "hub.sqlite3")
        try:
            return [row[0] for row in database.execute("SELECT body FROM notes ORDER BY id")]
        finally:
            database.close()

    # -- the fake process table and the fake hermes --------------------------

    def default_processes(self):
        """The host gateway with Deimos's translator from the development tree,
        a gateway in a container with none, and hermes-serve-orbit."""
        return [
            {
                "pid": USER_MANAGER_PID,
                "ppid": 1,
                "args": "/usr/lib/systemd/systemd --user",
            },
            {"pid": GATEWAY_PID, "ppid": USER_MANAGER_PID, "args": GATEWAY_ARGS},
            {
                "pid": FIRST_TRANSLATOR_PID,
                "ppid": GATEWAY_PID,
                "args": DEVELOPMENT_TRANSLATOR_ARGS,
            },
            {"pid": CONTAINER_GATEWAY_PID, "ppid": 1, "args": CONTAINER_GATEWAY_ARGS},
            {"pid": SERVE_PID, "ppid": USER_MANAGER_PID, "args": SERVE_ARGS},
        ]

    def write_processes(self, processes):
        (self.state / "processes.json").write_text(
            json.dumps({"calls": 0, "processes": processes}), encoding="utf-8"
        )

    def add_process(self, pid, ppid, args):
        path = self.state / "processes.json"
        table = json.loads(path.read_text(encoding="utf-8"))
        table["processes"].append({"pid": pid, "ppid": ppid, "args": args})
        path.write_text(json.dumps(table), encoding="utf-8")

    def processes(self):
        return json.loads((self.state / "processes.json").read_text(encoding="utf-8"))["processes"]

    def gateway_translators(self):
        """The args of every orbit-mcp child of the host gateway in the listing."""
        return [
            p["args"]
            for p in self.processes()
            if p["ppid"] == GATEWAY_PID and "orbit-mcp" in p["args"]
        ]

    def write_hermes(self, config):
        """How the fake hermes answers; see FAKE_HERMES for the keys."""
        config = {
            "gateway_pid": GATEWAY_PID,
            "status_output": HOST_STATUS_OUTPUT,
            **config,
        }
        (self.state / "hermes.json").write_text(json.dumps(config), encoding="utf-8")

    def hermes_calls(self):
        return [entry["argv"] for entry in self.events("hermes")]

    def record(self, entry):
        with self._events_lock:
            with (self.state / "events.jsonl").open("a", encoding="utf-8") as handle:
                handle.write(json.dumps(entry) + "\n")

    def events(self, tool=None):
        lines = (self.state / "events.jsonl").read_text(encoding="utf-8").splitlines()
        entries = [json.loads(line) for line in lines if line.strip()]
        return [entry for entry in entries if tool is None or entry["tool"] == tool]

    def clear_events(self):
        (self.state / "events.jsonl").write_text("", encoding="utf-8")

    def restarts(self):
        """What was restarted, in order: systemd units by name, the gateway as GATEWAY_RESTART."""
        found = []
        for entry in self.events():
            if (
                entry["tool"] == "systemctl"
                and len(entry["argv"]) > 1
                and entry["argv"][1] == "restart"
            ):
                found.append(entry["argv"][-1])
            elif entry["tool"] == "hermes" and entry["argv"] == GATEWAY_RESTART_ARGV:
                found.append(GATEWAY_RESTART)
        return found

    def profile_text(self, current):
        command = (
            f"{self.base}/current/.venv/bin/orbit-mcp"
            if current
            else "/home/example/projects/orbit/server/.venv/bin/orbit-mcp"
        )
        return (
            "mcp_servers:\n"
            "  orbit:\n"
            f"    command: {command}\n"
            "    env:\n"
            "      ORBIT_MCP_CONFIG: /home/example/.config/orbit/mcp.toml\n"
            "    enabled: true\n"
        )

    # -- releases and the repository ---------------------------------------

    def make_release(self, version, complete=True):
        """A synthetic release with the shape activate.sh checks, no Python in it."""
        release = self.base / "releases" / version
        (release / ".venv" / "bin").mkdir(parents=True)
        for name in ("orbit-hub", "orbit-mcp"):
            self._write_executable(release / ".venv" / "bin" / name, "#!/bin/sh\nexit 0\n")
        if complete:
            (release / "RELEASE").write_text(
                f"version={version}\n"
                f"commit={'a' * 40}\n"
                "commit_date=2026-09-25T10:00:00+02:00\n"
                "published_at=2026-09-25T08:00:00Z\n"
                "python=3.13.15\n",
                encoding="utf-8",
            )
        return release

    def git_env(self):
        env = dict(os.environ)
        env.update(
            GIT_CONFIG_GLOBAL=str(self.gitconfig),
            GIT_CONFIG_NOSYSTEM="1",
            GIT_AUTHOR_NAME="Orbit pruebas",
            GIT_AUTHOR_EMAIL="pruebas@orbit.invalid",
            GIT_COMMITTER_NAME="Orbit pruebas",
            GIT_COMMITTER_EMAIL="pruebas@orbit.invalid",
        )
        return env

    def git(self, *arguments):
        return subprocess.run(
            ["git", "-C", str(self.repo), *arguments],
            env=self.git_env(),
            capture_output=True,
            text=True,
            check=True,
        ).stdout

    def make_repo(self):
        """A repository whose only content is this tree's server/, committed."""
        shutil.copytree(
            ROOT / "server",
            self.repo / "server",
            ignore=shutil.ignore_patterns(
                ".venv",
                "__pycache__",
                ".pytest_cache",
                ".ruff_cache",
                "*.sqlite*",
                "*.key",
            ),
        )
        self.git("init", "-q", "-b", "main")
        return self.commit("server of this tree")

    def commit(self, message):
        self.git("add", "-A")
        self.git("commit", "-q", "--no-gpg-sign", "-m", message)
        return self.git("rev-parse", "HEAD").strip()

    def repo_file(self, name):
        return self.repo / "server" / name

    def repo_version(self):
        return project_version(self.repo_file("pyproject.toml").read_text(encoding="utf-8"))

    def set_repo_version(self, version, lock=True):
        """Change the version the way scripts/release/set-version.py does, or only in pyproject."""
        pyproject = self.repo_file("pyproject.toml")
        text = pyproject.read_text(encoding="utf-8")
        pyproject.write_text(
            re.sub(
                r'(?m)^version = "\d+\.\d+\.\d+"',
                f'version = "{version}"',
                text,
                count=1,
            ),
            encoding="utf-8",
        )
        if lock:
            path = self.repo_file("uv.lock")
            text = path.read_text(encoding="utf-8")
            path.write_text(
                re.sub(
                    r'(?m)^(name = "orbit-hub"\nversion = ")[^"]+(")',
                    lambda match: match.group(1) + version + match.group(2),
                    text,
                    count=1,
                ),
                encoding="utf-8",
            )

    # -- running the scripts -----------------------------------------------

    def env(self, **overrides):
        env = {
            key: value
            for key, value in os.environ.items()
            if not key.startswith(("ORBIT_", "XDG_RUNTIME_DIR", "DBUS_SESSION_BUS_ADDRESS"))
            and key not in ("VIRTUAL_ENV", "UV_PROJECT_ENVIRONMENT", "PYTHONPATH")
        }
        env.update(
            PATH=f"{self.bin}{os.pathsep}{os.environ.get('PATH', '')}",
            ORBIT_HUB_UNDER_TEST="1",
            ORBIT_HUB_BASE=str(self.base),
            ORBIT_HUB_URL=self.hub.url,
            ORBIT_DEIMOS_PROFILE=str(self.profile),
            ORBIT_HUB_HERMES=str(self.hermes),
            # Polls of the fake listing are instant; the default 1 s pause
            # would only make the tests slower, not truer.
            ORBIT_HUB_GATEWAY_POLL="0.05",
            ORBIT_REPO=str(self.repo),
            GIT_CONFIG_GLOBAL=str(self.gitconfig),
            GIT_CONFIG_NOSYSTEM="1",
            UV_OFFLINE="1",
        )
        uv = find_uv()
        if uv:
            env["ORBIT_UV"] = uv
        env.update(overrides)
        return env

    def _check_redirected(self, env):
        """Refuse, in the test itself, to start a hub script that could reach the machine."""
        base = Path(env.get("ORBIT_HUB_BASE", UNCONFIGURED_BASE))
        first = env.get("PATH", "").split(os.pathsep)[0]
        problems = []
        if self.root not in base.parents:
            problems.append(f"the base {base} is not in {self.root}")
        if first != str(self.bin):
            problems.append(f"the PATH does not start with the fakes ({first})")
        for tool in ("systemctl", "journalctl", "ps", "hermes"):
            if FAKE_MARK not in (self.bin / tool).read_text(encoding="utf-8"):
                problems.append(f"{tool} is not the fake")
        hermes = Path(env.get("ORBIT_HUB_HERMES", "/"))
        if self.root not in hermes.parents:
            problems.append(f"ORBIT_HUB_HERMES ({hermes}) is not in {self.root}")
        elif hermes.exists() and FAKE_MARK not in hermes.read_text(encoding="utf-8"):
            problems.append(f"ORBIT_HUB_HERMES ({hermes}) is not the fake")
        if env.get("ORBIT_HUB_URL") != self.hub.url:
            problems.append("ORBIT_HUB_URL is not the fake hub")
        if Path(env.get("ORBIT_DEIMOS_PROFILE", "/")).parent != self.root:
            problems.append("ORBIT_DEIMOS_PROFILE is not in the temporary directory")
        if env.get("ORBIT_HUB_UNDER_TEST") != "1":
            problems.append("ORBIT_HUB_UNDER_TEST is missing")
        if problems:
            raise AssertionError("test without redirection: " + "; ".join(problems))

    def run(self, script, *arguments, env=None, timeout=300):
        env = self.env() if env is None else env
        self._check_redirected(env)
        return self._spawn([str(HUB_SCRIPTS / script), *arguments], env, timeout)

    def source_lib(self, snippet, env=None, timeout=60):
        """Source lib.sh in bash and run `snippet` after it, fully redirected."""
        env = self.env() if env is None else env
        self._check_redirected(env)
        return self._source(snippet, env, timeout)

    def _source(self, snippet, env, timeout):
        return self._spawn(
            ["bash", "-c", f'source "$1"\n{snippet}', "bash", str(LIB)], env, timeout
        )

    def _spawn(self, command, env, timeout):
        try:
            return subprocess.run(
                command,
                env=env,
                cwd=self.root,
                capture_output=True,
                text=True,
                timeout=timeout,
                stdin=subprocess.DEVNULL,
            )
        except (FileNotFoundError, PermissionError) as error:
            return subprocess.CompletedProcess(command, 127, "", f"could not run: {error}")
