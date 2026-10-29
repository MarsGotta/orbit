"""Stream a step's log until its owner records the final byte count.

Descendants may keep writing to the file after the command exits. They cannot
keep the gate's output pipe open, and their lifetime never decides step completion.
"""

import sys
import time
from pathlib import Path


def stream(log: Path, completed: Path) -> None:
    with log.open("rb") as source:
        end = None
        while True:
            if end is None and completed.exists():
                end = int(completed.read_text())
            remaining = 65536 if end is None else min(65536, end - source.tell())
            if remaining <= 0:
                return
            data = source.read(remaining)
            if data:
                sys.stdout.buffer.write(data)
                sys.stdout.buffer.flush()
            else:
                # Poll only for new output/completion, never a command deadline.
                time.sleep(0.05)


if __name__ == "__main__":
    stream(Path(sys.argv[1]), Path(sys.argv[2]))
