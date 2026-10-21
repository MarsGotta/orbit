"""The latest wire date in milliseconds since 1970: the end of year 9999.

The same ceiling as ``orbit_core::calendar::MAX_UNIX_MS`` and the macOS helper, so a
signed request time that crosses from the Mac is never valid on one side and
malformed on the other. It is a range of representable dates, not a policy: it
decides nothing about what may be read, kept or asked for.
"""

MAX_UNIX_MS = 253_402_300_799_999
