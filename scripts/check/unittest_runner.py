#!/usr/bin/env python3
"""Runs the guards exactly like ``python3 -B -m unittest discover -s DIR -p 'test_*.py'``
and also writes, as JSON, what did not pass as a pass: skipped tests and
expected failures. ``skips.py`` compares that list with the allowed skips.

Usage: python3 -B unittest_runner.py START_DIR REPORT.json
"""

import json
import sys
import unittest
from pathlib import Path


def main(arguments: list[str]) -> int:
    if len(arguments) != 2:
        print(__doc__, file=sys.stderr)
        return 2
    start, report = arguments
    sys.dont_write_bytecode = True
    # `python -m unittest` puts the working directory first on the path.
    sys.path[0] = ""
    suite = unittest.TestLoader().discover(start, pattern="test_*.py")
    result = unittest.TextTestRunner().run(suite)
    Path(report).write_text(
        json.dumps(
            {
                "tests_run": result.testsRun,
                "skipped": [[test.id(), reason] for test, reason in result.skipped],
                "expected_failures": [[test.id(), ""] for test, _ in result.expectedFailures],
                "unexpected_successes": [test.id() for test in result.unexpectedSuccesses],
                "failures": [[test.id(), ""] for test, _ in result.failures],
                "errors": [[test.id(), ""] for test, _ in result.errors],
            },
            ensure_ascii=False,
            indent=1,
        ),
        encoding="utf-8",
    )
    if result.testsRun == 0:
        print(f"no guard was found in {start}", file=sys.stderr)
        return 5
    return 0 if result.wasSuccessful() else 1


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
