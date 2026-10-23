"""The words the brand manual retired stay out of what Orbit says.

The manual's «Cómo habla Orbit» renamed them: «Privacidad detenida» and
«Rearmar» are «Todo está parado» and «Reactivar»; «Volver a transcribir» and
«Volver a escribir» are «Rehacer transcripción» and «Rehacer resumen». The
copy is in the interface's dictionary, its screens and the texts the daemon,
the helper and the hub hand to the person, so this reads the shipped sources
of all four (tests and generated code aside), line by line, and fails with
each place that still says one of them. Only the Spanish words count: `rearm`
stays the wire's and the code's name.
"""

import re
import sys
import unittest
from pathlib import Path

sys.dont_write_bytecode = True

ROOT = Path(__file__).resolve().parents[2]

RETIRED = re.compile(
    r"privacidad detenida"
    r"|\brearm(?:a|as|e|es|en|an|ar|arlo|arla|ada|ado|ó|ará)\b"
    r"|\bvolver a (?:transcribir|escribir)\b"
    r"|\bvolviendo a transcribir\b",
    re.IGNORECASE,
)

# Where each part keeps its copy.
SOURCES = (
    ("apps/desktop/src", (".ts", ".tsx")),
    ("apps/desktop/src-tauri/src", (".rs",)),
    ("crates", (".rs",)),
    ("native/macos/Sources", (".swift",)),
    ("server/orbit_hub", (".py",)),
    ("server/orbit_mcp", (".py",)),
)


def shipped(path: Path) -> bool:
    parts = path.relative_to(ROOT).parts
    name = path.name
    return not (
        "tests" in parts
        or "generated" in parts
        or "target" in parts
        or "node_modules" in parts
        or ".test." in name
        or name.endswith("_tests.rs")
        or name == "tests.rs"
        or name.startswith("test_")
    )


def retired_copy():
    found = []
    for directory, suffixes in SOURCES:
        for path in sorted((ROOT / directory).rglob("*")):
            if path.suffix not in suffixes or not path.is_file() or not shipped(path):
                continue
            lines = path.read_text(encoding="utf-8").splitlines()
            for number, line in enumerate(lines, start=1):
                if RETIRED.search(line):
                    found.append(f"{path.relative_to(ROOT)}:{number}: {line.strip()}")
    return found


class RetiredCopyTests(unittest.TestCase):
    def test_no_retired_word_is_left_in_what_the_user_sees(self):
        self.assertEqual(retired_copy(), [])

    def test_the_check_finds_each_retired_word_and_spares_identifiers(self):
        for copy in (
            'bail!("Privacidad detenida: Orbit no lee la agenda");',
            '"Conecta y rearma Orbit"',
            '"hasta que la rearmes"',
            '+ "Detén la captura antes de rearmarlo."',
            '"El rearme por pipe"',
            "retranscribeTitle: '¿Volver a transcribir la reunión?',",
            "<h2>¿Volver a escribir el resumen?</h2>",
            "Volviendo a transcribir la reunión…{\" \"}",
        ):
            self.assertTrue(RETIRED.search(copy), copy)
        for code in (
            'case .rearm: "rearm"',
            'code: "local_rearm_unavailable"',
            "fn rearm_without_stop()",
            "// Rearmed with Zoom still on the microphone",
        ):
            self.assertFalse(RETIRED.search(code), code)


if __name__ == "__main__":
    unittest.main()
