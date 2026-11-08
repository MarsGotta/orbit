"""The layer guard of scripts/check/layers.py, on small invented workspaces.

Each test writes a Cargo workspace (manifests, Rust sources and its own
scripts/check/layers.toml) to a temporary directory and runs the guard on it
by its command line, `python3 -B scripts/check/layers.py ROOT`, the way the
gate runs it on the repository with no argument. The guard reads the
workspace with `cargo metadata --no-deps --offline`, so these tests need
cargo, like the rest of the guards zone.

Every problem the guard finds is one line that starts with its cause and
names the package or the file (with its line) and the numbers.
"""

import copy
import json
import re
import subprocess
import sys
import tempfile
import textwrap
import unittest
from pathlib import Path

import tomllib

ROOT = Path(__file__).resolve().parents[2]
GUARD = ROOT / "scripts" / "check" / "layers.py"
TIMEOUT = 180

# The invented workspace: `kernel` is the pure core; `relay` and `pilot` are
# the engine, and `pilot` holds the coordinator `Runtime` over two domains,
# `alpha` and `beta`; `gadget` is a Mac client, so what the engine may not do
# is its job; `shell` is the app client. `shell`'s dev and build dependencies
# are not in its rule: only normal dependencies count.
DEPENDENCIES = {
    "kernel": {},
    "relay": {"dependencies": ["kernel"]},
    "pilot": {"dependencies": ["kernel", "relay"]},
    "gadget": {"dependencies": ["kernel"]},
    "shell": {
        "dependencies": ["kernel", "pilot"],
        "dev-dependencies": ["relay"],
        "build-dependencies": ["gadget"],
    },
}

SOURCES = {
    "crates/kernel/src/lib.rs": "pub struct Fact;\n",
    "crates/relay/src/lib.rs": """
        //! An engine package with no process, macOS or keychain code.
        #[cfg(target_os = "linux")]
        pub fn on_linux() {}

        pub fn hop() {}
    """,
    "crates/gadget/src/lib.rs": """
        //! A Mac client package: launching, macOS and the keychain are its job.
        use objc2_app_kit::NSWorkspace;

        #[cfg(target_os = "macos")]
        pub fn open() {
            let _ = std::process::Command::new("open");
            let _ = keyring::Entry::new("example-service", "example-user");
        }
    """,
    "crates/shell/src/lib.rs": "pub fn show() {}\n",
    "crates/pilot/src/lib.rs": """
        //! An engine with a coordinator over two domains.
        pub mod daemon;
        mod transport;
    """,
    "crates/pilot/src/transport.rs": """
        //! The one engine file layers.toml lets launch a process, branch on
        //! macOS and use the keychain crate.
        pub fn launch() {
            let _ = std::process::Command::new("helper-tool");
        }

        #[cfg(target_os = "macos")]
        pub fn secret() {
            let _ = keyring::Entry::new("example-service", "example-user");
        }
    """,
    "crates/pilot/src/daemon.rs": """
        mod alpha;
        mod beta;
        mod runtime;
        mod ticks;
        #[cfg(test)]
        mod checks;

        pub use runtime::Runtime;
    """,
    "crates/pilot/src/daemon/runtime.rs": """
        use super::*;

        pub struct Runtime {
            pub(super) state: u8,
            pub(super) first: alpha::AlphaHandle,
            pub(super) second: beta::BetaHandle,
        }
    """,
    "crates/pilot/src/daemon/alpha.rs": """
        use super::*;

        mod deep;

        pub(super) struct AlphaHandle;

        impl AlphaHandle {
            fn read(&self) {}
        }

        impl Runtime {
            fn alpha_one(&mut self) {}

            fn alpha_two(&mut self) {
                fn nested() {}
                nested();
            }
        }

        // Command::new, keyring and cfg(target_os = "macos") in a comment are not code.
        fn quiet() -> &'static str {
            "std::process::Command::new(keyring) target_os = macos"
        }

        #[cfg(test)]
        mod tests {
            use super::*;

            impl Runtime {
                fn only_in_tests(&self) {}
            }

            #[cfg(target_os = "macos")]
            #[test]
            fn launches() {
                let _ = std::process::Command::new("fake-helper");
            }
        }
    """,
    "crates/pilot/src/daemon/alpha/deep.rs": """
        use super::*;

        impl Runtime {
            fn alpha_deep(&self) {}
        }
    """,
    "crates/pilot/src/daemon/beta.rs": """
        use super::*;

        pub(super) struct BetaHandle;

        impl Runtime {
            fn beta_one(&self) {}
        }
    """,
    "crates/pilot/src/daemon/ticks.rs": """
        use super::*;

        pub(super) struct CallRuntime;

        impl Runtime {
            pub fn tick(&mut self) {}

            fn tock(&self) {}
        }

        impl CallRuntime {
            fn not_the_runtime(&self) {}
        }

        impl Default for Runtime {
            fn default() -> Self {
                unimplemented!()
            }
        }
    """,
    "crates/pilot/src/daemon/checks.rs": """
        use super::*;

        impl Runtime {
            fn fake_step(&self) {}
        }
    """,
}

