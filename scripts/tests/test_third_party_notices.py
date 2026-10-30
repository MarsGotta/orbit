"""The notice of the software by others that Orbit.app ships.

scripts/release/third-party-notices.py writes it at build time from what the
build installed: the interface's packages as `pnpm licenses list --prod --json`
lists them, and the crates compiled into the app as `cargo metadata` resolves
them. Here both listings are synthetic and their packages are directories of
this test; package and author names are invented.
"""

import importlib.util
import json
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "scripts" / "release" / "third-party-notices.py"


def load(path, name):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


MIT_OF = (
    "MIT License\n\nCopyright (c) 2024 {who}\n\nPermission is hereby granted, free of charge.\n"
)


class Fixture:
    """Two interface packages, three crates and the app's own two crates."""

    def __init__(self, scratch: Path):
        self.scratch = scratch
        self.pnpm = {
            "MIT": [
                {
                    "name": "@ejemplo/editor",
                    "versions": ["2.1.0"],
                    "paths": [
                        str(self.package("editor", {"LICENSE": MIT_OF.format(who="Ana Ruiz")}))
                    ],
                    "license": "MIT",
                },
                {
                    "name": "sin-fichero",
                    "versions": ["1.0.0"],
                    "paths": [str(self.package("sin-fichero", {"README.md": "léeme\n"}))],
                    "license": "MIT",
                },
            ],
            "ISC": [
                {
                    "name": "colores",
                    "versions": ["0.3.0", "0.4.1"],
                    "paths": [
                        str(self.package("colores-0.3", {"license": "ISC de Bruno Sanz\n"})),
                        str(self.package("colores-0.4", {"license": "ISC de Bruno Sanz\n"})),
                    ],
                    "license": "ISC",
                }
            ],
        }
        app = self.package("app", {})
        core = self.package("core", {})
        tiempo = self.package(
            "tiempo",
            {
                "LICENSE-MIT": MIT_OF.format(who="Carla Gil"),
                "LICENSE-APACHE": "Apache License\nVersion 2.0\n",
            },
        )
        copia = self.package("copia", {"LICENSE-MIT": MIT_OF.format(who="Carla Gil")})
        propio = self.package("propio", {"COPYING.txt": "Licencia propia de Dario Paz\n"})
        prueba = self.package("prueba", {"LICENSE": "solo para pruebas\n"})
        construir = self.package("construir", {"LICENSE": "solo para compilar\n"})

        def crate(name, version, directory, licence, source="registry+https://example.test"):
            return {
                "id": f"{name} {version}",
                "name": name,
                "version": version,
                "license": licence,
                "license_file": None,
                "manifest_path": str(directory / "Cargo.toml"),
                "source": source,
            }

        def normal(pkg):
            return {"pkg": pkg, "dep_kinds": [{"kind": None, "target": None}]}

        self.cargo = {
            "packages": [
                crate("orbit-desktop", "0.11.0", app, None, source=None),
                crate("orbit-core", "0.11.0", core, None, source=None),
                crate("tiempo", "1.2.3", tiempo, "MIT OR Apache-2.0"),
                crate("copia", "0.1.0", copia, "MIT"),
                crate("propio", "4.0.0", propio, "LicenseRef-propia"),
                crate("prueba", "9.9.9", prueba, "MIT"),
                crate("construir", "5.0.0", construir, "MIT"),
            ],
            "workspace_members": ["orbit-desktop 0.11.0", "orbit-core 0.11.0"],
            "resolve": {
                "root": None,
                "nodes": [
                    {
                        "id": "orbit-desktop 0.11.0",
                        "deps": [
                            normal("orbit-core 0.11.0"),
                            normal("tiempo 1.2.3"),
                            {"pkg": "prueba 9.9.9", "dep_kinds": [{"kind": "dev", "target": None}]},
                            {
                                "pkg": "construir 5.0.0",
                                "dep_kinds": [{"kind": "build", "target": None}],
                            },
                        ],
                    },
                    {
                        "id": "orbit-core 0.11.0",
                        "deps": [normal("copia 0.1.0"), normal("propio 4.0.0")],
                    },
                    {"id": "tiempo 1.2.3", "deps": []},
                    {"id": "copia 0.1.0", "deps": []},
                    {"id": "propio 4.0.0", "deps": []},
                    {"id": "prueba 9.9.9", "deps": []},
                    {"id": "construir 5.0.0", "deps": []},
                ],
            },
        }

    def package(self, name, files):
        directory = self.scratch / "packages" / name
        directory.mkdir(parents=True)
        for file, text in files.items():
            (directory / file).write_text(text, encoding="utf-8")
        return directory

    def write(self):
        pnpm = self.scratch / "pnpm.json"
        cargo = self.scratch / "cargo.json"
        pnpm.write_text(json.dumps(self.pnpm), encoding="utf-8")
        cargo.write_text(json.dumps(self.cargo), encoding="utf-8")
        return pnpm, cargo


