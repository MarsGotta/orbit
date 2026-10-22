"""Public builds and deployment scripts require explicit infrastructure."""

import importlib.util
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import tomllib

ROOT = Path(__file__).resolve().parents[2]


class InfrastructureTests(unittest.TestCase):
    def harness(self):
        path = ROOT / "scripts/e2e-mac/isolated_hub.py"
        spec = importlib.util.spec_from_file_location("isolated_hub_fixture", path)
        module = importlib.util.module_from_spec(spec)
        sys.modules[spec.name] = module
        self.addCleanup(sys.modules.pop, spec.name, None)
        spec.loader.exec_module(module)
        return module

    def test_hub_script_configuration_refuses_a_missing_base_before_any_effect(self):
        for variables, missing in (
            ({}, "ORBIT_HUB_BASE"),
            ({"ORBIT_HUB_BASE": "/fixture"}, "ORBIT_DEIMOS_PROFILE"),
        ):
            with self.subTest(missing=missing):
                result = subprocess.run(
                    [
                        "bash",
                        "-c",
                        'source "$1"; printf allowed',
                        "bash",
                        str(ROOT / "scripts/hub/lib.sh"),
                    ],
                    env={"PATH": "/usr/bin:/bin", "HOME": "/nonexistent", **variables},
                    text=True,
                    capture_output=True,
                    timeout=10,
                    check=False,
                )
                self.assertNotEqual(result.returncode, 0)
                self.assertIn(missing, result.stderr)
                self.assertEqual(result.stdout, "")

    def test_installers_refuse_a_missing_origin_before_any_effect(self):
        for name in ("scripts/release/install.sh", "scripts/release/install-on-mac.sh"):
            with self.subTest(script=name), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                home, tools = root / "home", root / "bin"
                home.mkdir()
                tools.mkdir()
                # Even the old fallback on a Mac cannot use its network or processes.
                for tool, body in (("uname", "printf 'Linux\\n'"), ("curl", "exit 73")):
                    executable = tools / tool
                    executable.write_text(f"#!/bin/sh\n{body}\n")
                    executable.chmod(0o700)
                result = subprocess.run(
                    ["bash", str(ROOT / name)],
                    env={"PATH": f"{tools}:/usr/bin:/bin", "HOME": str(home)},
                    text=True,
                    capture_output=True,
                    timeout=10,
                    check=False,
                )
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("ORBIT_INSTALL_BASE", result.stderr)
                self.assertEqual(list(home.iterdir()), [])

    def test_symbol_upload_requires_organization_and_project(self):
        with tempfile.TemporaryDirectory() as folder:
            symbols = Path(folder) / "1.0.0/orbit-desktop.dSYM.zip"
            symbols.parent.mkdir()
            symbols.touch()
            for extra, missing in (
                ({}, "SENTRY_ORG"),
                ({"SENTRY_ORG": "example-org"}, "SENTRY_PROJECT"),
            ):
                with self.subTest(missing=missing):
                    result = subprocess.run(
                        [
                            "bash",
                            str(ROOT / "scripts/release/upload-symbols.sh"),
                            "1.0.0",
                        ],
                        env={
                            "PATH": "/usr/bin:/bin",
                            "ORBIT_RELEASE_DIR": folder,
                            **extra,
                        },
                        text=True,
                        capture_output=True,
                        timeout=10,
                        check=False,
                    )
                    self.assertNotEqual(result.returncode, 0)
                    self.assertIn(missing, result.stderr)

            tools = Path(folder) / "bin"
            tools.mkdir()
            sentry = tools / "sentry"
            sentry.write_text('#!/bin/sh\nprintf "%s\\n" "$SENTRY_ORG" "$SENTRY_PROJECT" "$@"\n')
            sentry.chmod(0o700)
            configured = subprocess.run(
                ["bash", str(ROOT / "scripts/release/upload-symbols.sh"), "1.0.0"],
                env={
                    "PATH": f"{tools}:/usr/bin:/bin",
                    "ORBIT_RELEASE_DIR": folder,
                    "SENTRY_ORG": "example-org",
                    "SENTRY_PROJECT": "example-project",
                },
                text=True,
                capture_output=True,
                timeout=10,
                check=False,
            )
            self.assertEqual(configured.returncode, 0, configured.stderr)
            self.assertEqual(
                configured.stdout.splitlines(),
                [
                    "example-org",
                    "example-project",
                    "debug-files",
                    "upload",
                    "--type",
                    "dsym",
                    "--wait",
                    str(symbols),
                ],
            )

    def test_real_harness_requires_provider_configuration_but_dry_run_is_isolated(self):
        module = self.harness()
        environment = {
            "ORBIT_TEST_WHISPER_URL": "https://transcriber.example.test",
            "ORBIT_TEST_PROCESSING_URL": "https://model.example.test/v1/chat/completions",
            "ORBIT_TEST_PROCESSING_KEY_FILE": "/fixture/model.key",
            "ORBIT_DIARIZATION_PYTHON": "/fixture/python",
            "ORBIT_DIARIZATION_BUNDLE": "/fixture/bundle",
            "ORBIT_DIARIZATION_GPU_LOCK": "/fixture/inference.lock",
        }
        for missing in environment:
            environment = {
                "ORBIT_TEST_WHISPER_URL": "https://transcriber.example.test",
                "ORBIT_TEST_PROCESSING_URL": "https://model.example.test/v1/chat/completions",
                "ORBIT_TEST_PROCESSING_KEY_FILE": "/fixture/model.key",
                "ORBIT_TEST_HERMES_PROFILE": "fixture-assistant",
                "ORBIT_DIARIZATION_PYTHON": "/fixture/python",
                "ORBIT_DIARIZATION_BUNDLE": "/fixture/bundle",
                "ORBIT_DIARIZATION_GPU_LOCK": "/fixture/inference.lock",
            }
        for missing in environment:
            with (
                self.subTest(missing=missing),
                patch.dict(os.environ, environment, clear=True),
            ):
                del os.environ[missing]
                with self.assertRaisesRegex(RuntimeError, missing):
                    module.config_text(module.Paths(Path("/fixture")), "127.0.0.1", 8653)
        with patch.dict(os.environ, environment, clear=True):
            config = tomllib.loads(
                module.config_text(module.Paths(Path("/fixture")), "127.0.0.1", 8653)
            )
            self.assertEqual(
                config["processing"]["endpoint"],
                environment["ORBIT_TEST_PROCESSING_URL"],
            )
            self.assertEqual(
                config["diarization"]["bundle"],
                environment["ORBIT_DIARIZATION_BUNDLE"],
            )
            self.assertEqual(config["profile"], environment["ORBIT_TEST_HERMES_PROFILE"])
        with patch.dict(os.environ, {"HOME": "/nonexistent", "PATH": "/usr/bin:/bin"}, clear=True):
            config = tomllib.loads(
                module.config_text(
                    module.Paths(Path("/fixture")),
                    "127.0.0.1",
                    8653,
                    {
                        "whisper": "http://127.0.0.1:1234",
                        "processing": "http://127.0.0.1:1235",
                        "processing_key": "/fixture/model.key",
                    },
                )
            )
            self.assertNotIn("diarization", config)
            with self.assertRaisesRegex(RuntimeError, "ORBIT_TEST_HERMES_TOKEN_ENV"):
                module.start_hermes(module.Paths(Path("/fixture")), 8653, Path("/fixture/log"))

    def test_systemd_unit_requires_paths_and_renders_the_selected_release(self):
        script = ROOT / "scripts/hub/render-unit.sh"
        for variables, missing in (
            ({}, "ORBIT_HUB_BASE"),
            ({"ORBIT_HUB_BASE": "/opt/sample-hub"}, "ORBIT_HUB_CONFIG"),
            (
                {
                    "ORBIT_HUB_BASE": "/opt/sample-hub",
                    "ORBIT_HUB_CONFIG": "/opt/sample-config/hub.toml",
                },
                "ORBIT_HUB_ENV_FILE",
            ),
        ):
            with self.subTest(missing=missing):
                result = subprocess.run(
                    ["bash", str(script)],
                    env={"PATH": "/usr/bin:/bin", **variables},
                    capture_output=True,
                    text=True,
                    timeout=10,
                    check=False,
                )
                self.assertNotEqual(result.returncode, 0)
                self.assertIn(missing, result.stderr)
        result = subprocess.run(
            ["bash", str(script)],
            env={
                "PATH": "/usr/bin:/bin",
                "ORBIT_HUB_BASE": "/opt/sample-hub",
                "ORBIT_HUB_CONFIG": "/opt/sample-config/hub.toml",
                "ORBIT_HUB_ENV_FILE": "/opt/sample-config/hub.env",
            },
            capture_output=True,
            text=True,
            timeout=10,
            check=False,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn(
            "ExecStart=/opt/sample-hub/current/.venv/bin/orbit-hub --config /opt/sample-config/hub.toml serve",
            result.stdout,
        )
        self.assertIn("WorkingDirectory=/opt/sample-hub/current\n", result.stdout)
        self.assertIn("EnvironmentFile=/opt/sample-config/hub.env\n", result.stdout)
        self.assertNotIn("@ORBIT_", result.stdout)

    def test_the_nightly_copy_needs_its_destination_and_copies_with_the_active_release(self):
        script = ROOT / "scripts/hub/render-unit.sh"
        paths = {
            "PATH": "/usr/bin:/bin",
            "ORBIT_HUB_BASE": "/opt/sample-hub",
            "ORBIT_HUB_CONFIG": "/opt/sample-config/hub.toml",
            "ORBIT_HUB_ENV_FILE": "/opt/sample-config/hub.env",
        }

        def render(name, **variables):
            return subprocess.run(
                ["bash", str(script), name],
                env={**paths, **variables},
                capture_output=True,
                text=True,
                timeout=10,
                check=False,
            )

        missing = render("orbit-hub-backup.service")
        self.assertNotEqual(missing.returncode, 0)
        self.assertIn("ORBIT_HUB_BACKUP_DIR is missing", missing.stderr)
        service = render("orbit-hub-backup.service", ORBIT_HUB_BACKUP_DIR="/mnt/sample-copies")
        self.assertEqual(service.returncode, 0, service.stderr)
        self.assertIn(
            "ExecStart=/opt/sample-hub/current/.venv/bin/orbit-hub --config"
            " /opt/sample-config/hub.toml backup /mnt/sample-copies\n",
            service.stdout,
        )
        self.assertIn("Type=oneshot\n", service.stdout)
        self.assertNotIn("@ORBIT_", service.stdout)
        timer = render("orbit-hub-backup.timer")
        self.assertEqual(timer.returncode, 0, timer.stderr)
        self.assertIn("OnCalendar=*-*-* 03:30:00\n", timer.stdout)
        self.assertIn("Persistent=true\n", timer.stdout)
        unknown = render("../../../etc/passwd")
        self.assertEqual(unknown.returncode, 2)

    def test_real_harness_passes_the_selected_profile_and_token_to_its_process(self):
        module = self.harness()
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            paths = module.Paths(root)
            paths.secrets.mkdir()
            paths.hermes_token.write_text("fixture-token")
            hermes = root / "hermes"
            hermes.write_text(
                '#!/bin/sh\nprintf "%s\\n" "$@"\n'
                'test "${ORBIT_FAKE_TOKEN:-}" = fixture-token || exit 73\n'
            )
            hermes.chmod(0o700)
            log = root / "launch.log"
            with patch.dict(
                os.environ,
                {
                    "PATH": "/usr/bin:/bin",
                    "HOME": str(root),
                    "ORBIT_HUB_HERMES": str(hermes),
                    "ORBIT_TEST_HERMES_PROFILE": "fixture-assistant",
                    "ORBIT_TEST_HERMES_TOKEN_ENV": "ORBIT_FAKE_TOKEN",
                },
                clear=True,
            ):
                process = module.start_hermes(paths, 8653, log)
                try:
                    self.assertEqual(process.popen.wait(timeout=10), 0)
                finally:
                    process.stop(10)
            self.assertEqual(
                log.read_text().splitlines(),
                [
                    "--profile",
                    "fixture-assistant",
                    "serve",
                    "--isolated",
                    "--host",
                    "127.0.0.1",
                    "--port",
                    "8653",
                ],
            )