# `impl Runtime` has 6 methods outside tests: alpha_one, alpha_two,
# alpha_deep and beta_one in the domain folders, tick and tock in the
# coordinator.
RULES = {
    "packages": {
        "kernel": [],
        "relay": ["kernel"],
        "pilot": ["kernel", "relay"],
        "gadget": ["kernel"],
        "shell": ["kernel", "pilot"],
    },
    "engine": {
        "packages": ["pilot", "relay"],
        "command_new": ["crates/pilot/src/transport.rs"],
        "cfg_macos": ["crates/pilot/src/transport.rs"],
        "keyring": ["crates/pilot/src/transport.rs"],
        "appkit": [],
        "unix_signals": [],
        "forbidden_crates": ["keyring", "objc2-app-kit", "cocoa"],
    },
    "runtime": {"package": "pilot", "methods": 6, "in_domains": 4},
}


def manifest(name: str, tables: dict[str, list[str]]) -> str:
    lines = ["[package]", f'name = "{name}"', 'version = "0.0.0"', 'edition = "2021"', "publish = false"]
    for table, names in tables.items():
        lines.append(f"[{table}]")
        lines.extend(f'{dependency} = {{ path = "../{dependency}" }}' for dependency in names)
    return "\n".join(lines) + "\n"


def render_rules(rules: dict) -> str:
    lines = ["[packages]"]
    lines.extend(f"{name} = {json.dumps(uses)}" for name, uses in rules["packages"].items())
    if "optional" in rules:
        lines += ["", "[optional]"]
        lines.extend(f"{name} = {json.dumps(uses)}" for name, uses in rules["optional"].items())
    if "launch_only_in" in rules:
        lines += ["", "[launch_only_in]"]
        lines.extend(
            f"{name} = {json.dumps(uses)}"
            for name, uses in rules["launch_only_in"].items()
        )
    lines += ["", "[engine]"]
    lines.extend(f"{key} = {json.dumps(value)}" for key, value in rules["engine"].items())
    runtime = rules["runtime"]
    lines += [
        "",
        "[runtime]",
        f"package = {json.dumps(runtime['package'])}",
        f"methods = {runtime['methods']}",
        f"in_domains = {runtime['in_domains']}",
    ]
    return "\n".join(lines) + "\n"


def line_number(text: str, needle: str, occurrence: int = 1) -> int:
    """The 1-based line of the `occurrence`-th `needle` in `text`."""
    index = -1
    for _ in range(occurrence):
        index = text.index(needle, index + 1)
    return text.count("\n", 0, index) + 1


