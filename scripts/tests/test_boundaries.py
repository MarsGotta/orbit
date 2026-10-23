"""The domain-boundary guard of scripts/check/boundaries.py, on small invented crates.

The real crates are checked by the gate's own step (`guards-boundaries`).
"""

import sys
import tempfile
import textwrap
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts" / "check"))

import boundaries

CORE = {
    "crates/core/Cargo.toml": "[package]\nname = 'core'\n",
    "crates/core/src/lib.rs": "mod transition;\nmod shared;\n",
    "crates/core/src/shared.rs": "pub struct Shared;\n",
    "crates/core/src/transition.rs": """
        mod alpha;
        mod beta;
        mod router;
        #[cfg(test)]
        mod checks;
        pub struct Step;
    """,
    "crates/core/src/transition/alpha.rs": """
        use super::{Step, beta};
        use crate::shared::Shared;

        pub(super) fn decide(step: &mut Step) {
            beta::ensure_ready();
            let _state = super::beta::BetaState::default();
            let _ = (step, Shared);
            // beta::secret() in a comment is not code.
            let _text = "beta::secret()";
        }

        #[cfg(test)]
        mod tests {
            use super::super::beta::secret;
        }
    """,
    "crates/core/src/transition/beta.rs": """
        #[derive(Default)]
        pub struct BetaState;
        pub(super) fn ensure_ready() {}
        pub(super) fn secret() {}
        mod inner;
    """,
    "crates/core/src/transition/beta/inner.rs": "pub(super) fn deep() {}\n",
    "crates/core/src/transition/router.rs": """
        use super::{alpha, beta};
        fn route() { beta::secret(); beta::inner::deep(); }
    """,
    "crates/core/src/transition/checks.rs": "use super::beta::secret;\n",
}

CORE_SETTINGS = {
    "crate": "crates/core",
    "parent": "transition",
    "coordinators": ["router"],
    "offers": {"beta": ["ensure_ready"]},
}

DAEMON = {
    "crates/daemon/Cargo.toml": "[package]\nname = 'daemon'\n",
    "crates/daemon/src/lib.rs": "mod daemon;\n",
    "crates/daemon/src/daemon.rs": "mod runtime;\nmod alpha;\nmod beta;\nmod kernel;\nuse runtime::*;\n",
    "crates/daemon/src/daemon/runtime.rs": """
        pub(super) struct Runtime {
            pub(super) state: u8,
            pub(super) first: alpha::AlphaHandle,
            pub(super) second: beta::BetaHandle,
        }
    """,
    "crates/daemon/src/daemon/alpha.rs": """
        use super::*;
        pub(super) struct AlphaHandle { pub(super) queue: Vec<u8> }
        impl Runtime {
            fn alpha_step(&mut self) {
                self.second.read();
                let _ = self.second.pending.len();
                let _ = self.first.queue.len();
                let _ = self.state;
            }
        }
        fn helper(runtime: &mut Runtime) {
            runtime.second.pending.clear();
            let _ = beta::BetaHandle::default();
        }
    """,
    "crates/daemon/src/daemon/beta.rs": """
        #[derive(Default)]
        pub(super) struct BetaHandle { pub(super) pending: Vec<u8> }
        impl BetaHandle { pub(super) fn read(&self) {} }
    """,
    "crates/daemon/src/daemon/kernel.rs": """
        use super::*;
        impl Runtime {
            fn route(&mut self) { self.second.pending.clear(); self.first.queue.clear(); }
        }
    """,
}

DAEMON_SETTINGS = {
    "crate": "crates/daemon",
    "parent": "daemon",
    "handles": "crates/daemon/src/daemon/runtime.rs",
}


def scan(files: dict[str, str], name: str, settings: dict):
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        for relative, text in files.items():
            path = root / relative
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(textwrap.dedent(text).lstrip("\n"), encoding="utf-8")
        area = boundaries.build_area(name, settings, root)
        return area, boundaries.scan(area, root)


