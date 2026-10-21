"""Copies of shared bounds, located and measured by contracts/constants.json."""

import ast
import json
import operator
import re
import unittest
from fractions import Fraction
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
TIME_UNITS = {"seconds": 1_000_000_000, "milliseconds": 1_000_000, "nanoseconds": 1}
OPERATIONS = {
    ast.Add: operator.add,
    ast.Mult: operator.mul,
    ast.Div: operator.floordiv,
}


def read_constant(source, name):
    """Read string/integer declarations and arithmetic without executing source code."""

    def number(symbol, resolving):
        if symbol in resolving:
            raise ValueError(f"circular reference: {symbol}")
        declaration = re.search(
            rf"(?m)^[ \t]*(?:(?:pub(?:\([^)]*\))?|public|static|export|const|let)\s+)*"
            rf"{re.escape(symbol)}(?:\s*:[^=\n]+)?\s*=\s*([^;\n#]+)",
            source,
        )
        # The updater's regression test pins the ceiling in assert_eq! rather
        # than declaring another constant. It is a copy of the same number.
        assertion = re.search(
            rf"assert_eq!\(\s*{re.escape(symbol)}\s*,\s*([\d_]+)\s*,", source
        )
        match = declaration or assertion
        if match is None:
            raise ValueError(f"the definition of {symbol} is missing")
        return evaluate(
            ast.parse(match.group(1).strip(), mode="eval").body, (*resolving, symbol)
        )

    def evaluate(node, resolving):
        if isinstance(node, ast.Constant) and type(node.value) in (int, str):
            return node.value
        if isinstance(node, ast.Name):
            return number(node.id, resolving)
        if isinstance(node, ast.BinOp) and type(node.op) in OPERATIONS:
            return OPERATIONS[type(node.op)](
                evaluate(node.left, resolving), evaluate(node.right, resolving)
            )
        raise ValueError(f"constant expression not supported: {ast.dump(node)}")

    return number(name, ())


class ConstantsCopies(unittest.TestCase):
    def test_registered_copies_match(self):
        entries = json.loads((ROOT / "contracts/constants.json").read_text())
        for entry in entries:
            for copy in entry["copies"]:
                relative = copy["file"]
                # Python server modules are imported by the hub's guard in its
                # own venv. Only the standalone Mac harness is read as data here.
                if relative.startswith("server/"):
                    continue
                with self.subTest(file=relative, name=copy["name"]):
                    actual = read_constant((ROOT / relative).read_text(), copy["name"])
                    if copy["unit"] == entry["unit"]:
                        expected = entry["value"]
                    else:
                        expected = Fraction(
                            entry["value"] * TIME_UNITS[entry["unit"]],
                            TIME_UNITS[copy["unit"]],
                        )
                    self.assertEqual(
                        actual,
                        expected,
                        f"{relative}:{copy['name']}={actual} {copy['unit']}; "
                        f"expected {expected} {copy['unit']} ({entry['name']})",
                    )


if __name__ == "__main__":
    unittest.main()
