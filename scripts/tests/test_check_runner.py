"""A failed gate step must finish even if its helper retains the output pipe."""

import os
import signal
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]


class CheckRunnerTests(unittest.TestCase):
    def test_failed_step_with_a_live_helper_finishes_and_records_its_exit_code(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = Path(directory)
            (fixture / "fail.py").write_text(
                "import subprocess, sys\n"
                "subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(600)'])\n"
                "print('deliberate fixture failure', flush=True)\n"
                "sys.exit(23)\n"
            )
            command = """
set -euo pipefail
source scripts/check/runner.sh
check_init "$1/logs"
check_run failure 'deliberate failure' -- python3 "$1/fail.py"
check_run following 'following step' -- printf 'following step ran\n'
check_finish
"""
            process = subprocess.Popen(
                ["bash", "-c", command, "fixture", directory],
                cwd=ROOT,
                stdout=subprocess.PIPE,
                stderr=subprocess.STDOUT,
                start_new_session=True,
            )
            try:
                # This only catches a pipe held by the fixture's ten-minute
                # sleeper; step completion and its real exit code decide success.
                output, _ = process.communicate(timeout=10)
                self.assertEqual(process.returncode, 1, output.decode())
                self.assertIn(b"following step ran", output)
                results = (fixture / "logs/results.tsv").read_text()
                self.assertIn("failure\tfailed\t", results)
                self.assertIn("code 23", results)
                self.assertIn("following\tok\t", results)
                self.assertEqual(
                    (fixture / "logs/failure.log").read_text(),
                    "deliberate fixture failure\n",
                )
            finally:
                try:
                    os.killpg(process.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
                process.communicate()


if __name__ == "__main__":
    unittest.main()