class Workspace(unittest.TestCase):
    """A fresh invented workspace per test, with its rules in RULES."""

    def setUp(self) -> None:
        directory = tempfile.TemporaryDirectory(prefix="orbit-layers-")
        self.addCleanup(directory.cleanup)
        self.root = Path(directory.name)
        self.rules = copy.deepcopy(RULES)
        self.dependencies = copy.deepcopy(DEPENDENCIES)
        self.write("Cargo.toml", '[workspace]\nmembers = ["crates/*"]\nresolver = "2"\n')
        for path, text in SOURCES.items():
            self.write(path, text)
        self.write_manifests()
        self.write_rules()

    def write(self, relative: str, text: str) -> str:
        text = textwrap.dedent(text).lstrip("\n")
        path = self.root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text, encoding="utf-8")
        return text

    def append(self, relative: str, text: str) -> str:
        """Adds `text` at the end of a file and returns the whole file."""
        path = self.root / relative
        whole = path.read_text(encoding="utf-8") + "\n" + textwrap.dedent(text).lstrip("\n")
        path.write_text(whole, encoding="utf-8")
        return whole

    def write_manifests(self) -> None:
        for name, tables in self.dependencies.items():
            self.write(f"crates/{name}/Cargo.toml", manifest(name, tables))
            if not (self.root / f"crates/{name}/src/lib.rs").exists():
                self.write(f"crates/{name}/src/lib.rs", "pub fn exists() {}\n")

    def write_rules(self) -> None:
        self.write("scripts/check/layers.toml", render_rules(self.rules))

    def run_guard(self) -> subprocess.CompletedProcess:
        return subprocess.run(
            [sys.executable, "-B", str(GUARD), str(self.root)],
            check=False,
            capture_output=True,
            text=True,
            timeout=TIMEOUT,
        )

    def assert_passes(self) -> None:
        result = self.run_guard()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def problem_lines(self, result: subprocess.CompletedProcess, cause: str) -> list[str]:
        output = result.stdout + result.stderr
        return [line for line in output.splitlines() if re.search(rf"\b{re.escape(cause)}\b", line)]

    def assert_problem(self, result: subprocess.CompletedProcess, cause: str, *facts) -> None:
        """Exit 1, and one line with `cause` carries every fact: text, or a number."""
        output = result.stdout + result.stderr
        self.assertEqual(result.returncode, 1, output)
        lines = self.problem_lines(result, cause)
        self.assertTrue(lines, f"no line names the cause {cause}:\n{output}")

        def carries(line: str) -> bool:
            for fact in facts:
                if isinstance(fact, int):
                    if not re.search(rf"(?<!\d){fact}(?!\d)", line):
                        return False
                elif fact not in line:
                    return False
            return True

        self.assertTrue(
            any(carries(line) for line in lines),
            f"no {cause} line carries {facts}:\n{output}",
        )

    def assert_no_problem(self, result: subprocess.CompletedProcess, cause: str) -> None:
        self.assertEqual(self.problem_lines(result, cause), [], result.stdout + result.stderr)


class TheRealTree(unittest.TestCase):
    def test_the_guard_passes_on_the_repository_as_it_is(self) -> None:
        """The photo: layers.toml holds exactly what the repository has today."""
        result = subprocess.run(
            [sys.executable, "-B", str(GUARD)],
            check=False,
            cwd=ROOT,
            capture_output=True,
            text=True,
            timeout=TIMEOUT,
        )
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_the_gate_runs_the_guard_in_the_guards_zone(self) -> None:
        for arguments in (["--plan"], ["--plan", "guards"]):
            result = subprocess.run(
                ["bash", "scripts/check.sh", *arguments],
                check=False,
                cwd=ROOT,
                capture_output=True,
                text=True,
                timeout=TIMEOUT,
            )
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            steps = {line.split("\t")[0]: line for line in result.stdout.splitlines()}
            self.assertIn("guards-layers", steps, result.stdout)
            self.assertIn("scripts/check/layers.py", steps["guards-layers"])

    def rules(self) -> dict:
        return tomllib.loads(
            (ROOT / "scripts" / "check" / "layers.toml").read_text(encoding="utf-8")
        )

    def test_the_engine_takes_the_control_client_only_through_its_feature(self) -> None:
        optional = self.rules()["optional"].get("orbit-daemon", [])
        self.assertIn("orbit-control", optional)
        self.assertIn("orbit-mac", optional)

    def test_the_app_core_launches_processes_only_in_its_updater(self) -> None:
        self.assertEqual(
            sorted(self.rules().get("launch_only_in", {}).get("orbit-app-core", [])),
            [
                "crates/orbit-app-core/src/update/installer.rs",
                "crates/orbit-app-core/src/update/retirement.rs",
            ],
        )


