"""The hub's tests connect only to ports this test process bound itself (REF-13).

On the server where the gate runs, fixed ports of this machine are live
services: the real speech-to-text among them. A test that names one, or builds
one at run time, talks to that service. ``install`` wraps ``socket.socket`` so
that every IPv4/IPv6 connect is checked before it happens: the port must be one
this process bound (a synthetic peer from ``support_client_e2e.tcp_server`` or
``support_synthetic_stt.UnitStt``) and the host loopback. Anything else is
refused before a packet leaves, with the address in the error, and recorded, so
``conftest.py`` fails the test that tried even when the code under test swallows
the error (the transcription worker does, and retries).

What it does not cover: child processes (the Rust daemon, ``python -m``
launches), which get their addresses from the test; a transport
that never opens a socket (``httpx.MockTransport``, Starlette's ``TestClient``);
and Unix sockets, which are files the test created.
"""

import errno
import ipaddress
import socket
import threading

INET = (socket.AF_INET, socket.AF_INET6)


class Fence:
    """Ports bound by this process, and connections refused because of it."""

    def __init__(self):
        self._lock = threading.Lock()
        self._bound: set[int] = set()
        self._refused: list[str] = []

    def bound(self, port: int) -> None:
        with self._lock:
            self._bound.add(port)

    def allows(self, address) -> bool:
        host, port = address[0], address[1]
        with self._lock:
            owned = port in self._bound
        return owned and _loopback(host)

    def refuse(self, address) -> ConnectionRefusedError:
        described = f"{address[0]}:{address[1]}"
        with self._lock:
            self._refused.append(described)
        return ConnectionRefusedError(
            errno.ECONNREFUSED,
            f"REF-13: un test del hub sólo se conecta a puertos que abrió este proceso; "
            f"{described} no lo es (¿un servicio real de esta máquina?)",
        )

    def refusals(self) -> int:
        with self._lock:
            return len(self._refused)

    def refused_since(self, mark: int) -> list[str]:
        with self._lock:
            return self._refused[mark:]


def _loopback(host) -> bool:
    if host == "localhost":
        return True
    try:
        return ipaddress.ip_address(host).is_loopback
    except ValueError:
        return False


_INSTALLED: Fence | None = None


def install() -> Fence:
    """Wrap bind/connect once per process and return the fence that decides."""
    global _INSTALLED
    if _INSTALLED is not None:
        return _INSTALLED
    fence = Fence()
    real_bind = socket.socket.bind
    real_connect = socket.socket.connect
    real_connect_ex = socket.socket.connect_ex

    def bind(self, address):
        result = real_bind(self, address)
        if self.family in INET:
            fence.bound(self.getsockname()[1])
        return result

    def connect(self, address):
        if self.family in INET and not fence.allows(address):
            raise fence.refuse(address)
        return real_connect(self, address)

    def connect_ex(self, address):
        if self.family in INET and not fence.allows(address):
            return fence.refuse(address).errno
        return real_connect_ex(self, address)

    socket.socket.bind = bind
    socket.socket.connect = connect
    socket.socket.connect_ex = connect_ex
    _INSTALLED = fence
    return fence
