"""Which system the hub runs on, for the adapters that differ by it.

The hub runs on a Linux server or on a Mac. Only the adapters that depend on
the system ask, and they ask when they act (`host_system.linux()`), never at
import, so a test can stand in for another system by replacing this function.
"""

import sys


def linux() -> bool:
    return sys.platform.startswith("linux")