class Packages(Workspace):
    def test_a_workspace_that_matches_its_rules_passes(self) -> None:
        """Dev and build dependencies, and the non-engine `gadget`, are not restricted."""
        self.assert_passes()

    def test_a_dependency_the_rule_does_not_allow_fails(self) -> None:
        self.dependencies["relay"]["dependencies"].append("gadget")
        self.write_manifests()
        result = self.run_guard()
        self.assert_problem(result, "dependency_not_allowed", "relay", "gadget")

    def test_a_dependency_the_code_no_longer_has_fails(self) -> None:
        self.dependencies["pilot"]["dependencies"].remove("relay")
        self.write_manifests()
        result = self.run_guard()
        self.assert_problem(result, "dependency_no_longer_used", "pilot", "relay")

    def test_a_rule_for_a_package_that_does_not_exist_fails(self) -> None:
        self.rules["packages"]["phantom"] = ["kernel"]
        self.write_rules()
        self.assert_problem(self.run_guard(), "rule_without_package", "phantom")

    def test_a_new_package_takes_its_rule_from_layers_toml_alone(self) -> None:
        self.dependencies["hub-link"] = {"dependencies": ["kernel"]}
        self.write_manifests()
        self.assert_problem(self.run_guard(), "package_without_rule", "hub-link")

        self.rules["packages"]["hub-link"] = ["kernel"]
        self.write_rules()
        self.assert_passes()

        self.dependencies["pilot"]["dependencies"].append("hub-link")
        self.write_manifests()
        self.assert_problem(self.run_guard(), "dependency_not_allowed", "pilot", "hub-link")
        self.rules["packages"]["pilot"].append("hub-link")
        self.write_rules()
        self.assert_passes()

        self.dependencies["hub-link"]["dependencies"].append("relay")
        self.write_manifests()
        self.assert_problem(self.run_guard(), "dependency_not_allowed", "hub-link", "relay")


class OptionalDependencies(Workspace):
    """[optional]: the Mac package only reaches the engine behind a feature."""

    def make_relay_optional_in_pilot(self) -> None:
        self.write(
            "crates/pilot/Cargo.toml",
            """
            [package]
            name = "pilot"
            version = "0.0.0"
            edition = "2021"
            publish = false

            [dependencies]
            kernel = { path = "../kernel" }
            relay = { path = "../relay", optional = true }

            [features]
            default = ["mac"]
            mac = ["dep:relay"]
            """,
        )

    def test_a_dependency_listed_optional_that_is_optional_passes(self) -> None:
        self.make_relay_optional_in_pilot()
        self.rules["optional"] = {"pilot": ["relay"]}
        self.write_rules()
        self.assert_passes()

    def test_a_dependency_listed_optional_that_is_required_fails(self) -> None:
        self.rules["optional"] = {"pilot": ["relay"]}
        self.write_rules()
        self.assert_problem(self.run_guard(), "dependency_must_be_optional", "pilot", "relay")

    def test_a_dependency_listed_optional_outside_its_rule_fails(self) -> None:
        self.rules["optional"] = {"pilot": ["gadget"]}
        self.write_rules()
        self.assert_problem(self.run_guard(), "dependency_must_be_optional", "pilot", "gadget")

    def test_a_dependency_listed_optional_that_is_gone_fails(self) -> None:
        self.dependencies["pilot"]["dependencies"].remove("relay")
        self.write_manifests()
        self.rules["optional"] = {"pilot": ["relay"]}
        self.write_rules()
        self.assert_problem(self.run_guard(), "dependency_no_longer_used", "pilot", "relay", "[optional]")

    def test_an_optional_line_for_a_package_that_does_not_exist_fails(self) -> None:
        self.rules["optional"] = {"phantom": ["kernel"]}
        self.write_rules()
        self.assert_problem(self.run_guard(), "rule_without_package", "phantom")

    def test_rules_without_the_optional_table_pass(self) -> None:
        self.assertNotIn("optional", self.rules)
        self.assert_passes()


