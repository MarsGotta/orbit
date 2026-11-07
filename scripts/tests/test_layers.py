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
                cwd=ROOT,
                capture_output=True,
                text=True,
                timeout=TIMEOUT,
            )
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            steps = {line.split("\t")[0]: line for line in result.stdout.splitlines()}
            self.assertIn("guards-layers", steps, result.stdout)
            self.assertIn("scripts/check/layers.py", steps["guards-layers"])


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
