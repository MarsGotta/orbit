#!/usr/bin/env python3
"""Run an explicitly selected, trusted OrbitNative build in its isolated fixture mode.

This executes the native binary, unlike verify-macos-archive.py. It checks only
real stdio, quiet/latched state, a private latch file and acknowledged process
shutdown. It is NOT evidence for TCC, audio, Accessibility, login items, Keychain,
production storage or a normal installed-app launch. Use a fresh build that
implements --test-state-directory; this cannot sandbox an untrusted executable.
No HOME override, installation, provider call or capability request is performed.

Usage: python3 -B scripts/smoke-native-macos.py --binary <built OrbitNative>
"""

import argparse
import json
import math
import os
import selectors
import shutil
import stat
import subprocess
import sys
import tempfile
import time
import uuid
from pathlib import Path

MAX_LINE = 1_048_576
MAX_STDOUT = 2_097_152
MAX_STDERR = 65_536
CHUNK = 16_384
HEARTBEAT_SECONDS = 2.0
CLEANUP_SECONDS = 3.0
QUIET = {
    "privacy_stopped": True,
    "connected": True,
    "microphone": False,
    "system_audio": False,
    "playing": False,
}


class SmokeError(Exception):
    """Only fixed local codes reach the report, never native output or paths."""

    def __init__(self, code):
        super().__init__(code)
        self.code = code


def require(condition, code):
    if not condition:
        raise SmokeError(code)


def unique_object(pairs):
    result = {}
    for key, value in pairs:
        require(key not in result, "duplicate_json_key")
        result[key] = value
    return result


def finite_number(value):
    number = float(value)
    require(math.isfinite(number), "nonfinite_json_number")
    return number


def reject_constant(_value):
    raise SmokeError("nonfinite_json_number")


def unicode_scalars(value):
    if isinstance(value, str):
        value.encode("utf-8", errors="strict")
    elif isinstance(value, dict):
        for key, child in value.items():
            unicode_scalars(key)
            unicode_scalars(child)
    elif isinstance(value, list):
        for child in value:
            unicode_scalars(child)


def frame_from_bytes(raw):
    require(0 < len(raw) <= MAX_LINE, "invalid_stdout_line_size")
    try:
        value = json.loads(
            raw.decode("utf-8", errors="strict"),
            object_pairs_hook=unique_object,
            parse_float=finite_number,
            parse_constant=reject_constant,
        )
        unicode_scalars(value)
    except (ValueError, UnicodeError, RecursionError):
        raise SmokeError("invalid_stdout_json") from None
    require(type(value) is dict, "invalid_stdout_frame")
    require(
        type(value.get("v")) is int and value["v"] == 1, "protocol_version_mismatch"
    )
    return value


class Lines:
    def __init__(self):
        self.pending = bytearray()

    def feed(self, data):
        self.pending.extend(data)
        result = []
        while (end := self.pending.find(b"\n")) >= 0:
            require(end <= MAX_LINE, "invalid_stdout_line_size")
            result.append(frame_from_bytes(bytes(self.pending[:end])))
            del self.pending[: end + 1]
        require(len(self.pending) <= MAX_LINE, "invalid_stdout_line_size")
        return result

    def finish(self):
        require(not self.pending, "truncated_stdout_line")


def quiet_snapshot(value):
    require(type(value) is dict, "invalid_state")
    for key, expected in QUIET.items():
        require(value.get(key) is expected, "state_not_quiet_and_latched")
    for key in (
        "stream_id",
        "microphone_stream_id",
        "system_audio_stream_id",
        "streams",
    ):
        if key in value:
            item = value[key]
            require(
                item is None or item == "" or item == [] or item == {},
                "unexpected_stream",
            )
    # Permission preflight facts belong to the OS; they are not requested or
    # asserted here. Likewise, do not fabricate legacy AEC/processing availability.


