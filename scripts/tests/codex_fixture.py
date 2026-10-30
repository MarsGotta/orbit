"""Local official-asset stand-in; release tests must never download binaries."""

import hashlib
import io
import tarfile
from pathlib import Path


def write_codex_fixture(root, server):
    archive = root / "codex-fixture.tar.gz"
    content = b'#!/bin/sh\n[ "$1" = "--version" ] || exit 91\nprintf "codex-cli 0.164.0\\n"\n'
    # The complete package member snapshot is independent of the installer.
    listing = Path(__file__).with_name("codex-package-members.txt")
    members = listing.read_text().splitlines()
    with tarfile.open(archive, "w:gz") as tar:
        for member in members:
            data = content if member == "bin/codex" else b"invented package member\n"
            entry = tarfile.TarInfo(member)
            entry.size, entry.mode = (
                len(data),
                0o755
                if "/bin/" in member
                or member.startswith("bin/")
                or member.endswith(("/rg", "/bwrap"))
                else 0o644,
            )
            tar.addfile(entry, io.BytesIO(data))
    checksum = hashlib.sha256(archive.read_bytes()).hexdigest()
    (server / "codex-release.toml").write_text(
        'version = "0.164.0"\n'
        'target = "x86_64-unknown-linux-musl"\n'
        f'url = "{archive.as_uri()}"\n'
        f'sha256 = "{checksum}"\n'
        f"members = {members!r}\n",
        encoding="utf-8",
    )
    return checksum