class NoticeTests(unittest.TestCase):
    def setUp(self):
        self.scratch = Path(tempfile.mkdtemp(prefix="orbit-notices-"))
        self.addCleanup(shutil.rmtree, self.scratch)
        self.fixture = Fixture(self.scratch)

    def generate(self, **change):
        pnpm, cargo = self.fixture.write()
        output = self.scratch / "out" / "NOTICES.txt"
        output.parent.mkdir(exist_ok=True)
        arguments = {
            "--pnpm-licenses": pnpm,
            "--cargo-metadata": cargo,
            "--root": "orbit-desktop",
            "--output": output,
            **change,
        }
        completed = subprocess.run(
            [sys.executable, "-B", str(SCRIPT)]
            + [str(part) for pair in arguments.items() for part in pair],
            capture_output=True,
            text=True,
            timeout=60,
            check=False,
        )
        text = output.read_text(encoding="utf-8") if output.exists() else None
        return completed, text

    def test_every_shipped_package_is_named_with_its_version_and_licence(self):
        completed, text = self.generate()
        self.assertEqual(completed.returncode, 0, completed.stderr)
        for line in (
            "@ejemplo/editor 2.1.0 · MIT",
            "colores 0.3.0 · ISC",
            "colores 0.4.1 · ISC",
            "sin-fichero 1.0.0 · MIT",
            "tiempo 1.2.3 · MIT OR Apache-2.0",
            "copia 0.1.0 · MIT",
            "propio 4.0.0 · LicenseRef-propia",
        ):
            self.assertIn(line, text)
        # Orbit's own crates are under Orbit's LICENSE; a crate only tests or
        # the build use is not in the app.
        for absent in (
            "orbit-desktop 0.11.0",
            "orbit-core 0.11.0",
            "prueba 9.9.9",
            "construir 5.0.0",
        ):
            self.assertNotIn(absent, text)

    def test_each_licence_text_ships_once_with_the_packages_that_carry_it(self):
        completed, text = self.generate()
        self.assertEqual(completed.returncode, 0, completed.stderr)
        for licence in (
            MIT_OF.format(who="Ana Ruiz"),
            MIT_OF.format(who="Carla Gil"),
            "Apache License\nVersion 2.0\n",
            "ISC de Bruno Sanz\n",
            "Licencia propia de Dario Paz\n",
        ):
            self.assertEqual(text.count(licence.strip()), 1, licence)
        self.assertIn("sin-fichero 1.0.0 · MIT · its package carries no licence file", text)

    def test_the_notice_starts_with_the_header_the_archive_verifier_checks(self):
        completed, text = self.generate()
        self.assertEqual(completed.returncode, 0, completed.stderr)
        notices = load(SCRIPT, "third_party_notices")
        verify = load(
            ROOT / "scripts" / "release" / "verify-macos-archive.py", "verify_macos_archive"
        )
        self.assertEqual(notices.HEADER, verify.NOTICES_HEADER.decode("utf-8"))
        self.assertTrue(text.startswith(notices.HEADER))
        self.assertLessEqual(len(verify.NOTICES_HEADER), 32, "the verifier keeps 32 bytes of it")

    def test_the_same_listings_write_the_same_notice(self):
        _, first = self.generate()
        _, second = self.generate()
        self.assertEqual(first, second)

    def test_no_path_of_the_build_machine_is_written(self):
        _, text = self.generate()
        self.assertNotIn(str(self.scratch), text)

    def test_an_empty_listing_is_refused_rather_than_shipping_no_notice(self):
        empty = self.scratch / "empty.json"
        empty.write_text("{}", encoding="utf-8")
        completed, text = self.generate(**{"--pnpm-licenses": empty})
        self.assertNotEqual(completed.returncode, 0)
        self.assertIn("pnpm", completed.stderr)
        self.assertIsNone(text)
        nothing = self.scratch / "nothing.json"
        nothing.write_text("", encoding="utf-8")
        completed, text = self.generate(**{"--cargo-metadata": nothing})
        self.assertNotEqual(completed.returncode, 0)
        self.assertIn("cargo metadata", completed.stderr)
        self.assertIsNone(text)

    def test_a_package_that_is_not_where_its_listing_says_is_refused_by_name(self):
        shutil.rmtree(self.scratch / "packages" / "tiempo")
        completed, text = self.generate()
        self.assertNotEqual(completed.returncode, 0)
        self.assertIn("tiempo 1.2.3", completed.stderr)
        self.assertIsNone(text)

    def test_an_unknown_root_is_refused(self):
        completed, _ = self.generate(**{"--root": "otra-app"})
        self.assertNotEqual(completed.returncode, 0)
        self.assertIn("otra-app", completed.stderr)


if __name__ == "__main__":
    unittest.main()
