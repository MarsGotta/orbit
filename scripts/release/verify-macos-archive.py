#!/usr/bin/env python3
"""Inspect an Orbit macOS ZIP using only bounded, read-only standard-library I/O.

No member is extracted or executed. A checksum is not proof of authenticity:
obtain the ZIP, checksum and expected commit from a trusted build channel.
This verifies neither code signatures nor notarization; codesign runs in macOS CI.

The expected version comes from the workspace manifest, so the archive name is
Orbit-<version>-macos-<arch>.zip for the version this repository declares.

Usage: python3 scripts/release/verify-macos-archive.py Orbit-<version>-macos-arm64.zip \
    --architecture arm64 --commit <40 lowercase hex digits> \
    --checksum-file Orbit-<version>-macos-arm64.zip.sha256 [--manifest build-manifest.txt]
"""

import argparse
import hashlib
import io
import json
import os
import plistlib
import re
import stat
import struct
import sys
import unicodedata
import zipfile
import zlib
from pathlib import Path
from xml.parsers.expat import ExpatError

# The five ceilings of an archive this script may read. The archive comes from
# the build Mac or the download origin and is not trusted until it passes, so
# each one bounds what a malformed or hostile ZIP can cost this process; none
# is a size budget for Orbit. Measured on the installed 0.5.1 (arm64, the
# reviewer, when the daemon was still a binary of its own): MacOS/cua-driver
# 31,918,688 B, Resources/orbit 15,529,088 B, MacOS/Orbit 13,257,488 B,
# MacOS/OrbitNative 4,025,744 B. Now the daemon is inside MacOS/Orbit: at most the sum of the two, about
# 28.8 MB (a Linux release build of the merged app measured 31,234,416 B, and
# Mach-O is smaller than that build's ELF). On Linux those binaries deflate
# to 39-43 % of their size.
#
# The whole archive is read into memory once (SHA-256, then the ZIP from that
# buffer). 64 MiB bounds that memory; at the Linux ratio the about 62 MiB of
# the 0.5.x members deflate to about 26 MB (not measured on a real archive).
MAX_ZIP_BYTES = 64 * 1024 * 1024
# Every member is inflated, streaming, to check its size and CRC: this bounds
# the work of a ZIP bomb across the archive. The 0.5.x tree is about 62 MiB
# uncompressed, about half of it.
MAX_TOTAL_BYTES = 128 * 1024 * 1024
# The same per member: a member larger than anything the bundle ships is not
# the bundle. The largest is the third-party driver at 31,918,688 B, 1.56 MiB
# under this; a driver update that outgrows it is refused with both numbers,
# and only then does this move, because only then is a larger member ours.
MAX_MEMBER_BYTES = 32 * 1024 * 1024
# Central-directory entries parsed before zipfile allocates one object each.
# The nested engine app and the third-party notice bring the synthetic tree to
# 24 members (25 with a stapled ticket); 128 leaves room for explicit
# directories an archiver adds.
MAX_MEMBERS = 128
# Info.plist, the build manifest and the driver's licence are kept in memory
# and parsed; they measure under 4 KiB, and 64 KiB is sixteen times that.
MAX_METADATA_BYTES = 64 * 1024


def _expected_version() -> str:
    """The version this build is supposed to carry, read from the repository.

    Deliberately not taken from the archive under inspection: a mislabelled
    package would then certify itself. The workspace manifest is the same source
    the packaging script uses, so the two cannot drift apart.
    """
    manifest = Path(__file__).resolve().parents[2] / "Cargo.toml"
    for line in manifest.read_text(encoding="utf-8").splitlines():
        if line.startswith("version = "):
            return line.split('"')[1]
    raise SystemExit("Cargo.toml declares no readable version")