class CoreTests(unittest.TestCase):
    def test_the_domains_are_the_children_of_the_parent_but_coordinators(self):
        area, _ = scan(CORE, "core", CORE_SETTINGS)
        self.assertEqual(area.domains, {"alpha", "beta"})

    def test_offered_functions_and_types_make_an_edge_and_no_reach(self):
        _, graph = scan(CORE, "core", CORE_SETTINGS)
        self.assertEqual(set(graph.edges), {("alpha", "beta")})
        self.assertEqual(graph.reaches, {})
        self.assertEqual(graph.edges[("alpha", "beta")], "crates/core/src/transition/alpha.rs:1")

    def test_a_function_not_offered_or_a_submodule_is_a_reach(self):
        files = dict(CORE)
        files["crates/core/src/transition/alpha.rs"] = """
            use super::beta::inner::deep;
            pub(super) fn decide() { super::beta::secret(); deep(); }
        """
        _, graph = scan(files, "core", CORE_SETTINGS)
        self.assertEqual(
            set(graph.reaches),
            {("alpha", "beta", "inner::deep"), ("alpha", "beta", "secret")},
        )

    def test_a_name_imported_from_outside_the_area_is_not_a_domain(self):
        files = dict(CORE)
        files["crates/core/src/transition/alpha.rs"] = """
            use other_crate::beta;
            pub(super) fn decide() { beta::secret(); }
        """
        _, graph = scan(files, "core", CORE_SETTINGS)
        self.assertEqual(graph.edges, {})


class DaemonTests(unittest.TestCase):
    def test_the_domains_are_the_modules_that_own_a_handle(self):
        area, _ = scan(DAEMON, "daemon", DAEMON_SETTINGS)
        self.assertEqual(area.handles, {"first": "alpha", "second": "beta"})
        self.assertEqual(area.domains, {"alpha", "beta"})

    def test_handle_methods_are_allowed_and_handle_fields_are_reaches(self):
        _, graph = scan(DAEMON, "daemon", DAEMON_SETTINGS)
        self.assertEqual(set(graph.edges), {("alpha", "beta")})
        self.assertEqual(set(graph.reaches), {("alpha", "beta", ".pending")})
        self.assertEqual(
            graph.reaches[("alpha", "beta", ".pending")], "crates/daemon/src/daemon/alpha.rs:6"
        )


class ProblemTests(unittest.TestCase):
    def setUp(self):
        self.area, self.graph = scan(DAEMON, "daemon", DAEMON_SETTINGS)

    def test_the_declared_graph_passes(self):
        settings = dict(DAEMON_SETTINGS, edges={"alpha": ["beta"]})
        settings["reaches"] = {"alpha -> beta": [".pending"]}
        self.assertEqual(boundaries.problems(self.area, self.graph, settings), [])

    def test_a_new_edge_and_a_new_reach_fail_naming_where(self):
        found = boundaries.problems(self.area, self.graph, DAEMON_SETTINGS)
        self.assertEqual(len(found), 2)
        self.assertIn(
            "new edge alpha -> beta (first at crates/daemon/src/daemon/alpha.rs:5)", found[0]
        )
        self.assertIn("alpha reaches into beta's internals with `.pending`", found[1])
        self.assertIn("takes no new entries", found[1])

    def test_entries_the_code_no_longer_has_fail(self):
        settings = dict(DAEMON_SETTINGS, edges={"alpha": ["beta"], "beta": ["alpha"]})
        settings["reaches"] = {"alpha -> beta": [".pending", ".gone"]}
        found = boundaries.problems(self.area, self.graph, settings)
        self.assertEqual(
            found,
            [
                (
                    "daemon: boundaries.toml allows beta -> alpha, but the code has no such edge "
                    "any more: remove it."
                ),
                (
                    "daemon: boundaries.toml lists the reach alpha -> beta `.gone`, which the code "
                    "no longer has: remove it."
                ),
            ],
        )

    def test_an_unknown_domain_fails(self):
        settings = dict(DAEMON_SETTINGS, edges={"alpha": ["beta"], "gamma": ["beta"]})
        settings["reaches"] = {"alpha -> beta": [".pending"]}
        found = boundaries.problems(self.area, self.graph, settings)
        self.assertIn("daemon: boundaries.toml names `gamma`, which is not a domain here.", found)


class UseTreeTests(unittest.TestCase):
    def test_groups_nest_and_renames_drop(self):
        self.assertEqual(
            boundaries.expand_use("super::{Step, beta::{secret as hidden, inner::deep}, *}"),
            ["super::Step", "super::beta::secret", "super::beta::inner::deep", "super::*"],
        )


if __name__ == "__main__":
    unittest.main()
