"""Linux kernel boundary for every official vendor process and its children.

Network stays shared. /run/systemd/resolve stays readable for Ubuntu DNS;
user-session and container control sockets are masked separately instead.
"""

import asyncio
import os
import shutil
import sys
import tomllib
from dataclasses import fields, is_dataclass
from pathlib import Path

from pydantic import BaseModel

from ..errors import HubError
from .jail_secret_cache import INITIAL_SCAN_WAIT_SECONDS, SecretScanUnavailable
from .jail_secrets import excluded
from .tool_settings import HOME_FOLDERS

OWNER_CREDENTIALS = (
    ".ssh",
    ".gnupg",
    ".claude",
    ".claude.json",
    ".codex",
    ".config/gh",
    ".config/gcloud",
    ".aws",
    ".azure",
    ".kube",
    ".docker",
    ".netrc",
    ".npmrc",
    ".pypirc",
    ".git-credentials",
    ".password-store",
    ".local/share/keyrings",
)
SECRET_FIELDS = {
    "key_file",
    "credential_file",
    "hermes_credential_file",
    "api_key_file",
    "orbit_mcp_config",
    "config_file",
    "tls_key_file",
}
# A local mount probe must not hold up assistant admission indefinitely.
PROBE_SECONDS = 10


def workspace_root(settings):
    return settings.data_dir.with_name(settings.data_dir.name + "-workspaces")


def escape_paths():
    return tuple(
        map(
            Path,
            (
                f"/run/user/{os.getuid()}",
                "/run/docker.sock",
                "/run/docker",
                "/run/containerd",
                "/var/snap/lxd/common/lxd",
                "/var/lib/lxd",
            ),
        )
    )


def secret_paths(value, names=SECRET_FIELDS):
    """Enumerate configured secret locations, without opening any credential."""
    if is_dataclass(value):
        value = {field.name: getattr(value, field.name) for field in fields(value)}
    elif isinstance(value, BaseModel):
        value = value.model_dump()
    if isinstance(value, dict):
        for name, entry in value.items():
            if name in names and entry is not None:
                yield Path(entry).expanduser().resolve()
            else:
                yield from secret_paths(entry, names)
    elif isinstance(value, (tuple, list)):
        for entry in value:
            yield from secret_paths(entry, names)


def hidden_paths(settings, agent_id, configs=None):
    paths = [settings.data_dir.resolve(), *secret_paths(settings)]
    root = workspace_root(settings)
    if root.exists():
        paths.extend(sorted(p.resolve() for p in root.iterdir() if p.name != agent_id))
    if configs is not None:
        paths.extend(Path(p).resolve() for aid, p in configs.working_folders() if aid != agent_id)
    # Translator TOMLs can point at keys outside data_dir. Read only their
    # paths, never the credential payload, including operator overrides.
    for config in tuple(dict.fromkeys(secret_paths(settings, {"orbit_mcp_config"}))):
        if config.is_file():
            try:
                with config.open("rb") as handle:
                    paths.extend(secret_paths(tomllib.load(handle)))
            except (OSError, ValueError):
                raise unavailable("invalid_secret_configuration") from None
    paths.extend(p.resolve() for p in escape_paths())
    paths.extend((Path.home() / p).resolve() for p in OWNER_CREDENTIALS)
    return tuple(dict.fromkeys(paths))


def installed_paths():
    # Release source and interpreter environment must never become a workspace:
    # changing either would turn the next unsandboxed hub startup into execution.
    return (Path(__file__).resolve().parents[2], Path(sys.prefix).resolve())