class Exchange:
    """Only the five allowlisted commands; no configurable capability workflow."""

    def __init__(self, now):
        self.phase = "ready"
        self.pending = {}
        self.outgoing = bytearray()
        self.last_sent = now
        self.checked = []
        self.heartbeats = 0

    def queue(self, command, now):
        require(
            command in ("hello", "status", "privacy_stop", "shutdown", "heartbeat"),
            "invalid_runner_command",
        )
        request_id = str(uuid.uuid4())
        frame = (
            json.dumps(
                {"v": 1, "id": request_id, "command": command}, separators=(",", ":")
            ).encode("ascii")
            + b"\n"
        )
        require(len(self.outgoing) + len(frame) <= CHUNK, "runner_output_limit")
        self.pending[request_id] = command
        self.outgoing.extend(frame)
        self.last_sent = now

    def heartbeat(self, now):
        if (
            self.phase not in ("ready", "shutdown", "exit")
            and now - self.last_sent >= HEARTBEAT_SECONDS
            and "heartbeat" not in self.pending.values()
        ):
            self.queue("heartbeat", now)

    def accept(self, frame, now):
        event = frame.get("event")
        if event == "ready":
            require(self.phase == "ready", "duplicate_ready")
            require(
                frame.get("fixture") is True
                and frame.get("launch_mode") == "isolated_test_stdio",
                "not_isolated_fixture",
            )
            require(
                type(frame.get("protocol")) is int and frame["protocol"] == 1,
                "protocol_version_mismatch",
            )
            require(
                frame.get("selected_text_available") is False,
                "fixture_selection_mismatch",
            )
            require(
                frame.get("supported_audio_processing") == ["unprocessed", "voice_processing"]
                and frame.get("voice_processing_available") is False
                and frame.get("playback_reference_available") is False,
                "fixture_audio_processing_mismatch",
            )
            self.checked.append("isolated_ready")
            self.phase = "hello"
            self.queue("hello", now)
            return
        require(self.phase != "ready", "frame_before_ready")
        if event == "state":
            quiet_snapshot(frame)
            return
        if event == "privacy_stop":
            require(frame.get("privacy_stopped") is True, "privacy_not_latched")
            require(type(frame.get("drain_complete")) is bool, "invalid_privacy_event")
            return  # An event is not the correlated drain acknowledgement.
        if event == "capture_stopped":
            # REF-12: a stop that is not Mars's brake (here the shutdown) stops
            # capture and leaves the latch as it was; the fixture's stays set.
            require(frame.get("privacy_stopped") is True, "fixture_latch_lost")
            require(type(frame.get("drain_complete")) is bool, "invalid_capture_event")
            return  # Neither is this event the drain acknowledgement.
        require(event == "response", "unexpected_native_event")
        request_id = frame.get("id")
        require(
            type(request_id) is str and request_id in self.pending,
            "unexpected_response_id",
        )
        require(frame.get("ok") is True, "native_request_rejected")
        command = self.pending[request_id]
        result = frame.get("result")
        require(type(result) is dict, "invalid_response_result")
        if command == "shutdown":
            require(result.get("drain_complete") is True, "shutdown_drain_unconfirmed")
        else:
            quiet_snapshot(result)
            if command == "privacy_stop":
                require(
                    result.get("drain_complete") is True, "privacy_drain_unconfirmed"
                )
        del self.pending[request_id]
        if command == "heartbeat":
            self.heartbeats += 1
            return
        require(command == self.phase, "response_out_of_phase")
        self.checked.append(command)
        self.phase = {
            "hello": "status",
            "status": "privacy_stop",
            "privacy_stop": "shutdown",
            "shutdown": "exit",
        }[command]
        if self.phase != "exit":
            self.queue(self.phase, now)

    def finish(self):
        require(
            self.phase == "exit" and not self.pending and not self.outgoing,
            "incomplete_shutdown_exchange",
        )


