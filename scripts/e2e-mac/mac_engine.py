"""CuaDriver for the real test's Mac side: start it bounded by its capability
manifest, one tool call, and stop the one this test started.

`CuaDriver` is mixed into mac.py's `Mac`: its methods use the Mac's paths and
system, the capability manifest it writes (`manifest`) and Orbit Pruebas's pid
(`_app_pid`). Copied to the Mac with mac.py, which imports it; runs with the
Mac's python3, so it stays within Python 3.9.
"""

from __future__ import annotations

import json
import os
import re
import signal
import time

from mac_controls import MacFailure
from mac_drafts_panel import SOCKET_RETRY_S

# Where the official installer puts CuaDriver's daemon; a pid is the test's
# only if its executable is in there.
CUA_APP = "/Applications/CuaDriver.app/"
# How CuaDriver words a refusal on permissions (authorization.rs,
# session_manifest.rs, the macOS permission gate).
PERMISSION_REFUSALS = (
    "Permission denied",
    "outside the capability manifest",
    "capability manifest denies",
    "permission",
)


class CuaDriver:
    def _cua_status(self) -> str | None:
        """`cua-driver status`, or None when no CuaDriver daemon is running."""
        code, output, _ = self.system.run(str(self.paths.cua), "status")
        return output if code == 0 else None

    def engine_start(self, max_s: float) -> dict:
        """Starts CuaDriver's daemon under its own macOS identity (LaunchServices,
        so its own Accessibility and Screen Recording grants apply), bounded by
        the manifest, and checks that it took it."""
        if self._cua_status() is not None:
            raise MacFailure(
                "a CuaDriver is already running; the test does not touch it: stop it with "
                "«cua-driver stop» and launch the test again"
            )
        app_pid = self._app_pid()
        self.paths.own.mkdir(mode=0o700, exist_ok=True)
        descriptor = os.open(self.paths.manifest, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
        with os.fdopen(descriptor, "w", encoding="utf-8") as file:
            file.write(self.manifest(app_pid))
        self.system.run_checked(
            "open", "-n", "-g", "-a", "CuaDriver", "--args", "serve",
            "--permission-mode", "bounded",
            "--capability-manifest", str(self.paths.manifest),
            "--approve-capability-manifest",
        )  # fmt: skip
        start = self.system.now()
        while True:
            status = self._cua_status()
            pid = re.search(r"^\s*pid: (\d+)$", status or "", re.MULTILINE)
            if pid is not None:
                break
            if self.system.now() - start > max_s:
                raise MacFailure(f"CuaDriver did not answer within {max_s:.0f} s of starting it")
            time.sleep(SOCKET_RETRY_S)
        self.paths.cua_pid.write_text(pid.group(1) + "\n", encoding="utf-8")
        mode = re.search(r"permission mode: (\w+)", status or "")
        manifest = "capability manifest: configured=true, approved_at_startup=true, valid=true"
        if mode is None or mode.group(1) != "bounded" or manifest not in (status or ""):
            raise MacFailure(
                "CuaDriver did not start in bounded mode with its manifest: "
                + " | ".join(line.strip() for line in (status or "").splitlines()[1:])
            )
        return {"pid": int(pid.group(1)), "app_pid": app_pid}

    def engine_call(self, tool: str, arguments: dict) -> dict:
        """One tool call. A refusal is said by its name: the driver refusing its
        own authorization process, or a permission the manifest or macOS does
        not give, never a bare failure."""
        code, output, errors = self.system.run(
            str(self.paths.cua), "call", tool, json.dumps(arguments, ensure_ascii=False)
        )
        if code != 0:
            cause = errors.strip()[:500]
            if "own authorization process" in cause:
                raise MacFailure(
                    f"CuaDriver refuses to act on its own authorization process "
                    f"(pid {arguments.get('pid')}) in {tool}: {cause}"
                )
            if any(mark in cause for mark in PERMISSION_REFUSALS):
                raise MacFailure(f"CuaDriver refused {tool} for permissions: {cause}")
            raise MacFailure(f"CuaDriver failed in {tool} (code {code}): {cause}")
        try:
            result = json.loads(output)
        except ValueError:
            result = {"text": output.strip()}
        return {"result": result, "bytes": len(output.encode())}

    def engine_stop(self, max_s: float) -> dict:
        """Stops the CuaDriver this test started: by the pid it wrote down,
        and only if that pid is still CuaDriver's."""
        if not self.paths.cua_pid.exists():
            return {"stopped": True, "pid": None}
        pid = int(self.paths.cua_pid.read_text(encoding="utf-8").strip())
        ours = any(
            p == pid and command.startswith(CUA_APP) for p, command in self.system.processes()
        )
        if ours:
            self.system.run(str(self.paths.cua), "--expected-pid", str(pid), "stop")
            if self.system.wait_exit([pid], max_s):
                self.system.signal(pid, signal.SIGTERM)
        self.paths.cua_pid.unlink()
        return {"stopped": True, "pid": pid}
