"""Synthetic ZIPs only; never downloads, extracts or executes a macOS binary.

Run: python3 -B -m unittest discover -s scripts/tests -v
"""

import hashlib
import importlib.util
import io
import os
import plistlib
import stat
import struct
import sys
import tempfile
import unittest
import warnings
import zipfile
import zlib
from contextlib import redirect_stderr, redirect_stdout
from pathlib import Path
from unittest import mock

sys.dont_write_bytecode = True
SPEC = importlib.util.spec_from_file_location(
    "verify_macos_archive",
    Path(__file__).resolve().parents[1] / "release" / "verify-macos-archive.py",
)
verify = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(verify)
COMMIT = "a" * 40
ENGINE_CONTENTS = verify.CONTENTS + "/Helpers/Orbit Engine.app/Contents"
ENGINE_PLIST = ENGINE_CONTENTS + "/Info.plist"
# The notice third-party-notices.py writes at build time, and its first line.
NOTICES = verify.RESOURCES + "/third-party/NOTICES.txt"
NOTICES_HEADER = b"Third-party software in Orbit\n"


def engine_plist():
    from driver_fixture import verified_driver_version

    return plistlib.dumps(
        {
            "CFBundleIdentifier": "app.orbit.companion.cua-driver",
            "CFBundleName": "Orbit Engine",
            "CFBundleExecutable": "cua-driver",
            "CFBundlePackageType": "APPL",
            "LSUIElement": True,
            "CFBundleShortVersionString": verified_driver_version(),
            "CFBundleVersion": verified_driver_version(),
        }
    )


def manifest(architecture="arm64", minimum="15.0", notarized="false"):
    return (
        f"version={verify.VERSION}\narchitecture={architecture}\nsource_commit={COMMIT}\n"
        f"source_dirty=false\nminimum_macos={minimum}\nnotarized={notarized}\n"
    ).encode()


# A ticket's leading marker followed by inert filler; never a real ticket, which
# would need Apple and cannot be checked here anyway.
TICKET = verify.TICKET_MAGIC + b"\x01\x00\x00\x00synthetic fixture only\n"


def plist(minimum="15.0"):
    return plistlib.dumps(
        {
            "CFBundleIdentifier": "app.orbit.companion",
            "CFBundleShortVersionString": verify.VERSION,
            "CFBundleExecutable": "Orbit",
            "CFBundlePackageType": "APPL",
            "LSMinimumSystemVersion": minimum,
        }
    )


def entries(architecture="arm64", *, stapled=False):
    """Entries matching the generated tree; Mach-O headers are inert bytes.

    The stapled notarization ticket is excluded unless asked for, even though it
    is an optional member. It was added to OPTIONAL_FILES when packages started
    being notarized, and including it here by default would have every fixture
    carry a ticket while its manifest says `notarized=false` — a contradiction
    the verifier now refuses, and rightly. `stapled=True` moves both together,
    because in a real build they are one fact.
    """
    result = [(name + "/", b"", stat.S_IFDIR | 0o755) for name in sorted(verify.DIRECTORIES)]
    header = struct.pack("<8I", 0xFEEDFACF, verify.CPU_TYPES[architecture], 0, 2, 0, 0, 0, 0)
    optional = set(verify.OPTIONAL_FILES)
    if not stapled:
        optional.discard(verify.STAPLED_TICKET)
    for name in sorted(verify.REQUIRED_FILES | optional):
        data = header if name in verify.MACHOS else b"synthetic fixture only\n"
        if name == verify.PLIST:
            data = plist()
        if name == ENGINE_PLIST:
            data = engine_plist()
        if name == verify.MANIFEST:
            data = manifest(architecture, notarized="true" if stapled else "false")
        if name == verify.DRIVER_LICENSE:
            data = verify.DRIVER_LICENSE_BYTES
        if name == NOTICES:
            data = NOTICES_HEADER + b"====\n\nsynthetic fixture only\n"
        if name == verify.STAPLED_TICKET:
            data = TICKET
        mode = 0o755 if name in verify.EXECUTABLES else 0o644
        result.append((name, data, stat.S_IFREG | mode))
    return result


