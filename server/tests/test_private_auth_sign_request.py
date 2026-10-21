"""Client-side signing round trip for the hub's private routes.

One scheme, both directions: the headers ``sign_request`` produces must be the
headers the real ``Authenticator`` of each route admits, and nothing else's —
not another route's, not another protocol's, and not twice.
"""

import time
from uuid import uuid4

import pytest

from orbit_hub.bridge_route import HEADER_PREFIX as BRIDGE_HEADER_PREFIX
from orbit_hub.bridge_route import PATH as BRIDGE_PATH
from orbit_hub.bridge_route import PROTOCOL as BRIDGE_PROTOCOL
from orbit_hub.meetings_bridge import HEADER_PREFIX as MEETINGS_HEADER_PREFIX
from orbit_hub.meetings_bridge import LIST_PATH as MEETINGS_PATH
from orbit_hub.meetings_bridge import PROTOCOL as MEETINGS_PROTOCOL
from orbit_hub.private_auth import Authenticator, sign_request

KEY = bytes(range(32))
KEY_ID = "round-trip-key"
BODY = (
    b'{"protocol":"orbit_device_bridge_v1","device_id":"un-mac","capability":"screen_sources",'
    b'"parameters":{}}'
)
OTHER_PATH = "/internal/v1/somewhere/else"
OTHER_PROTOCOL = "orbit_something_else_v1"


class Rejected(Exception):
    """What every private route's failure type carries: a status and a code."""

    def __init__(self, status: int, code: str):
        self.status, self.code = status, code
        super().__init__(code)


def authenticator(protocol, path, header_prefix):
    return Authenticator(
        KEY_ID,
        KEY,
        protocol=protocol,
        path=path,
        header_prefix=header_prefix,
        failure=Rejected,
    )


def admit(auth, headers, body):
    """Exactly the call the routes make, with the scope's header list shape."""
    now = int(time.time() * 1000)
    scoped = [
        (name.lower().encode("ascii"), value.encode("ascii")) for name, value in headers.items()
    ]
    auth.admit(scoped, body, now, time.monotonic())


def signed(protocol=BRIDGE_PROTOCOL, path=BRIDGE_PATH, nonce=None):
    return sign_request(
        protocol,
        path,
        KEY_ID,
        KEY,
        BODY,
        issued_at_ms=int(time.time() * 1000),
        nonce=nonce or str(uuid4()),
    )


def test_a7_sign_request_is_admitted_by_the_real_authenticator_of_each_route():
    for protocol, path, prefix in (
        (BRIDGE_PROTOCOL, BRIDGE_PATH, BRIDGE_HEADER_PREFIX),
        # REF-10 retired the calendar consumer route; the meetings bridge is the
        # other private route that signs with its own key.
        (MEETINGS_PROTOCOL, MEETINGS_PATH, MEETINGS_HEADER_PREFIX),
    ):
        admit(authenticator(protocol, path, prefix), signed(protocol, path), BODY)


def test_a7_the_same_signature_is_refused_on_another_path_or_protocol():
    headers = signed()
    for protocol, path in ((BRIDGE_PROTOCOL, OTHER_PATH), (OTHER_PROTOCOL, BRIDGE_PATH)):
        with pytest.raises(Rejected) as rejected:
            admit(authenticator(protocol, path, BRIDGE_HEADER_PREFIX), headers, BODY)
        assert rejected.value.status == 401
        assert rejected.value.code == "unauthorized"


def test_a7_a_repeated_nonce_is_refused_as_a_replay():
    nonce = str(uuid4())
    headers = signed(nonce=nonce)
    auth = authenticator(BRIDGE_PROTOCOL, BRIDGE_PATH, BRIDGE_HEADER_PREFIX)
    admit(auth, headers, BODY)
    with pytest.raises(Rejected) as rejected:
        admit(auth, headers, BODY)
    assert rejected.value.status == 401
    assert rejected.value.code == "consumer_replay"
