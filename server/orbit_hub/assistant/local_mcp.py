"""Derive added assistants' translator configuration from this hub's listener."""

import json
import os
import secrets
import tempfile
from dataclasses import replace
from pathlib import Path

from ..private_auth import BridgeConfiguration
from ..secret_file import read_secret
from ..urls import protected_url
from .sign_in import private_folder


def _bridge(folder: Path, name: str, configured):
    if configured is not None:
        return configured
    path = folder / f"{name}.key"
    try:
        descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    except FileExistsError:
        # Never regenerate an invalid existing key: that could silently revoke
        # a running translator. read_secret rejects links and unsafe permissions.
        if len(read_secret(path, 32)) != 32:
            raise ValueError(f"The generated {name} key must contain 32 bytes") from None
    else:
        with os.fdopen(descriptor, "wb") as handle:
            handle.write(secrets.token_bytes(32))
            handle.flush()
            os.fsync(handle.fileno())
    return BridgeConfiguration(key_id=f"orbit-assistant-{name}", credential_file=str(path))


def prepare(settings, hub_url: str):
    """Called before app composition, so both private bridges use these keys.

    The URL comes from the actual serve arguments, never from a Mac/Host header.
    Files live outside the immutable release; explicit MCP paths remain untouched.
    """
    protected_url(hub_url, allow_insecure_localhost=True)
    folder = settings.data_dir.resolve() / "assistant-tools"
    private_folder(folder)
    prepared = replace(
        settings,
        device_bridge=_bridge(folder, "device", settings.device_bridge),
        meetings_bridge=_bridge(folder, "meetings", settings.meetings_bridge),
    )
    lines = [f"hub_url = {json.dumps(hub_url)}", 'default_device = "Orbit"', "[devices]"]
    for name, bridge in (
        ("device_bridge", prepared.device_bridge),
        ("meetings", prepared.meetings_bridge),
    ):
        lines.extend(
            (
                f"[{name}]",
                f"key_id = {json.dumps(bridge.key_id)}",
                f"key_file = {json.dumps(bridge.credential_file)}",
            )
        )
    path = prepared.data_dir.resolve() / "assistant-mcp.toml"
    with tempfile.NamedTemporaryFile(mode="w", dir=folder, delete=False) as handle:
        temporary = Path(handle.name)
        try:
            handle.write("\n".join(lines) + "\n")
            handle.flush()
            os.fsync(handle.fileno())
            os.replace(temporary, path)
        finally:
            temporary.unlink(missing_ok=True)
    return prepared


def listener_url(settings, args) -> str:
    host = args.host
    if host in {"0.0.0.0", "::"}:
        # For TLS the explicit allowed host also names the certificate/origin.
        # Plain HTTP is only served on loopback; serve() enforces that rule.
        candidates = [
            h for h in settings.allowed_hosts if "*" not in h and h not in {"0.0.0.0", "::"}
        ]
        dns_hosts = [h for h in candidates if h not in {"localhost", "127.0.0.1", "::1"}]
        host = next(iter(dns_hosts or candidates), "localhost")
    if ":" in host:
        host = f"[{host}]"
    return f"{'https' if args.tls_cert else 'http'}://{host}:{args.port}"