def changed(original, name, *, data=None, mode=None):
    return [
        (
            path,
            data if path == name and data is not None else payload,
            mode if path == name and mode is not None else permissions,
        )
        for path, payload, permissions in original
    ]


class NonSeekingOutput(io.BytesIO):
    """Force zipfile to produce genuine bit-3 data descriptors like ditto."""

    def seek(self, *args):
        raise OSError("synthetic unseekable writer")


def zip_bytes(original, compression=zipfile.ZIP_STORED, *, descriptors=False):
    output = NonSeekingOutput() if descriptors else io.BytesIO()
    with zipfile.ZipFile(output, "w", compression=zipfile.ZIP_STORED) as archive:
        for name, data, mode in original:
            info = zipfile.ZipInfo(name)
            info.compress_type = compression
            info.create_system = 3
            info.external_attr = mode << 16
            with warnings.catch_warnings():
                warnings.simplefilter("ignore", UserWarning)  # Intentional duplicate fixtures.
                archive.writestr(info, data)
    return output.getvalue()


def member_headers(raw, name):
    """Locate real central/local records without changing the ZIP layout."""
    with zipfile.ZipFile(io.BytesIO(raw)) as archive:
        info = archive.getinfo(name)
        cursor = archive.start_dir
        for candidate in archive.infolist():
            if candidate.filename == name:
                return info, cursor
            sizes = struct.unpack_from("<HHH", raw, cursor + 28)
            cursor += 46 + sum(sizes)
    raise AssertionError("synthetic member not found")


def forge_sizes(raw, name, *, size, checksum):
    info, central = member_headers(raw, name)
    forged = bytearray(raw)
    struct.pack_into("<I", forged, info.header_offset + 14, checksum)
    struct.pack_into("<I", forged, info.header_offset + 22, size)
    struct.pack_into("<I", forged, central + 16, checksum)
    struct.pack_into("<I", forged, central + 24, size)
    return bytes(forged)


def deflate_container(payload, original):
    """Insert exact compressed bytes, including intentionally invalid stream tails."""
    name = verify.RESOURCES + "/LICENSE"
    raw = zip_bytes(changed(entries(), name, data=payload))
    info, central = member_headers(raw, name)
    forged = bytearray(forge_sizes(raw, name, size=len(original), checksum=zlib.crc32(original)))
    struct.pack_into("<H", forged, info.header_offset + 8, zipfile.ZIP_DEFLATED)
    struct.pack_into("<H", forged, central + 10, zipfile.ZIP_DEFLATED)
    return bytes(forged)


class ArchiveVerificationTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(prefix="orbit-archive-fixture-")
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)

    def fixture(self, original=None, *, architecture="arm64", raw=None):
        data = (
            zip_bytes(entries(architecture) if original is None else original)
            if raw is None
            else raw
        )
        path = self.root / f"Orbit-{verify.VERSION}-macos-{architecture}.zip"
        path.write_bytes(data)
        checksum = self.root / (path.name + ".sha256")
        checksum.write_text(f"{hashlib.sha256(data).hexdigest()}  {path.name}\n", encoding="ascii")
        return path, {
            "architecture": architecture,
            "commit": COMMIT,
            "checksum_file": checksum,
        }

    def rejected(self, original=None, *, raw=None, message=None):
        path, options = self.fixture(original, raw=raw)
        if message is None:
            with self.assertRaises((verify.VerificationError, zipfile.BadZipFile)):
                verify.verify_archive(path, **options)
        else:
            with self.assertRaisesRegex((verify.VerificationError, zipfile.BadZipFile), message):
                verify.verify_archive(path, **options)

    def test_valid_arm_and_intel_without_claiming_signature_or_notarization(self):
        for architecture in verify.CPU_TYPES:
            with self.subTest(architecture=architecture):
                path, options = self.fixture(architecture=architecture)
                external = self.root / "build-manifest.txt"
                external.write_bytes(manifest(architecture))
                report = verify.verify_archive(path, **options, manifest=external)
                # The nested engine app adds its plist and resource seal; the
                # third-party notice and compiled icon are one each.
                self.assertEqual(report["members"], 25)
                self.assertEqual(report["source_commit"], COMMIT)
                self.assertEqual(report["architecture"], architecture)
                self.assertFalse(report["signature_verified"])
                self.assertFalse(report["notarization_verified"])
                # What the build claimed, kept apart from what was proved here.
                self.assertFalse(report["manifest_notarized"])
                self.assertIn("does not authenticate", report["trust_note"])

    def test_driver_is_required_executable_and_architecture_checked(self):
        driver = verify.CONTENTS + "/Helpers/Orbit Engine.app/Contents/MacOS/cua-driver"
        self.assertIn(driver, verify.REQUIRED_FILES)
        self.assertIn(driver, verify.EXECUTABLES)
        self.assertIn(driver, verify.MACHOS)
        self.rejected(
            [entry for entry in entries() if entry[0] != driver],
            message="required bundle files are missing",
        )
        self.rejected(
            changed(entries(), driver, mode=stat.S_IFREG | 0o644),
            message="executable without",
        )

    def test_compiled_icon_is_accepted_and_required_in_the_archive(self):
        name = verify.RESOURCES + "/Assets.car"
        without_icon = [entry for entry in entries() if entry[0] != name]
        path, options = self.fixture(
            without_icon + [(name, b"compiled icon fixture", stat.S_IFREG | 0o644)]
        )
        verify.verify_archive(path, **options)
        self.rejected(without_icon, message="required bundle files are missing")

    def test_nested_engine_identity_is_required_and_cannot_be_the_parent(self):
        original = entries()
        path, options = self.fixture(original)
        verify.verify_archive(path, **options)
        for field, value in (
            ("CFBundleIdentifier", "app.orbit.companion"),
            ("CFBundleName", "Orbit"),
            ("CFBundleExecutable", "Orbit"),
            ("CFBundleShortVersionString", "9.9.9"),
            ("LSUIElement", False),
        ):
            with self.subTest(field=field):
                info = plistlib.loads(engine_plist())
                info[field] = value
                self.rejected(
                    changed(original, ENGINE_PLIST, data=plistlib.dumps(info)),
                    message="engine",
                )
        self.rejected(
            [entry for entry in original if entry[0] != ENGINE_PLIST],
            message="required bundle files are missing",
        )

    def test_verification_never_extracts_executes_or_writes(self):
        path, options = self.fixture()
        before = {item.name: item.read_bytes() for item in self.root.iterdir()}
        with (
            mock.patch.object(zipfile.ZipFile, "extract", side_effect=AssertionError("extract")),
            mock.patch.object(
                zipfile.ZipFile, "extractall", side_effect=AssertionError("extractall")
            ),
            mock.patch("os.system", side_effect=AssertionError("execute")),
            mock.patch("subprocess.Popen", side_effect=AssertionError("execute")),
        ):
            verify.verify_archive(path, **options)
        self.assertEqual(before, {item.name: item.read_bytes() for item in self.root.iterdir()})

    @unittest.skipUnless(hasattr(os, "O_NOFOLLOW"), "atomic nofollow is platform-specific")
    def test_input_symlink_is_refused_without_following_it(self):
        path, options = self.fixture()
        target = self.root / "original.zip"
        path.rename(target)
        path.symlink_to(target)
        with self.assertRaises(OSError):
            verify.verify_archive(path, **options)
        self.assertEqual(target.read_bytes(), zip_bytes(entries()))

    @unittest.skipUnless(hasattr(os, "mkfifo") and hasattr(os, "O_NONBLOCK"), "POSIX FIFO fixture")
    def test_fifo_input_is_refused_without_waiting_for_a_writer(self):
        fifo = self.root / "synthetic.fifo"
        os.mkfifo(fifo, 0o600)
        with self.assertRaisesRegex(verify.VerificationError, "regular file"):
            verify.read_limited(fifo, 4096)

    def test_checksum_digest_name_multiple_lines_and_renamed_archive_rejected(self):
        for record in (
            "0" * 64 + "  {name}\n",
            "{digest}  wrong.zip\n",
            "{digest}  {name}\nextra\n",
        ):
            path, options = self.fixture()
            options["checksum_file"].write_text(
                record.format(name=path.name, digest=hashlib.sha256(path.read_bytes()).hexdigest())
            )
            with self.assertRaisesRegex(verify.VerificationError, "SHA-256"):
                verify.verify_archive(path, **options)
        path, options = self.fixture()
        other = path.with_name("renamed.zip")
        path.rename(other)
        with self.assertRaisesRegex(verify.VerificationError, "ZIP name"):
            verify.verify_archive(other, **options)

    def test_commit_requires_exact_40_hex_and_manifest_match(self):
        path, options = self.fixture()
        for commit in ("a" * 39, "g" * 40, "A" * 40, "b" * 40):
            with (
                self.subTest(commit=commit),
                self.assertRaises(verify.VerificationError),
            ):
                verify.verify_archive(path, **{**options, "commit": commit})

    def test_external_manifest_is_byte_exact(self):
        path, options = self.fixture()
        other = self.root / "manifest.txt"
        other.write_bytes(manifest() + b"\n")
        with self.assertRaisesRegex(verify.VerificationError, "byte for byte"):
            verify.verify_archive(path, **options, manifest=other)

    def test_manifest_rejects_wrong_fields_values_duplicates_and_dirty_builds(self):
        variants = [
            manifest().replace(b"false", b"true", 1),
            # `notarized=true` used to be listed here as a rejected value, back
            # when no package was ever notarized. Now that they are, refusing it
            # would refuse the good build; what still has to be rejected is a
            # value that is neither true nor false, since anything non-empty
            # would pass a naive boolean test. The guarantee this test is named
            # for — wrong fields, wrong values, duplicates and dirty builds are
            # refused — is unchanged.
            manifest().replace(b"notarized=false", b"notarized=yes"),
            manifest().replace(b"notarized=false", b"notarized="),
            manifest() + b"unknown=value\n",
            manifest() + f"version={verify.VERSION}\n".encode(),
            manifest().replace(b"architecture=arm64", b"architecture=x86_64"),
            manifest().replace(f"version={verify.VERSION}".encode(), b"version=9.9.9"),
            manifest().replace(b"\n", b"\r\n"),
        ]
        for data in variants:
            with self.subTest(data=data):
                self.rejected(changed(entries(), verify.MANIFEST, data=data))

    def test_missing_required_file_and_unexpected_ds_store_rejected(self):
        for missing in verify.REQUIRED_FILES:
            self.rejected([entry for entry in entries() if entry[0] != missing], message="missing")
        self.rejected(
            entries() + [(verify.RESOURCES + "/.DS_Store", b"x", stat.S_IFREG | 0o644)],
            message="unexpected",
        )

    def test_an_archive_with_a_separate_daemon_is_refused(self):
        # The daemon runs inside MacOS/Orbit. A package that still
        # carries Contents/Resources/orbit was built by the old script, and its
        # daemon would never run: it is not this release.
        daemon = verify.RESOURCES + "/orbit"
        self.assertNotIn(daemon, verify.REQUIRED_FILES)
        self.assertNotIn(daemon, verify.MACHOS)
        header = struct.pack("<8I", 0xFEEDFACF, verify.CPU_TYPES["arm64"], 0, 2, 0, 0, 0, 0)
        self.rejected(entries() + [(daemon, header, stat.S_IFREG | 0o755)], message="unexpected")

    def test_missing_or_modified_cua_driver_license_is_rejected(self):
        self.rejected(
            [entry for entry in entries() if entry[0] != verify.DRIVER_LICENSE],
            message="required bundle files are missing",
        )
        self.rejected(
            changed(
                entries(),
                verify.DRIVER_LICENSE,
                data=verify.DRIVER_LICENSE_BYTES + b"modified\n",
            ),
            message="cua-driver's licence",
        )

    def test_the_third_party_notice_is_required_as_its_generator_writes_it(self):
        self.assertIn(NOTICES, verify.REQUIRED_FILES)
        self.rejected(
            [entry for entry in entries() if entry[0] != NOTICES],
            message="required bundle files are missing",
        )
        self.rejected(
            changed(entries(), NOTICES, data=b"synthetic fixture only\n"),
            message="third-party notice",
        )
        # Large like a real one (about 1 MB): only its first line is kept.
        path, options = self.fixture(
            changed(entries(), NOTICES, data=NOTICES_HEADER + b"licence text\n" * 80_000)
        )
        verify.verify_archive(path, **options)

    def test_a_notarized_package_carries_its_stapled_ticket(self):
        path, options = self.fixture(entries(stapled=True))
        report = verify.verify_archive(path, **options)
        self.assertEqual(report["members"], 26)
        self.assertTrue(report["manifest_notarized"])
        self.assertTrue(report["stapled_ticket"])
        # Present and marked is not valid: nothing here talks to Apple.
        self.assertFalse(report["notarization_verified"])

    def test_claiming_notarization_without_the_stapled_ticket_is_rejected(self):
        # The failure this guards against is silent and only bites offline: a
        # ZIP built before stapling stays notarized on Apple's servers and
        # unstapled on the disk of whoever downloads it.
        self.rejected(
            [entry for entry in entries(stapled=True) if entry[0] != verify.STAPLED_TICKET],
            message="contradict",
        )

    def test_a_stapled_ticket_the_manifest_does_not_declare_is_rejected(self):
        self.rejected(
            entries() + [(verify.STAPLED_TICKET, TICKET, stat.S_IFREG | 0o644)],
            message="contradict",
        )

    def test_a_file_parked_at_the_ticket_path_is_not_a_ticket(self):
        self.rejected(
            changed(entries(stapled=True), verify.STAPLED_TICKET, data=b"not a ticket\n"),
            message="mark",
        )

    def test_explicit_directory_and_signature_records_are_optional(self):
        original = [
            entry
            for entry in entries()
            if not entry[0].endswith("/") and entry[0] not in verify.OPTIONAL_FILES
        ]
        path, options = self.fixture(original)
        self.assertEqual(verify.verify_archive(path, **options)["members"], 11)

    def test_duplicate_casefold_and_unicode_nfc_collisions(self):
        for first, second in (
            ("extra", "extra"),
            ("EXTRA", "extra"),
            ("e\u0301", "\u00e9"),
        ):
            with self.subTest(names=(first, second)):
                extra = [
                    (verify.RESOURCES + "/" + name, b"", stat.S_IFREG | 0o644)
                    for name in (first, second)
                ]
                self.rejected(entries() + extra, message="duplicate|collision")

    def test_file_directory_prefix_conflict(self):
        original = [entry for entry in entries() if entry[0] != verify.CONTENTS + "/MacOS/"]
        original.append((verify.CONTENTS + "/MacOS", b"x", stat.S_IFREG | 0o644))
        self.rejected(original, message="conflict between a ZIP file and directory")

    def test_traversal_absolute_backslash_empty_component_and_controls(self):
        for name in (
            "../outside",
            "/outside",
            "C:/outside",
            "Orbit.app\\outside",
            "Orbit.app//outside",
            "Orbit.app/./outside",
            "Orbit.app/../outside",
            "Orbit.app/line\nfeed",
        ):
            with self.subTest(name=name):
                self.rejected(entries() + [(name, b"x", stat.S_IFREG | 0o644)])

    def test_nul_filename_is_not_silently_truncated(self):
        raw = zip_bytes(entries() + [(verify.RESOURCES + "/badXname", b"", stat.S_IFREG | 0o644)])
        self.rejected(raw=raw.replace(b"badXname", b"bad\x00name"), message="NUL")

    def test_symlink_special_modes_and_missing_execution_permissions(self):
        name = next(iter(verify.MACHOS))
        for mode in (
            stat.S_IFLNK | 0o755,
            stat.S_IFIFO | 0o644,
            stat.S_IFREG | 0o4755,
            stat.S_IFREG | 0o777,
            stat.S_IFREG | 0o644,
        ):
            with self.subTest(mode=oct(mode)):
                self.rejected(changed(entries(), name, mode=mode))

    def test_macho_requires_each_exact_cpu_thin_header_and_executable_type(self):
        for name in verify.MACHOS:
            for data in (
                b"truncated",
                b"\xca\xfe\xba\xbe" + b"x" * 28,
                struct.pack("<8I", 0xFEEDFACF, verify.CPU_TYPES["x86_64"], 0, 2, 0, 0, 0, 0),
                struct.pack("<8I", 0xFEEDFACF, verify.CPU_TYPES["arm64"], 0, 6, 0, 0, 0, 0),
                struct.pack("<8I", 0xFEEDFACF, verify.CPU_TYPES["arm64"], 0, 2, 1, 128, 0, 0),
            ):
                with self.subTest(name=name, data=data):
                    self.rejected(changed(entries(), name, data=data), message="Mach-O")

    def test_plist_identity_version_minimum_and_duplicate_keys(self):
        base = plistlib.loads(plist())
        for field, value in (
            ("CFBundleIdentifier", "other.app"),
            ("CFBundleShortVersionString", "9.9.9"),
            ("LSMinimumSystemVersion", "14.9"),
            ("LSMinimumSystemVersion", True),
            ("CFBundleExecutable", "other"),
        ):
            self.rejected(
                changed(entries(), verify.PLIST, data=plistlib.dumps({**base, field: value}))
            )
        duplicate = plist().replace(
            b"</dict>",
            b"<key>CFBundleIdentifier</key><string>app.orbit.companion</string></dict>",
        )
        self.rejected(changed(entries(), verify.PLIST, data=duplicate), message="duplicate")

    def test_minimum_macos_15_plus_must_match_both_documents(self):
        original = changed(
            changed(entries(), verify.PLIST, data=plist("16.0")),
            verify.MANIFEST,
            data=manifest(minimum="16.0"),
        )
        path, options = self.fixture(original)
        self.assertEqual(verify.verify_archive(path, **options)["minimum_macos"], "16.0")
        self.rejected(changed(entries(), verify.PLIST, data=plist("16.0")))

    def test_crc_checked_even_for_license_not_only_metadata_and_binaries(self):
        original = changed(entries(), verify.RESOURCES + "/LICENSE", data=b"UNIQUE_CRC_FIXTURE")
        raw = zip_bytes(original).replace(b"UNIQUE_CRC_FIXTURE", b"UNIQUE_CRC_FIXTURX", 1)
        self.rejected(raw=raw, message="CRC")

    def test_stored_and_deflate_cannot_hide_suffix_behind_prefix_crc_and_size(self):
        name = verify.RESOURCES + "/LICENSE"
        for compression in (zipfile.ZIP_STORED, zipfile.ZIP_DEFLATED):
            # Exact reviewer reproduction, plus output crossing the decoder's
            # 64 KiB block boundary. This changes REAL local and central headers.
            for original in (b"XUNACCOUNTED_TAIL", b"X" + b"A" * 150_000):
                with self.subTest(compression=compression, length=len(original)):
                    raw = zip_bytes(changed(entries(), name, data=original), compression)
                    forged = forge_sizes(raw, name, size=1, checksum=zlib.crc32(b"X"))
                    # Demonstrate why using ZipExtFile plus len/CRC is insufficient.
                    with zipfile.ZipFile(io.BytesIO(forged)) as clipped:
                        self.assertEqual(clipped.read(name), b"X")
                    self.rejected(raw=forged, message="ZIP_STORED|expansion")

    def test_raw_deflate_rejects_unfinished_stream_and_unused_or_second_stream(self):
        original = b"synthetic complete output" * 20
        encoder = zlib.compressobj(wbits=-15)
        stream = encoder.compress(original) + encoder.flush()
        path, options = self.fixture(raw=deflate_container(stream, original))
        verify.verify_archive(path, **options)
        for payload in (stream[:-1], stream + b"UNACCOUNTED", stream + stream):
            with self.subTest(payload=payload):
                self.rejected(raw=deflate_container(payload, original), message="DEFLATE")

    def test_large_valid_deflate_crosses_output_blocks_without_truncation(self):
        name = verify.RESOURCES + "/LICENSE"
        original = b"all output must be checked" * 10_000
        raw = zip_bytes(changed(entries(), name, data=original), zipfile.ZIP_DEFLATED)
        path, options = self.fixture(raw=raw)
        self.assertGreater(
            verify.verify_archive(path, **options)["uncompressed_bytes"], len(original)
        )

    def test_real_data_descriptors_match_crc_sizes_and_member_boundaries(self):
        for compression in (zipfile.ZIP_STORED, zipfile.ZIP_DEFLATED):
            raw = zip_bytes(entries(), compression, descriptors=True)
            path, options = self.fixture(raw=raw)
            verify.verify_archive(path, **options)
            name = verify.RESOURCES + "/LICENSE"
            info, _ = member_headers(raw, name)
            name_size, extra_size = struct.unpack_from("<HH", raw, info.header_offset + 26)
            descriptor = info.header_offset + 30 + name_size + extra_size + info.compress_size
            self.assertEqual(raw[descriptor : descriptor + 4], b"PK\x07\x08")
            for relative in (0, 4, 8, 12):
                forged = bytearray(raw)
                forged[descriptor + relative] ^= 1
                self.rejected(raw=bytes(forged), message="descriptor")

    def test_local_header_must_match_central_crc_sizes_flags_method_and_name(self):
        raw = zip_bytes(entries())
        info, _ = member_headers(raw, verify.RESOURCES + "/LICENSE")
        for relative in (4, 6, 8, 14, 18, 22, 30):
            with self.subTest(relative=relative):
                forged = bytearray(raw)
                forged[info.header_offset + relative] ^= 1
                self.rejected(raw=bytes(forged), message="local|overlapping")

    def test_zip_member_total_metadata_and_outer_size_budgets(self):
        path, options = self.fixture()
        for constant, limit in (
            ("MAX_ZIP_BYTES", path.stat().st_size - 1),
            ("MAX_MEMBER_BYTES", 31),
            ("MAX_TOTAL_BYTES", 64),
            ("MAX_METADATA_BYTES", 32),
        ):
            with (
                self.subTest(constant=constant),
                mock.patch.object(verify, constant, limit),
                self.assertRaises(verify.VerificationError),
            ):
                verify.verify_archive(path, **options)

    def test_member_count_preflight_rejects_real_and_falsely_small_counts(self):
        original = entries() + [
            (verify.RESOURCES + f"/extra-{index}", b"", stat.S_IFREG | 0o644)
            for index in range(129)
        ]
        raw = zip_bytes(original)
        self.rejected(raw=raw, message="members")
        forged = bytearray(raw)
        end = forged.rfind(b"PK\x05\x06")
        struct.pack_into("<HH", forged, end + 8, 1, 1)
        self.rejected(raw=bytes(forged), message="ZIP directory limit exceeded")

    def test_truncated_nonzip_appended_and_multidisk_archives_rejected(self):
        raw = zip_bytes(entries())
        for data in (b"not a zip", raw[:-10], raw + b"appended bytes"):
            self.rejected(raw=data)
        forged = bytearray(raw)
        struct.pack_into("<H", forged, forged.rfind(b"PK\x05\x06") + 4, 1)
        self.rejected(raw=bytes(forged), message="multi-disk")

    def test_encrypted_and_unsupported_compression_flags_rejected(self):
        raw = zip_bytes(entries())
        for offset, value, message in (
            (8, 1, "encrypted"),
            (10, zipfile.ZIP_BZIP2, "compression"),
        ):
            forged = bytearray(raw)
            struct.pack_into("<H", forged, forged.find(b"PK\x01\x02") + offset, value)
            self.rejected(raw=bytes(forged), message=message)

    def test_cli_success_and_rejection_are_structured_without_traceback(self):
        path, options = self.fixture()
        arguments = [
            str(path),
            "--architecture",
            "arm64",
            "--commit",
            COMMIT,
            "--checksum-file",
            str(options["checksum_file"]),
        ]
        with redirect_stdout(io.StringIO()) as output:
            self.assertEqual(verify.main(arguments), 0)
        self.assertIn('"signature_verified": false', output.getvalue())
        options["checksum_file"].write_bytes(b"wrong")
        with redirect_stderr(io.StringIO()) as errors:
            self.assertEqual(verify.main(arguments), 1)
        self.assertIn("Verification refused", errors.getvalue())
        self.assertNotIn("Traceback", errors.getvalue())


if __name__ == "__main__":
    unittest.main()