VERSION = _expected_version()
ROOT = "Orbit.app"
CONTENTS = ROOT + "/Contents"
RESOURCES = CONTENTS + "/Resources"
PLIST = CONTENTS + "/Info.plist"
MANIFEST = RESOURCES + "/build-manifest.txt"
CUA_ENGINE_RELATIVE_PATH = "Helpers/Orbit Engine.app/Contents/MacOS/cua-driver"
DRIVER = CONTENTS + "/" + CUA_ENGINE_RELATIVE_PATH
ENGINE_CONTENTS = CONTENTS + "/Helpers/Orbit Engine.app/Contents"
ENGINE_PLIST = ENGINE_CONTENTS + "/Info.plist"
DRIVER_LICENSE = RESOURCES + "/third-party/cua-driver/LICENSE.md"
DRIVER_LICENSE_BYTES = (
    Path(__file__).resolve().parents[2]
    / "third-party"
    / "cua-driver"
    / "LICENSE.md"
).read_bytes()
# The notice of the interface's packages and the app's crates, written at
# build time from what the build installed (scripts/release/third-party-notices.py).
# It is generated, so only its first line is known here: that line, at the
# start of the member, says it is that notice and not a stray file.
NOTICES = RESOURCES + "/third-party/NOTICES.txt"
NOTICES_HEADER = b"Third-party software in Orbit\n"
# The daemon runs inside MacOS/Orbit; a Contents/Resources/orbit is an
# unexpected file of an archive the old script built.
MACHOS = {
    CONTENTS + "/MacOS/Orbit",
    CONTENTS + "/MacOS/OrbitNative",
    DRIVER,
}
EXECUTABLES = MACHOS
REQUIRED_FILES = EXECUTABLES | {
    PLIST,
    ENGINE_PLIST,
    MANIFEST,
    RESOURCES + "/LICENSE",
    DRIVER_LICENSE,
    NOTICES,
    RESOURCES + "/icon.icns",
}
# Where `stapler staple` leaves the notarization ticket for a bundle. Distinct
# from _CodeSignature/CodeResources, which is codesign's resource seal: same
# file name, different directory, entirely different meaning.
STAPLED_TICKET = CONTENTS + "/CodeResources"
OPTIONAL_FILES = {
    CONTENTS + "/_CodeSignature/CodeResources",
    ENGINE_CONTENTS + "/_CodeSignature/CodeResources",
    STAPLED_TICKET,
}
# First four bytes of a stapled ticket, so that any file parked at that path
# cannot pass for one. This identifies the format; it does not validate the
# ticket, which needs Apple and is not something a byte reader can do.
TICKET_MAGIC = b"s8ch"
DIRECTORIES = {
    ROOT,
    CONTENTS,
    CONTENTS + "/MacOS",
    CONTENTS + "/Helpers",
    CONTENTS + "/Helpers/Orbit Engine.app",
    ENGINE_CONTENTS,
    ENGINE_CONTENTS + "/MacOS",
    ENGINE_CONTENTS + "/_CodeSignature",
    RESOURCES,
    RESOURCES + "/third-party",
    RESOURCES + "/third-party/cua-driver",
    CONTENTS + "/_CodeSignature",
}
CPU_TYPES = {"arm64": 0x0100000C, "x86_64": 0x01000007}


class VerificationError(ValueError):
    """A bounded input failed the published development-bundle contract."""


class UniquePlistKeys(dict):
    """Do not let a second plist key silently replace security metadata."""

    def __setitem__(self, key, value):
        if key in self:
            raise VerificationError("duplicate key in Info.plist")
        super().__setitem__(key, value)


def require(condition, message):
    if not condition:
        raise VerificationError(message)


def read_limited(path, limit):
    """Read regular files only; callers must choose trusted parent directories.

    On POSIX, atomically refuse a final symlink and avoid blocking on a FIFO
    swapped into the pathname. Other platforms still check fstat before reading.
    """
    flags = os.O_RDONLY
    for option in ("O_CLOEXEC", "O_NONBLOCK", "O_NOFOLLOW", "O_BINARY"):
        flags |= getattr(os, option, 0)
    with os.fdopen(os.open(path, flags), "rb") as source:
        metadata = os.fstat(source.fileno())
        require(
            stat.S_ISREG(metadata.st_mode), "the input must be a regular file"
        )
        require(metadata.st_size <= limit, "input larger than the allowed limit")
        data = source.read(limit + 1)
    require(len(data) <= limit, "input larger than the allowed limit")
    return data


