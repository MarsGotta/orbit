"""Version of the external driver that the daemon accepts."""

import re
from pathlib import Path

CUA = Path(__file__).resolve().parents[2] / "crates/orbit-daemon/src/cua.rs"


def verified_driver_version():
    found = re.search(
        r'^pub const VERIFIED_DRIVER_VERSION: &str = "([0-9][0-9.]*)";$',
        CUA.read_text(encoding="utf-8"),
        re.MULTILINE,
    )
    if found is None:
        raise AssertionError(f"{CUA} ya no declara VERIFIED_DRIVER_VERSION")
    return found.group(1)