class Pipes:
    def __init__(self, process):
        self.process = process
        self.selector = selectors.DefaultSelector()
        self.counts = {"stdout": 0, "stderr": 0}
        self.limits = {"stdout": MAX_STDOUT, "stderr": MAX_STDERR}
        self.eof = set()
        self.writing = False
        for name in ("stdin", "stdout", "stderr"):
            handle = getattr(process, name)
            os.set_blocking(handle.fileno(), False)
            if name != "stdin":
                self.selector.register(handle, selectors.EVENT_READ, name)

    def wants_write(self, wanted):
        if wanted and not self.writing:
            self.selector.register(self.process.stdin, selectors.EVENT_WRITE, "stdin")
        elif not wanted and self.writing:
            self.selector.unregister(self.process.stdin)
        self.writing = wanted

    def read(self, name, *, cleanup=False):
        remaining = self.limits[name] - self.counts[name]
        if cleanup and remaining <= 0:
            self.selector.unregister(getattr(self.process, name))
            return None
        try:
            data = os.read(
                getattr(self.process, name).fileno(), min(CHUNK, max(0, remaining) + 1)
            )
        except (BlockingIOError, InterruptedError):
            return None
        self.counts[name] += len(data)
        if not data:
            self.eof.add(name)
            self.selector.unregister(getattr(self.process, name))
        if self.counts[name] > self.limits[name]:
            if cleanup:
                self.selector.unregister(getattr(self.process, name))
            else:
                raise SmokeError(name + "_limit")
        return data

    def cleanup_drain(self, timeout):
        self.wants_write(False)  # Stop commands, but keep stdin OPEN until exit.
        try:
            readable = self.selector.select(max(0, timeout))
        except (OSError, ValueError):
            time.sleep(min(0.01, max(0, timeout)))
            return  # Cleanup must still reach TERM/KILL after an I/O failure.
        for key, _ in readable:
            if key.data in self.counts:
                try:
                    self.read(key.data, cleanup=True)
                except (OSError, ValueError):
                    try:
                        self.selector.unregister(key.fileobj)
                    except (KeyError, ValueError):
                        pass

    def close(self):
        self.selector.close()
        for name in ("stdin", "stdout", "stderr"):
            try:
                getattr(self.process, name).close()
            except OSError:
                pass


def stop_owned_process(process, pipes, deadline):
    """Only this Popen child; never signal a name, group, other PID or service."""
    if process.poll() is not None:
        return "not_needed"
    for operation, allowance, label in (
        (process.terminate, 2.0, "terminated"),
        (process.kill, 1.0, "killed"),
    ):
        try:
            operation()
        except ProcessLookupError:
            pass
        except OSError:
            return "failed"
        until = min(deadline, time.monotonic() + allowance)
        while process.poll() is None and time.monotonic() < until:
            if pipes is not None:
                pipes.cleanup_drain(min(0.05, until - time.monotonic()))
            else:
                time.sleep(min(0.01, max(0, until - time.monotonic())))
        if process.poll() is not None:
            return label
    return "failed"


def check_latch(root):
    directory = root.lstat()
    require(
        stat.S_ISDIR(directory.st_mode)
        and stat.S_IMODE(directory.st_mode) == 0o700
        and directory.st_uid == os.getuid(),
        "invalid_state_directory",
    )
    marker = (root / "native-privacy-stop").lstat()
    require(
        stat.S_ISREG(marker.st_mode)
        and stat.S_IMODE(marker.st_mode) == 0o600
        and marker.st_uid == os.getuid()
        and marker.st_nlink == 1
        and marker.st_size == 0,
        "invalid_privacy_latch",
    )
    with os.scandir(root) as entries:
        for index, entry in enumerate(entries):
            require(
                index == 0 and entry.name == "native-privacy-stop",
                "unexpected_fixture_files",
            )


def profile_snapshot(root):
    """Metadata only: detect creations without reading profile file contents."""
    try:
        root_status = root.lstat()
    except FileNotFoundError:
        return None
    result = [
        (
            ".",
            root_status.st_mode,
            root_status.st_ino,
            root_status.st_size,
            root_status.st_mtime_ns,
            root_status.st_ctime_ns,
        )
    ]
    if stat.S_ISDIR(root_status.st_mode):
        with os.scandir(root) as entries:
            for entry in entries:
                status = entry.stat(follow_symlinks=False)
                result.append(
                    (
                        entry.name,
                        status.st_mode,
                        status.st_ino,
                        status.st_size,
                        status.st_mtime_ns,
                        status.st_ctime_ns,
                    )
                )
    return tuple(sorted(result))


def test_a3_fixture_does_not_change_real_profile(root, before):
    require(profile_snapshot(root) == before, "real_profile_changed_by_fixture")


