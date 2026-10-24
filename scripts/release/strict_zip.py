"""A ZIP read strictly, from bytes already in memory, for the archive verifier
(verify-macos-archive.py): the central directory bounded before zipfile parses
it, one unambiguous local header and payload per member, and DEFLATE inflated
in bounded blocks. Nothing is extracted, written or run.

The bundle's own rules (which members, which sizes, which metadata) stay in
the verifier; this module knows ZIP, not Orbit.
"""

import os
import stat
import struct
import zipfile
import zlib


class VerificationError(ValueError):
    """A bounded input failed the published development-bundle contract."""


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


def check_directory_budget(data, max_members):
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
        0 < count <= max_members,
        f"too many ZIP members or an empty archive: {count}, ceiling {max_members}",
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
            cursor <= end and observed <= max_members,
            "ZIP directory limit exceeded",
        )
    require(observed == count, "the ZIP count does not match its directory")
    return count, offset


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