def check_directory_budget(data):
    """Bound central-directory parsing BEFORE zipfile allocates a ZipInfo per entry.

    ZIP64 end records and multi-disk archives are unnecessary for these bundles. The
    strict layout also rejects appended payloads and ambiguous directory trailers.
    """
    end = data.rfind(b"PK\x05\x06", max(0, len(data) - 65557))
    require(end >= 0 and end + 22 <= len(data), "ZIP end missing or truncated")
    disk, central_disk, disk_count, count, size, offset, comment = struct.unpack_from(
        "<HHHHIIH", data, end + 4
    )
    require(
        disk == central_disk == 0 and disk_count == count, "multi-disk ZIP not supported"
    )
    require(
        0 < count <= MAX_MEMBERS,
        f"too many ZIP members or an empty archive: {count}, ceiling {MAX_MEMBERS}",
    )
    require(
        offset + size == end and end + 22 + comment == len(data),
        "ambiguous ZIP directory",
    )
    require(data[:4] == b"PK\x03\x04", "the ZIP does not start with a member")
    cursor, observed = offset, 0
    while cursor < end:
        require(
            cursor + 46 <= end and data[cursor : cursor + 4] == b"PK\x01\x02",
            "invalid ZIP central header",
        )
        name_size, extra_size, comment_size = struct.unpack_from(
            "<HHH", data, cursor + 28
        )
        cursor += 46 + name_size + extra_size + comment_size
        observed += 1
        require(
            cursor <= end and observed <= MAX_MEMBERS,
            "ZIP directory limit exceeded",
        )
    require(observed == count, "the ZIP count does not match its directory")
    return count, offset


def normalized_path(name):
    return unicodedata.normalize("NFC", name).casefold()


def check_members(members, expected_count):
    """Permit only the generated bundle tree; directory records are optional.

    .DS_Store, AppleDouble and other extras are rejected, not silently ignored.
    _CodeSignature/CodeResources may be present, but is not a verified signature.
    """
    require(len(members) == expected_count, "inconsistent ZIP central count")
    paths, files, total = {}, set(), 0
    for member in members:
        name = member.filename
        require(
            name == member.orig_filename and "\x00" not in name, "ZIP name with NUL"
        )
        require(
            "\\" not in name and not name.startswith("/"),
            "absolute ZIP path or one with a backslash",
        )
        require(len(name.encode("utf-8")) <= 1024, "ZIP name too long")
        require(
            all(ord(char) >= 32 and ord(char) != 127 for char in name),
            "ZIP name with control characters",
        )
        path = name[:-1] if member.is_dir() else name
        require(
            all(part not in ("", ".", "..") for part in path.split("/")),
            "traversal or non-canonical ZIP path",
        )
        folded = normalized_path(path)
        require(folded not in paths, "duplicate ZIP path or NFC/casefold collision")
        paths[folded] = member.is_dir()
        mode = member.external_attr >> 16
        require(member.create_system == 3, "Unix modes missing from the ZIP")
        kind = stat.S_IFMT(mode)
        require(kind != stat.S_IFLNK, "ZIP symbolic link not supported")
        require(
            kind == (stat.S_IFDIR if member.is_dir() else stat.S_IFREG),
            "ZIP member type not supported",
        )
        require(
            mode & 0o7022 == 0, "special ZIP permissions or writable by others"
        )
        require(not member.flag_bits & 1, "encrypted ZIP not supported")
        require(not member.flag_bits & ~0x080E, "ZIP flags not supported")
        require(
            member.compress_type in (zipfile.ZIP_STORED, zipfile.ZIP_DEFLATED),
            "ZIP compression not supported",
        )
        require(member.volume == 0, "multi-disk ZIP member")
        require(
            0 <= member.file_size <= MAX_MEMBER_BYTES,
            f"ZIP member {name} of {member.file_size} B, larger than the ceiling of "
            f"{MAX_MEMBER_BYTES} B",
        )
        require(
            0 <= member.compress_size <= MAX_ZIP_BYTES, "invalid ZIP compressed size"
        )
        if member.compress_type == zipfile.ZIP_STORED:
            require(
                member.compress_size == member.file_size, "ZIP_STORED sizes differ"
            )
        total += member.file_size
        require(
            total <= MAX_TOTAL_BYTES,
            f"ZIP content of {total} B, larger than the ceiling of {MAX_TOTAL_BYTES} B",
        )
        if member.is_dir():
            require(member.file_size == 0, "ZIP directory with content")
        else:
            files.add(path)
        if path in EXECUTABLES:
            require(
                mode & 0o555 == 0o555, "executable without read/execute permissions"
            )
        if path in (PLIST, ENGINE_PLIST, MANIFEST, DRIVER_LICENSE):
            require(
                member.file_size <= MAX_METADATA_BYTES,
                "bundle metadata too large",
            )
    for path in paths:
        parts = path.split("/")
        require(
            all(
                paths.get("/".join(parts[:index]), True)
                for index in range(1, len(parts))
            ),
            "conflict between a ZIP file and directory",
        )
    require(REQUIRED_FILES <= files, "required bundle files are missing")
    require(
        files <= REQUIRED_FILES | OPTIONAL_FILES,
        "unexpected file: only the generated tree is accepted, without .DS_Store/AppleDouble",
    )
    require(
        all(
            member.filename.rstrip("/") in DIRECTORIES
            for member in members
            if member.is_dir()
        ),
        "unexpected bundle directory",
    )
    return total


