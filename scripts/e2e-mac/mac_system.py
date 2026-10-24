"""The Mac's commands and processes for the real test's Mac side: run a
command, list the processes, signal one and wait on its exit, the clock.

`Mac` (mac.py) takes a `System`; the dry run swaps in one that answers like a
Mac would. Copied to the Mac with mac.py, which imports it; runs with the
Mac's python3, so it stays within Python 3.9.
"""

from __future__ import annotations

import os
import select
import shutil
import subprocess
import time

from mac_controls import MacFailure


class System:
    def run(self, *command: str) -> tuple[int, str, str]:
        """Runs a command to its end: exit code, stdout and stderr."""
        done = subprocess.run(list(command), capture_output=True, text=True, check=False)
        return done.returncode, done.stdout, done.stderr

    def run_checked(self, *command: str) -> str:
        code, output, errors = self.run(*command)
        if code != 0:
            raise MacFailure(f"«{command[0]}» exited with {code}: {errors.strip()[:300]}")
        return output

    def has_command(self, name: str) -> bool:
        return shutil.which(name) is not None

    def processes(self) -> list[tuple[int, str]]:
        output = self.run_checked("ps", "-axo", "pid=,comm=")
        rows = []
        for line in output.splitlines():
            parts = line.strip().split(None, 1)
            if len(parts) == 2 and parts[0].isdigit():
                rows.append((int(parts[0]), parts[1]))
        return rows

    def signal(self, pid: int, number: int) -> None:
        try:
            os.kill(pid, number)
        except ProcessLookupError:
            pass

    def wait_exit(self, pids: list[int], max_s: float) -> list[int]:
        """Waits on the exit of each process (kqueue NOTE_EXIT); returns the
        ones still alive at the bound."""
        queue = select.kqueue()
        alive = set()
        for pid in pids:
            event = select.kevent(
                pid,
                select.KQ_FILTER_PROC,
                select.KQ_EV_ADD | select.KQ_EV_ONESHOT,
                select.KQ_NOTE_EXIT,
            )
            try:
                queue.control([event], 0, 0)
                alive.add(pid)
            except ProcessLookupError:
                continue
        deadline = time.monotonic() + max_s
        while alive:
            left = deadline - time.monotonic()
            if left <= 0:
                break
            for event in queue.control(None, len(alive), left):
                alive.discard(event.ident)
        queue.close()
        return sorted(alive)

    def now(self) -> float:
        return time.monotonic()

    def pause(self, seconds: float) -> None:
        time.sleep(seconds)

    def local_time(self, seconds: float) -> time.struct_time:
        """The Mac's local time, the clock the app names a conversation by."""
        return time.localtime(seconds)
