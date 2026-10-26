"""Install a release's pinned official Codex asset before sealing the release.

This runs only while publishing, never at hub startup or on a Mac request.
The manifest belongs to the same committed server tree as the protocol code.
"""

import hashlib
import platform
import shutil
import sys
import tarfile
import tempfile
import tomllib
import urllib.request
from pathlib import Path, PurePosixPath


def extract_package(tar, release, expected):
    """Validate the entire pinned inventory before writing any executable."""
    files = [member for member in tar.getmembers() if not member.isdir()]
    found = {member.name for member in files}
    missing, extra = set(expected) - found, found - set(expected)
    if missing or extra:
        raise ValueError(
            f"Codex package inventory mismatch: missing {sorted(missing)}, unexpected {sorted(extra)}"
        )
    if len(found) != len(files):
        raise ValueError("Codex package contains duplicate file members")
    for member in files:
        path = PurePosixPath(member.name)
        if not member.isfile() or path.is_absolute() or ".." in path.parts:
            raise ValueError(f"Codex package member {member.name} is not a safe regular file")
    for member in files:
        destination = release / member.name
        destination.parent.mkdir(parents=True, exist_ok=True)
        # No extractall, archive links, ownership or arbitrary mode bits.
        with tar.extractfile(member) as source, destination.open("xb") as target:
            shutil.copyfileobj(source, target)
        destination.chmod(0o755 if member.mode & 0o111 else 0o644)


def install(release: Path) -> dict:
    with (release / "codex-release.toml").open("rb") as handle:
        pin = tomllib.load(handle)
    system, machine = platform.system(), platform.machine()
    if (system, machine) != ("Linux", "x86_64") or pin["target"] != "x86_64-unknown-linux-musl":
        raise ValueError(
            f"Codex {pin['version']}: unsupported platform {system}/{machine}; "
            f"this release ships {pin['target']} (Linux/x86_64)"
        )
    # A streamed download bounds memory independently of the upstream asset's
    # size. Socket inactivity is capped at 30 s; publish also has a total timeout.
    with tempfile.TemporaryFile() as archive:
        digest = hashlib.sha256()
        with urllib.request.urlopen(pin["url"], timeout=30) as response:
            while chunk := response.read(1024 * 1024):
                archive.write(chunk)
                digest.update(chunk)
        actual = digest.hexdigest()
        if actual != pin["sha256"]:
            raise ValueError(
                f"Codex {pin['version']} SHA-256 mismatch for {pin['url']}: "
                f"expected {pin['sha256']}, received {actual}"
            )
        archive.seek(0)
        with tarfile.open(fileobj=archive, mode="r:gz") as tar:
            extract_package(tar, release, pin["members"])
    return pin


def main():
    try:
        install(Path(sys.argv[1]))
    except (OSError, ValueError, KeyError, tarfile.TarError) as exc:
        print(f"Cannot ship Codex: {exc}", file=sys.stderr)
        raise SystemExit(1) from None


if __name__ == "__main__":
    main()