def local_payload(data, member, boundary):
    """Require one unambiguous local header/data/descriptor interval per member."""
    offset = member.header_offset
    require(
        0 <= offset and offset + 30 <= boundary,
        "overlapping or truncated ZIP local header",
    )
    (
        signature,
        version,
        flags,
        method,
        _,
        _,
        crc,
        packed,
        unpacked,
        name_size,
        extra_size,
    ) = struct.unpack_from("<4s5H3I2H", data, offset)
    require(signature == b"PK\x03\x04", "invalid ZIP local header signature")
    require(
        (version, flags, method)
        == (member.extract_version, member.flag_bits, member.compress_type),
        "cabeceras local/central ZIP distintas",
    )
    start = offset + 30 + name_size + extra_size
    end = start + member.compress_size
    require(start <= end <= boundary, "overlapping or out-of-bounds ZIP data")
    encoding = "utf-8" if flags & 0x0800 else "cp437"
    require(
        data[offset + 30 : offset + 30 + name_size] == member.filename.encode(encoding),
        "nombre local/central ZIP distinto",
    )
    expected = (member.CRC, member.compress_size, member.file_size)
    if flags & 8:
        require(
            (crc, packed, unpacked) in ((0, 0, 0), expected),
            "inconsistent ZIP local sizes",
        )
        require(boundary - end in (12, 16), "invalid ZIP descriptor length")
        descriptor = data[end:boundary]
        if len(descriptor) == 16 and descriptor[:4] == b"PK\x07\x08":
            descriptor = descriptor[4:]
        require(
            len(descriptor) == 12 and struct.unpack("<3I", descriptor) == expected,
            "ZIP descriptor missing or different from the central header",
        )
    else:
        require(
            (crc, packed, unpacked) == expected,
            "ZIP local/central sizes or CRC differ",
        )
        require(end == boundary, "unaccounted bytes between ZIP members")
    return memoryview(data)[start:end]


def uncompressed_blocks(payload, method):
    """Do not use ZipExtFile: it silently clips output to a forged file_size.

    Raw DEFLATE must finish exactly at the payload boundary. Each decode allocates
    at most 64 KiB; its consumer checks actual output length and CRC, not a prefix.
    """
    decoder = zlib.decompressobj(-15) if method == zipfile.ZIP_DEFLATED else None
    for offset in range(0, len(payload), 64 * 1024):
        pending = payload[offset : offset + 64 * 1024]
        if decoder is None:
            yield pending
            continue
        while pending:
            require(not decoder.eof, "bytes after the DEFLATE end")
            block = decoder.decompress(pending, 64 * 1024)
            pending = decoder.unconsumed_tail
            require(not decoder.unused_data, "bytes left over after the DEFLATE stream")
            yield block
    if decoder is not None:
        # Drain bounded internal output without flush(), whose length is only an
        # initial buffer hint and cannot impose a hard expansion bound.
        while not decoder.eof:
            block = decoder.decompress(b"", 64 * 1024)
            if not block:
                break
            yield block
        require(
            decoder.eof and not decoder.unused_data and not decoder.unconsumed_tail,
            "truncated or incomplete DEFLATE stream",
        )


def read_members(data, members, central_offset):
    """Validate complete compressed streams/CRCs; retain only metadata and headers."""
    retained = {}
    ordered = sorted(members, key=lambda member: member.header_offset)
    require(
        ordered[0].header_offset == 0,
        "unaccounted prefix before the first ZIP member",
    )
    for index, member in enumerate(ordered):
        boundary = (
            ordered[index + 1].header_offset
            if index + 1 < len(ordered)
            else central_offset
        )
        require(
            boundary <= central_offset, "ZIP local header outside the members area"
        )
        payload = local_payload(data, member, boundary)
        count, checksum = 0, 0
        limit = (
            MAX_METADATA_BYTES
            if member.filename in (PLIST, ENGINE_PLIST, MANIFEST, DRIVER_LICENSE)
            else 32
        )
        prefix = bytearray()
        for block in uncompressed_blocks(payload, member.compress_type):
            count += len(block)
            require(
                count <= member.file_size and count <= MAX_MEMBER_BYTES,
                "ZIP expansion out of its bounds",
            )
            checksum = zlib.crc32(block, checksum)
            if len(prefix) < limit:
                prefix.extend(block[: limit - len(prefix)])
        require(
            count == member.file_size and checksum == member.CRC,
            "wrong ZIP size or CRC",
        )
        retained[member.filename] = bytes(prefix)
    return retained


