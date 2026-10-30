"""No two tracked files in one folder may differ only in letter case.

macOS builds Orbit on a case-insensitive filesystem; development and the gate
run on Linux, where case matters. A component `Markdown.tsx` next to a module
`markdown.ts` built and passed every test on Linux, and on the Mac
`import "./Markdown"` resolved to `markdown.ts`: the 0.4.0 release build stopped
at `pnpm build` (2026-10-15). An import names a module by its stem, so two
stems in one folder that match ignoring case are a collision even with
different extensions.
"""

import subprocess
import unittest
from collections import defaultdict
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]


def collisions(paths):
    """Groups of paths in one folder whose names, or whose stems, match ignoring case."""
    by_name = defaultdict(set)
    by_stem = defaultdict(set)
    for path in paths:
        folder, _, name = path.rpartition("/")
        by_name[(folder.lower(), name.lower())].add(path)
        by_stem[(folder.lower(), name.split(".")[0].lower())].add(path)
    found = []
    for group in list(by_name.values()) + list(by_stem.values()):
        names = {path.rpartition("/")[2] for path in group}
        stems = {name.split(".")[0] for name in names}
        if len(group) > 1 and (len(stems) > 1 or len(names) != len({n.lower() for n in names})):
            found.append(sorted(group))
    return sorted({tuple(group) for group in found})


class CaseCollisionTests(unittest.TestCase):
    def test_the_rule_finds_a_component_next_to_its_module(self):
        listing = [
            "apps/desktop/src/features/notes/Markdown.tsx",
            "apps/desktop/src/features/notes/markdown.ts",
            "apps/desktop/src/features/notes/markdown-doc.ts",
            "crates/orbit-core/src/lib.rs",
        ]
        self.assertEqual(
            collisions(listing),
            [
                (
                    "apps/desktop/src/features/notes/Markdown.tsx",
                    "apps/desktop/src/features/notes/markdown.ts",
                )
            ],
        )

    def test_the_rule_leaves_a_module_and_its_test_alone(self):
        listing = ["src/a/markdown.ts", "src/a/markdown.test.ts", "src/b/Markdown.tsx"]
        self.assertEqual(collisions(listing), [])

    def test_no_tracked_files_collide_on_a_case_insensitive_mac(self):
        listing = (
            subprocess.run(
                ["git", "-C", str(ROOT), "ls-files", "-z"],
                check=True,
                capture_output=True,
            )
            .stdout.decode()
            .split("\0")
        )
        found = collisions([path for path in listing if path])
        self.assertEqual(
            found,
            [],
            "these files resolve to the same module on macOS; rename one: "
            + "; ".join(", ".join(group) for group in found),
        )


if __name__ == "__main__":
    unittest.main()