def run_smoke(binary, *, timeout=30.0):
    """Tests may replace Popen with a labelled synthetic peer; that is not macOS evidence."""
    require(
        type(timeout) in (int, float) and math.isfinite(timeout) and 5 <= timeout <= 30,
        "invalid_timeout",
    )
    started = time.monotonic()
    deadline = started + timeout
    work_deadline = deadline - CLEANUP_SECONDS
    process = pipes = root = None
    real_profile = Path.home() / "Library" / "Application Support" / "Orbit"
    real_profile_before = profile_snapshot(real_profile)
    exchange = Exchange(started)
    decoder = Lines()
    error = None
    cleanup = "not_needed"
    removed = False
    try:
        selected = Path(binary)
        require(selected.is_absolute(), "binary_must_be_absolute")
        executable = selected.resolve(strict=True)
        require(
            executable.is_file() and os.access(executable, os.X_OK), "invalid_binary"
        )
        root = Path(tempfile.mkdtemp(prefix="orbit-native-smoke-"))
        # macOS commonly aliases /var to /private/var. Pass the actual freshly
        # created directory, not an alias that the native isolation gate rejects.
        root = root.resolve(strict=True)
        require(stat.S_IMODE(root.lstat().st_mode) == 0o700, "invalid_state_directory")
        # No env argument: in particular, do not override HOME or Keychain paths.
        process = subprocess.Popen(
            [str(executable), "--stdio", "--test-state-directory", str(root)],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            bufsize=0,
            close_fds=True,
        )
        pipes = Pipes(process)
        while True:
            now = time.monotonic()
            require(now < work_deadline, "smoke_timeout")
            exchange.heartbeat(now)
            pipes.wants_write(bool(exchange.outgoing))
            for key, _ in pipes.selector.select(min(0.1, work_deadline - now)):
                if key.data == "stdin":
                    try:
                        written = os.write(
                            process.stdin.fileno(), exchange.outgoing[:CHUNK]
                        )
                    except (BlockingIOError, InterruptedError):
                        continue
                    require(written > 0, "stdin_write_failed")
                    del exchange.outgoing[:written]
                    pipes.wants_write(bool(exchange.outgoing))
                else:
                    data = pipes.read(key.data)
                    if data is not None and key.data == "stdout":
                        if not data:
                            decoder.finish()
                        for frame in decoder.feed(data):
                            exchange.accept(frame, time.monotonic())
            if process.poll() is not None and pipes.eof == {"stdout", "stderr"}:
                require(process.returncode == 0, "native_exit_nonzero")
                exchange.finish()
                require(not process.stdin.closed, "stdin_closed_before_exit")
                check_latch(root)
                test_a3_fixture_does_not_change_real_profile(
                    real_profile, real_profile_before
                )
                break
    except SmokeError as failure:
        error = failure.code
    except KeyboardInterrupt:
        error = "smoke_interrupted"
    except (OSError, ValueError, OverflowError):
        error = "runner_io_failure"
    finally:
        if process is not None:
            cleanup = stop_owned_process(process, pipes, deadline)
            if cleanup != "not_needed":
                error = error or "forced_process_cleanup"
            if cleanup == "failed":
                error = "process_cleanup_failed"
            if pipes is not None:
                pipes.close()
            else:
                for handle in (process.stdin, process.stdout, process.stderr):
                    if handle is not None:
                        handle.close()
        if root is not None and (process is None or process.poll() is not None):
            try:
                shutil.rmtree(
                    root
                )  # Exact fresh directory owned by this invocation only.
                removed = True
            except OSError:
                error = error or "temporary_cleanup_failed"
    return {
        "ok": error is None,
        "error": error,
        "evidence": "isolated_native_protocol_smoke",
        "checked": exchange.checked,
        "heartbeat_responses": exchange.heartbeats,
        "exit_code": process.returncode if process is not None else None,
        "stdout_bytes": pipes.counts["stdout"] if pipes is not None else 0,
        "stderr_bytes": pipes.counts["stderr"] if pipes is not None else 0,
        "cleanup": cleanup,
        "temporary_state_removed": removed,
        "elapsed_ms": round((time.monotonic() - started) * 1000),
        "timeout_ms": round(timeout * 1000),
        "work_budget_ms": round((timeout - CLEANUP_SECONDS) * 1000),
        "stdin_held_open_until_exit": error is None,
        "home_overridden": False,
        "tcc_audio_or_production_validated": False,
    }


class Arguments(argparse.ArgumentParser):
    def error(self, _message):
        raise SmokeError("invalid_arguments")


def main(argv=None):
    parser = Arguments(description=__doc__)
    parser.add_argument("--binary", required=True, type=Path)
    parser.add_argument(
        "--timeout",
        type=float,
        default=30.0,
        help="presupuesto total de 5..30 s, incluidos 3 s reservados a cleanup",
    )
    try:
        arguments = parser.parse_args(argv)
        require(sys.platform == "darwin", "macos_required")
        result = run_smoke(arguments.binary, timeout=arguments.timeout)
    except SmokeError as failure:
        result = {"ok": False, "error": failure.code}
    print(json.dumps(result, ensure_ascii=False, sort_keys=True))
    return 0 if result["ok"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
