"""The Mac's commands and processes for the real test's Mac side: run a
command, list the processes, signal one and wait on its exit, the clock.

`Mac` (mac.py) takes a `System`; the dry run swaps in one that answers like a
Mac would. Copied to the Mac with mac.py, which imports it; runs with the
Mac's python3, so it stays within Python 3.9.
"""

from __future__ import annotations

import json
import os
import select
import shutil
import subprocess
import time
from contextlib import contextmanager

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

    def wall_ms(self) -> int:
        return int(time.time() * 1000)

    def pause(self, seconds: float) -> None:
        time.sleep(seconds)

    def local_time(self, seconds: float) -> time.struct_time:
        """The Mac's local time, the clock the app names a conversation by."""
        return time.localtime(seconds)

    def audio_state(self, logs, pid):
        latest = None
        for name in ("daemon.log.1", "daemon.log"):
            path = logs / name
            if not path.exists():
                continue
            for line in path.read_text(encoding="utf-8").splitlines():
                try:
                    row = json.loads(line)
                except ValueError:
                    continue
                if row.get("event") != "acceptance_audio_state" or row.get("pid") != pid:
                    continue
                fields = row.get("fields", {})
                keys = ("privacy_stopped", "microphone", "system_audio", "playing")
                if all(fields.get(k) in ("true", "false") for k in keys):
                    latest = {k: fields[k] == "true" for k in keys}
        return latest

    @contextmanager
    def speaking(self, paths, text):
        # A finite passage lasts beyond these short native checks, with no
        # speaker output. The child is owned by this call and always reaped.
        passage = " ".join(f"{text} Passage {n}." for n in range(100))
        process = subprocess.Popen(
            ["say", "-a", "BlackHole 2ch", passage],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        try:
            yield process
            if process.poll() is not None:
                raise MacFailure("fixture speech ended before the native check completed")
        finally:
            if process.poll() is None:
                process.terminate()
            process.wait(timeout=10)

    @contextmanager
    def silent_output(self):
        previous = self.run_checked("SwitchAudioSource", "-c", "-t", "output").strip()
        try:
            self.run_checked("SwitchAudioSource", "-t", "output", "-s", "BlackHole 2ch")
            observed = self.run_checked("SwitchAudioSource", "-c", "-t", "output").strip()
            if observed != "BlackHole 2ch":
                raise MacFailure("playback is not routed to BlackHole; no playback started")
            yield
        finally:
            self.run_checked("SwitchAudioSource", "-t", "output", "-s", previous)

    def clear_clipboard(self):
        subprocess.run(["pbcopy"], input=b"", check=True, timeout=10)