def arguments(settings, agent, working_folder, configs=None, *, vendor_home=None):
    """One readable argv prefix, also used by the real-kernel acceptance test."""
    folder = settings.data_dir.resolve() / "agents" / agent.id
    vendor = vendor_home or folder / HOME_FOLDERS[agent.kind]
    bridge = folder / "bridge"
    work = Path(working_folder).resolve()
    args = [
        "bwrap",
        "--ro-bind",
        "/",
        "/",
        "--dev",
        "/dev",
        "--proc",
        "/proc",
        "--tmpfs",
        "/tmp",
        "--unshare-pid",
        "--die-with-parent",
        "--new-session",
        "--setenv",
        "TMPDIR",
        "/tmp",
    ]
    masked = []
    for path in (*hidden_paths(settings, agent.id, configs), workspace_root(settings).resolve()):
        if any(parent in path.parents for parent in masked):
            continue
        if not path.exists():
            continue
        if path.is_dir():
            masked.append(path)
            args.extend(("--tmpfs", str(path)))
            # Create mount points before sealing the empty data tree. Sources
            # of subsequent binds are opened from bwrap's original root.
            if path == settings.data_dir.resolve():
                args.extend(("--dir", str(vendor), "--dir", str(bridge)))
            if path in work.parents:
                args.extend(("--dir", str(work)))
            args.extend(("--remount-ro", str(path)))
        else:
            args.extend(("--ro-bind", "/dev/null", str(path)))
    if bridge.is_dir():
        args.extend(("--ro-bind", str(bridge), str(bridge)))
    args.extend(("--bind", str(work), str(work), "--bind", str(vendor), str(vendor)))
    try:
        secrets = settings.assistant_jail_cache.paths()
    except SecretScanUnavailable as error:
        raise unavailable(str(error)) from None
    resolved_vendor = vendor.resolve()
    for path in secrets:
        if (
            not excluded(path, work, resolved_vendor, masked)
            and path.is_file()
            and os.access(path, os.R_OK)
        ):
            args.extend(("--ro-bind", "/dev/null", str(path)))
    args.extend(("--chdir", str(work)))
    return args


def unavailable(cause, status=None):
    message = (
        "No se puede aislar al asistente: falta bubblewrap; instala bubblewrap en el hub"
        if cause == "missing"
        else "No se puede crear la jaula de bubblewrap; revisa los espacios de nombres de usuario "
        "y el perfil de AppArmor de Ubuntu (bwrap-userns-restrict) para permitir bubblewrap"
    )
    numbers = {"cause": cause}
    if cause.startswith("secret_scan_"):
        reasons = {
            "secret_scan_timeout": (
                f"la revisión de secretos del hub no terminó en {INITIAL_SCAN_WAIT_SECONDS} segundos"
            ),
            "secret_scan_failed": "no se pudo completar la revisión de secretos del hub",
            "secret_scan_not_ready": "la revisión de secretos del hub aún no ha terminado",
        }
        message = (
            f"No se puede aislar al asistente todavía: {reasons[cause]}. "
            "Vuelve a intentarlo en unos instantes"
        )
        numbers["wait_limit_seconds"] = INITIAL_SCAN_WAIT_SECONDS
    if status is not None:
        numbers["exit_status"] = status
    # Existing Macs already render this stable adapter refusal and its message.
    return HubError("agent_adapter_missing", message, numbers=numbers)


async def launch(settings, agent, working_folder, configs=None, *, vendor_home=None):
    try:
        await settings.assistant_jail_cache.wait_ready()
    except SecretScanUnavailable as error:
        raise unavailable(str(error)) from None
    args = await asyncio.to_thread(
        arguments, settings, agent, working_folder, configs, vendor_home=vendor_home
    )
    binary = shutil.which("bwrap")
    if binary is None:
        raise unavailable("missing")
    from .tool_availability import validate_linux_executable

    command = getattr(getattr(agent, "settings", None), "command", None)
    if command:
        await asyncio.to_thread(validate_linux_executable, agent.kind, command[0])
    args[0] = binary
    try:
        probe = await asyncio.create_subprocess_exec(
            *args,
            "--",
            "/bin/true",
            stdout=asyncio.subprocess.DEVNULL,
            stderr=asyncio.subprocess.DEVNULL,
        )
        try:
            status = await asyncio.wait_for(probe.wait(), PROBE_SECONDS)
        except TimeoutError:
            probe.kill()
            await probe.wait()
            raise unavailable("probe_timeout") from None
    except OSError:
        raise unavailable("launch_failed") from None
    if status:
        raise unavailable("namespace_or_mount_denied", status)
    return [*args, "--"]
