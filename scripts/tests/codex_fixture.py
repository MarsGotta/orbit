"""Local official-asset stand-in; release tests must never download binaries."""

import hashlib
import io
import tarfile


def write_codex_fixture(root, server):
    archive = root / "codex-fixture.tar.gz"
    content = b'#!/bin/sh\n[ "$1" = "--version" ] || exit 91\nprintf "codex-cli 0.164.0\\n"\n'
    member = "codex-x86_64-unknown-linux-musl"
    with tarfile.open(archive, "w:gz") as tar:
        entry = tarfile.TarInfo(member)
        entry.size, entry.mode = len(content), 0o755
        tar.addfile(entry, io.BytesIO(content))
    checksum = hashlib.sha256(archive.read_bytes()).hexdigest()
    (server / "codex-release.toml").write_text(
        'version = "0.164.0"\n'
        'target = "x86_64-unknown-linux-musl"\n'
        f'url = "{archive.as_uri()}"\n'
        f'sha256 = "{checksum}"\n'
        f'member = "{member}"\n', encoding="utf-8",
    )
    return checksum
