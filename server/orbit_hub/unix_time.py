"""The wire range of a date in milliseconds since 1970, shared by every vertical.

Year 1 to year 9999, the same span ``orbit_core::wire::unix_ms`` and the macOS
helper admit, so a date that crosses from the Mac is never valid on one side and
malformed on the other. It is a range of representable dates, not a policy: it
decides nothing about what may be read, kept or asked for.

"""

from typing import Annotated

from pydantic import Field

MIN_UNIX_MS = -62_135_596_800_000
MAX_UNIX_MS = 253_402_300_799_999

DateMs = Annotated[int, Field(ge=MIN_UNIX_MS, le=MAX_UNIX_MS)]