class TheEngine(Workspace):
    def test_command_new_in_an_engine_file_outside_the_list_fails(self) -> None:
        text = self.append(
            "crates/pilot/src/daemon/ticks.rs",
            """
            fn launch_tool() {
                let _ = std::process::Command::new("tool");
                let _ = tokio::process::Command::new("other-tool");
            }
            """,
        )
        line = line_number(text, "Command::new")
        result = self.run_guard()
        self.assert_problem(
            result,
            "engine_forbidden_use",
            f"crates/pilot/src/daemon/ticks.rs:{line}",
            "command_new",
            "pilot",
            2,
        )

    def test_target_os_macos_in_an_engine_file_outside_the_list_fails(self) -> None:
        relay = self.append(
            "crates/relay/src/lib.rs",
            """
            #[cfg(target_os = "macos")]
            pub fn on_mac() {}
            """,
        )
        beta = self.append(
            "crates/pilot/src/daemon/beta.rs",
            """
            fn elsewhere() -> bool {
                cfg!(not(target_os = "macos"))
            }
            """,
        )
        result = self.run_guard()
        relay_line = line_number(relay, 'target_os = "macos"')
        beta_line = line_number(beta, 'target_os = "macos"')
        self.assert_problem(
            result, "engine_forbidden_use", f"crates/relay/src/lib.rs:{relay_line}", "cfg_macos", "relay"
        )
        self.assert_problem(
            result,
            "engine_forbidden_use",
            f"crates/pilot/src/daemon/beta.rs:{beta_line}",
            "cfg_macos",
            "pilot",
        )

    def test_an_allowance_the_file_no_longer_needs_fails(self) -> None:
        self.write(
            "crates/pilot/src/transport.rs",
            """
            pub fn launch() {
                let _ = std::process::Command::new("helper-tool");
            }

            #[cfg(target_os = "macos")]
            pub fn secret() {}
            """,
        )
        result = self.run_guard()
        self.assert_problem(
            result, "engine_allowance_unused", "crates/pilot/src/transport.rs", "keyring"
        )
        self.assert_no_problem(result, "engine_forbidden_use")


    def test_launching_a_process_through_an_import_alias_fails(self) -> None:
        aliased = self.append(
            "crates/pilot/src/daemon/ticks.rs",
            """
            use tokio::process::Command as Run;

            fn go() {
                let _ = Run::new("tool");
            }
            """,
        )
        grouped = self.append("crates/pilot/src/daemon/beta.rs", "use std::process::{Stdio, Command as Launch};\n")
        wildcard = self.append("crates/relay/src/lib.rs", "use std::process::*;\n")
        result = self.run_guard()
        for path, text, needle, package in (
            ("crates/pilot/src/daemon/ticks.rs", aliased, "process::Command", "pilot"),
            ("crates/pilot/src/daemon/beta.rs", grouped, "process::{", "pilot"),
            ("crates/relay/src/lib.rs", wildcard, "process::*", "relay"),
        ):
            with self.subTest(path=path):
                line = line_number(text, needle)
                self.assert_problem(result, "engine_forbidden_use", f"{path}:{line}", "command_new", package)

    def test_a_process_type_that_launches_nothing_passes(self) -> None:
        self.append("crates/pilot/src/daemon/ticks.rs", "use std::process::ExitStatus;\n")
        self.assert_passes()

    def test_every_apple_platform_condition_counts_as_macos(self) -> None:
        vendor = self.append(
            "crates/pilot/src/daemon/beta.rs",
            """
            #[cfg(target_vendor = "apple")]
            fn on_apple() {}
            """,
        )
        ios = self.append(
            "crates/relay/src/lib.rs",
            """
            #[cfg(target_os = "ios")]
            fn on_ios() {}
            """,
        )
        result = self.run_guard()
        vendor_line = line_number(vendor, "target_vendor")
        ios_line = line_number(ios, 'target_os = "ios"')
        self.assert_problem(
            result, "engine_forbidden_use", f"crates/pilot/src/daemon/beta.rs:{vendor_line}", "cfg_macos", "pilot"
        )
        self.assert_problem(result, "engine_forbidden_use", f"crates/relay/src/lib.rs:{ios_line}", "cfg_macos", "relay")

    def test_unix_signals_in_an_engine_file_fail(self) -> None:
        unix = self.append("crates/pilot/src/daemon/ticks.rs", "use tokio::signal::unix::{signal, SignalKind};\n")
        wildcard = self.append("crates/pilot/src/daemon/beta.rs", "use tokio::signal::*;\n")
        ctrl_c = self.append(
            "crates/relay/src/lib.rs",
            """
            pub async fn wait() {
                let _ = tokio::signal::ctrl_c().await;
            }
            """,
        )
        kind = self.append(
            "crates/pilot/src/daemon/alpha/deep.rs",
            """
            fn which(kind: SignalKind) -> SignalKind {
                kind
            }
            """,
        )
        result = self.run_guard()
        for path, text, needle, package in (
            ("crates/pilot/src/daemon/ticks.rs", unix, "signal::unix", "pilot"),
            ("crates/pilot/src/daemon/beta.rs", wildcard, "signal::*", "pilot"),
            ("crates/relay/src/lib.rs", ctrl_c, "signal::ctrl_c", "relay"),
            ("crates/pilot/src/daemon/alpha/deep.rs", kind, "SignalKind", "pilot"),
        ):
            with self.subTest(path=path):
                line = line_number(text, needle)
                self.assert_problem(result, "engine_forbidden_use", f"{path}:{line}", "unix_signals", package)

    def test_signals_in_a_comment_or_a_client_package_pass(self) -> None:
        self.append("crates/pilot/src/daemon/ticks.rs", "// tokio::signal::unix and SignalKind are a client's job.\n")
        self.append("crates/gadget/src/lib.rs", "use tokio::signal::unix::{signal, SignalKind};\n")
        self.assert_passes()

    def test_a_forbidden_crate_under_another_name_fails(self) -> None:
        self.append("crates/pilot/Cargo.toml", 'kr = { package = "keyring", version = "3" }\n')
        self.assert_problem(self.run_guard(), "engine_forbidden_crate", "pilot", "keyring", "kr")

    def test_a_forbidden_crate_for_one_platform_or_for_tests_fails(self) -> None:
        self.append(
            "crates/relay/Cargo.toml",
            """
            [target.'cfg(target_os = "macos")'.dependencies]
            cocoa = "0.26"

            [dev-dependencies]
            objc2-app-kit = "0.3"
            """,
        )
        result = self.run_guard()
        self.assert_problem(result, "engine_forbidden_crate", "relay", "cocoa")
        self.assert_problem(result, "engine_forbidden_crate", "relay", "objc2-app-kit")

    def test_a_forbidden_crate_in_a_client_package_passes(self) -> None:
        self.append("crates/gadget/Cargo.toml", 'keyring = "3"\n')
        self.assert_passes()

    def test_rules_without_a_key_of_the_engine_cannot_be_checked(self) -> None:
        for key in ("unix_signals", "forbidden_crates"):
            with self.subTest(key=key):
                rules = copy.deepcopy(self.rules)
                del rules["engine"][key]
                self.write("scripts/check/layers.toml", render_rules(rules))
                result = self.run_guard()
                self.assertEqual(result.returncode, 2, result.stdout + result.stderr)
                self.assertIn(f"[engine].{key}", result.stderr)


