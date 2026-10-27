"""Suppress SDK wire traces while reading a vault, without hiding other requests."""

import logging
from contextlib import contextmanager
from contextvars import ContextVar

_READING = ContextVar("reading_private_vault", default=False)


class _PrivateReadFilter(logging.Filter):
    def filter(self, record):
        return not _READING.get()


# These are the synchronous transport's loggers (including HTTP/2 and proxies).
# HTTPX otherwise logs the search text in the query string at INFO; httpcore's
# debug wire records may also carry headers. Orbit only reports codes/counts.
for _name in (
    "httpx",
    "httpcore.connection",
    "httpcore.http11",
    "httpcore.http2",
    "httpcore.proxy",
    "httpcore.socks",
):
    logging.getLogger(_name).addFilter(_PrivateReadFilter())


@contextmanager
def private_read():
    marker = _READING.set(True)
    try:
        yield
    finally:
        _READING.reset(marker)