def check_macho(header, size, architecture):
    require(
        len(header) == 32 and header[:4] == b"\xcf\xfa\xed\xfe",
        "a thin 64-bit little-endian Mach-O is required",
    )
    _, cpu, _, file_type, commands, command_bytes, _, _ = struct.unpack("<8I", header)
    require(
        cpu == CPU_TYPES[architecture],
        "Mach-O CPU differs from the requested architecture",
    )
    require(file_type == 2, "Mach-O is not an executable")
    require(
        command_bytes <= size - 32 and commands <= command_bytes // 8,
        "truncated or inconsistent Mach-O header",
    )


def macos_version(value):
    require(
        isinstance(value, str)
        and re.fullmatch(r"[0-9]{1,3}\.[0-9]{1,3}(?:\.[0-9]{1,3})?", value),
        "invalid minimum macOS version",
    )
    parts = tuple(int(part) for part in value.split("."))
    return parts + (0,) * (3 - len(parts))


def check_metadata(plist_bytes, manifest_bytes, architecture, commit):
    require(b"<!ENTITY" not in plist_bytes, "XML entities not supported")
    info = plistlib.loads(plist_bytes, dict_type=UniquePlistKeys)
    require(isinstance(info, dict), "Info.plist must be a dictionary")
    for field, value in {
        "CFBundleIdentifier": "app.orbit.companion",
        "CFBundleShortVersionString": VERSION,
        "CFBundleExecutable": "Orbit",
        "CFBundlePackageType": "APPL",
    }.items():
        require(
            info.get(field) == value, "wrong Info.plist identity or version"
        )
    minimum = info.get("LSMinimumSystemVersion")
    require(
        macos_version(minimum) >= (15, 0, 0),
        "the bundle requires a minimum below macOS 15",
    )
    require(
        manifest_bytes.endswith(b"\n") and b"\r" not in manifest_bytes,
        "non-canonical manifest",
    )
    fields = {}
    for line in manifest_bytes.decode("ascii").splitlines():
        require(line.count("=") == 1, "invalid manifest line")
        key, value = line.split("=", 1)
        require(key not in fields, "duplicate manifest field")
        fields[key] = value
    # The claim is read, not dictated: pinning it to "false" made a notarized
    # package fail verification for being notarized. What must never be accepted
    # is a value that is neither, which would sail through a boolean test.
    notarized = fields.get("notarized")
    require(notarized in ("true", "false"), "the manifest does not declare notarization")
    expected = {
        "version": VERSION,
        "architecture": architecture,
        "source_commit": commit,
        "source_dirty": "false",
        "minimum_macos": minimum,
        "notarized": notarized,
    }
    require(
        fields == expected,
        "the manifest fields or values do not match the requested build",
    )
    return minimum, notarized == "true"


def check_engine_metadata(data):
    require(b"<!ENTITY" not in data, "XML entities not supported in the engine")
    info = plistlib.loads(data, dict_type=UniquePlistKeys)
    require(isinstance(info, dict), "the engine's Info.plist must be a dictionary")
    # The same version the bundler reads, without another hand-written copy.
    source = (
        Path(__file__).resolve().parents[2] / "crates/orbit-daemon/src/cua.rs"
    ).read_text()
    version = re.search(
        r'^pub const VERIFIED_DRIVER_VERSION: &str = "([0-9.]+)";', source, re.M
    )
    require(version is not None, "the engine's pinned version could not be read")
    for field, value in {
        "CFBundleIdentifier": "app.orbit.companion.cua-driver",
        "CFBundleName": "Orbit Engine",
        "CFBundleExecutable": "cua-driver",
        "CFBundlePackageType": "APPL",
        "CFBundleVersion": version[1],
        "CFBundleShortVersionString": version[1],
        "LSUIElement": True,
    }.items():
        require(
            info.get(field) == value and type(info.get(field)) is type(value),
            f"wrong engine Info.plist: {field}",
        )