class BadlyTypedRules(Workspace):
    """A layers.toml of the wrong shape cannot be checked: exit 2, its cause, no trace."""

    def assert_cannot_check(self, *facts: str) -> None:
        result = self.run_guard()
        output = result.stdout + result.stderr
        self.assertEqual(result.returncode, 2, output)
        self.assertNotIn("Traceback", output)
        for fact in facts:
            self.assertIn(fact, result.stderr)

    def write_top_level(self, line: str) -> None:
        """A key before the first table, where a table of that name would be."""
        self.write("scripts/check/layers.toml", f"{line}\n{render_rules(self.rules)}")

    def test_forbidden_crates_as_a_string_is_not_a_list_of_letters(self) -> None:
        """It used to compare the crate names with k, e, y, r... and pass with the real keyring."""
        self.append("crates/pilot/Cargo.toml", 'keyring = "3"\n')
        self.rules["engine"]["forbidden_crates"] = "keyring"
        self.write_rules()
        self.assert_cannot_check("[engine].forbidden_crates", "list of strings")

    def test_forbidden_crates_as_a_number_cannot_be_checked(self) -> None:
        self.rules["engine"]["forbidden_crates"] = 3
        self.write_rules()
        self.assert_cannot_check("[engine].forbidden_crates", "list of strings")

    def test_forbidden_crates_with_a_number_inside_cannot_be_checked(self) -> None:
        self.rules["engine"]["forbidden_crates"] = ["keyring", 3]
        self.write_rules()
        self.assert_cannot_check("[engine].forbidden_crates", "list of strings")

    def test_optional_as_a_number_cannot_be_checked(self) -> None:
        self.write_top_level("optional = 3")
        self.assert_cannot_check("[optional]", "table")

    def test_optional_as_a_list_cannot_be_checked(self) -> None:
        self.write_top_level('optional = ["relay"]')
        self.assert_cannot_check("[optional]", "table")

    def test_a_line_of_optional_that_is_a_number_cannot_be_checked(self) -> None:
        self.rules["optional"] = {"motor": 3}
        self.write_rules()
        self.assert_cannot_check("[optional].motor", "list of strings")

    def test_a_line_of_packages_that_is_a_number_cannot_be_checked(self) -> None:
        self.rules["packages"]["motor"] = 3
        self.write_rules()
        self.assert_cannot_check("[packages].motor", "list of strings")

    def test_a_table_that_is_a_number_cannot_be_checked(self) -> None:
        for table in ("packages", "engine", "runtime"):
            with self.subTest(table=table):
                self.write("scripts/check/layers.toml", f"{table} = 3\n")
                self.assert_cannot_check(f"[{table}]", "table")

    def test_an_engine_list_that_is_a_string_cannot_be_checked(self) -> None:
        for key in ("packages", "command_new", "cfg_macos", "keyring", "appkit", "unix_signals"):
            with self.subTest(key=key):
                rules = copy.deepcopy(self.rules)
                rules["engine"][key] = "crates/pilot/src/transport.rs"
                self.write("scripts/check/layers.toml", render_rules(rules))
                self.assert_cannot_check(f"[engine].{key}", "list of strings")

    def test_a_runtime_package_that_is_a_number_cannot_be_checked(self) -> None:
        self.rules["runtime"]["package"] = 3
        self.write_rules()
        self.assert_cannot_check("[runtime].package", "string")

    def test_a_runtime_count_that_is_no_number_cannot_be_checked(self) -> None:
        for key, value in (("methods", '"6"'), ("in_domains", "true")):
            with self.subTest(key=key):
                rules = copy.deepcopy(self.rules)
                rules["runtime"][key] = value
                self.write("scripts/check/layers.toml", render_rules(rules))
                self.assert_cannot_check(f"[runtime].{key}", "whole number")


class LaunchOnlyIn(Workspace):
    """[launch_only_in]: a client package launches processes only in the files it names."""

    def with_a_launcher_in_shell(self) -> None:
        self.write(
            "crates/shell/src/lib.rs", "pub fn show() {}\nmod update;\nmod other;\n"
        )
        self.write(
            "crates/shell/src/update.rs",
            """
            pub fn install() {
                let _ = std::process::Command::new("tool");
            }
            """,
        )
        self.write("crates/shell/src/other.rs", "pub fn quiet() {}\n")
        self.rules["launch_only_in"] = {"shell": ["crates/shell/src/update.rs"]}
        self.write_rules()

    def test_a_launch_in_the_named_file_passes(self) -> None:
        self.with_a_launcher_in_shell()
        self.assert_passes()

    def test_a_launch_outside_the_named_files_fails(self) -> None:
        self.with_a_launcher_in_shell()
        text = self.write(
            "crates/shell/src/other.rs",
            """
            pub fn quiet() {
                let _ = std::process::Command::new("tool");
            }
            """,
        )
        self.assert_problem(
            self.run_guard(),
            "launch_outside_allowed_files",
            "crates/shell/src/other.rs",
            line_number(text, "Command::new"),
            "shell",
        )

    def test_a_launch_in_a_test_passes(self) -> None:
        self.with_a_launcher_in_shell()
        self.write(
            "crates/shell/src/other.rs",
            """
            pub fn quiet() {}

            #[cfg(test)]
            mod tests {
                #[test]
                fn launches_in_tests_only() {
                    let _ = std::process::Command::new("tool");
                }
            }
            """,
        )
        self.assert_passes()

    def test_an_allowance_the_file_no_longer_needs_fails(self) -> None:
        self.with_a_launcher_in_shell()
        self.write("crates/shell/src/update.rs", "pub fn install() {}\n")
        self.assert_problem(
            self.run_guard(), "launch_allowance_unused", "crates/shell/src/update.rs"
        )

    def test_a_line_for_a_package_that_does_not_exist_fails(self) -> None:
        self.rules["launch_only_in"] = {"phantom": ["crates/phantom/src/lib.rs"]}
        self.write_rules()
        self.assert_problem(self.run_guard(), "rule_without_package", "phantom")

    def test_rules_without_the_table_pass(self) -> None:
        self.assertNotIn("launch_only_in", self.rules)
        self.assert_passes()

    def test_a_line_that_is_not_a_list_cannot_be_checked(self) -> None:
        self.write(
            "scripts/check/layers.toml",
            render_rules(self.rules) + "\n[launch_only_in]\nshell = 3\n",
        )
        result = self.run_guard()
        self.assertEqual(result.returncode, 2, result.stdout + result.stderr)