def verify_archive(path, *, architecture, commit, checksum_file, manifest=None):
    """Return structural evidence for this byte snapshot, never a trust/signature verdict."""
    require(architecture in CPU_TYPES, "architecture not supported")
    require(
        re.fullmatch(r"[0-9a-f]{40}", commit) is not None,
        "commit must be a SHA of 40 lowercase hexadecimal characters",
    )
    path = Path(path)
    expected_name = f"Orbit-{VERSION}-macos-{architecture}.zip"
    require(
        path.name == expected_name, "ZIP name does not match version and architecture"
    )
    data = read_limited(path, MAX_ZIP_BYTES)
    digest = hashlib.sha256(data).hexdigest()
    checksum = read_limited(checksum_file, 4096)
    expected_checksum = f"{digest}  {expected_name}".encode("ascii")
    require(
        checksum in (expected_checksum, expected_checksum + b"\n"),
        "the checksum file SHA-256 or name does not match exactly",
    )
    count, central_offset = check_directory_budget(data)
    with zipfile.ZipFile(io.BytesIO(data), "r") as archive:
        members = archive.infolist()
        total = check_members(members, count)
        contents = read_members(data, members, central_offset)
    check_engine_metadata(contents[ENGINE_PLIST])
    sizes = {member.filename: member.file_size for member in members}
    for name in MACHOS:
        check_macho(contents[name], sizes[name], architecture)
    minimum, manifest_notarized = check_metadata(
        contents[PLIST], contents[MANIFEST], architecture, commit
    )
    require(
        contents[DRIVER_LICENSE] == DRIVER_LICENSE_BYTES,
        "cua-driver's licence does not match the versioned text",
    )
    require(
        contents[NOTICES].startswith(NOTICES_HEADER),
        "the third-party notice does not start as third-party-notices.py writes it",
    )
    if manifest is not None:
        require(
            read_limited(manifest, MAX_METADATA_BYTES) == contents[MANIFEST],
            "external manifest differs byte for byte",
        )
    # The one notarization fact bytes alone can establish: that the ticket was
    # stapled BEFORE the archive was built. A package zipped first stays
    # notarized on Apple's servers and unstapled on the disk of whoever
    # downloads it, which is exactly where it has to work without a network.
    stapled = STAPLED_TICKET in contents
    if stapled:
        require(
            contents[STAPLED_TICKET][: len(TICKET_MAGIC)] == TICKET_MAGIC,
            "the stapled file does not carry the mark of a notarization ticket",
        )
    require(
        stapled == manifest_notarized,
        "manifest and ticket contradict each other: notarization declared without "
        "a stapled ticket, or a ticket without declaring it",
    )
    return {
        "archive": expected_name,
        "bytes": len(data),
        "sha256": digest,
        "architecture": architecture,
        "source_commit": commit,
        "members": count,
        "uncompressed_bytes": total,
        "minimum_macos": minimum,
        # Two different facts, kept apart on purpose: what the build claims, and
        # what this verifier proved. It reads bytes without running anything, so
        # it can never prove either signature or notarization.
        "signature_verified": False,
        "notarization_verified": False,
        "manifest_notarized": manifest_notarized,
        # Present and correctly marked, which is not the same as valid: only
        # `stapler validate` on macOS can say that.
        "stapled_ticket": stapled,
        "trust_note": "The checksum does not authenticate the origin. Signature: verify with codesign on macOS; no content was run or extracted.",
    }


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("zip", type=Path)
    parser.add_argument("--architecture", required=True, choices=tuple(CPU_TYPES))
    parser.add_argument("--commit", required=True)
    parser.add_argument("--checksum-file", required=True, type=Path)
    parser.add_argument("--manifest", type=Path)
    args = parser.parse_args(argv)
    try:
        report = verify_archive(
            args.zip,
            architecture=args.architecture,
            commit=args.commit,
            checksum_file=args.checksum_file,
            manifest=args.manifest,
        )
    except (
        OSError,
        ValueError,
        zipfile.BadZipFile,
        ExpatError,
        EOFError,
        struct.error,
        RuntimeError,
        NotImplementedError,
        zlib.error,
        RecursionError,
    ) as error:
        print(f"Verification refused: {error}", file=sys.stderr)
        return 1
    print(json.dumps(report, ensure_ascii=False, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