class TheRuntimeRatchet(Workspace):
    def test_a_new_method_in_the_coordinator_fails(self) -> None:
        self.append(
            "crates/pilot/src/daemon/ticks.rs",
            """
            impl Runtime {
                fn third(&self) {}
            }
            """,
        )
        result = self.run_guard()
        self.assert_problem(result, "runtime_methods_grew", "pilot", 7, 6)
        self.assert_no_problem(result, "runtime_domain_methods_grew")

    def test_a_method_moved_into_a_domain_folder_fails(self) -> None:
        self.write(
            "crates/pilot/src/daemon/ticks.rs",
            """
            use super::*;

            impl Runtime {
                pub fn tick(&mut self) {}
            }
            """,
        )
        self.append(
            "crates/pilot/src/daemon/beta.rs",
            """
            impl Runtime {
                fn tock(&self) {}
            }
            """,
        )
        result = self.run_guard()
        self.assert_problem(result, "runtime_domain_methods_grew", "pilot", 5, 4)
        self.assert_no_problem(result, "runtime_methods_grew")

    def test_fewer_methods_ask_for_the_recorded_numbers_to_fall(self) -> None:
        self.write(
            "crates/pilot/src/daemon/alpha.rs",
            """
            use super::*;

            mod deep;

            pub(super) struct AlphaHandle;

            impl Runtime {
                fn alpha_one(&mut self) {}
            }
            """,
        )
        result = self.run_guard()
        self.assert_problem(result, "runtime_methods_shrank", "pilot", 5, 6)
        self.assert_problem(result, "runtime_domain_methods_shrank", "pilot", 3, 4)

        self.rules["runtime"].update(methods=5, in_domains=3)
        self.write_rules()
        self.assert_passes()


if __name__ == "__main__":
    unittest.main()
