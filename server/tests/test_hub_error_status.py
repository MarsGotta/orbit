"""P7-04: moving status ownership into HubError preserves the public HTTP contract."""

import pytest

from orbit_hub.protocol import HubError

# P9-08, SIM-02 and SIM-41: the codes of retired verticals left with the
# routes that answered them.
CASES = {
    401: "unauthorized",
    403: "revoked invalid_pairing",
    404: "session_not_found recording_not_found note_not_requested note_template_not_found",
    409: "chunk_conflict note_not_ready note_revision_changed recording_finalized manifest_conflict manifest_mismatch",
    413: "too_large",
    429: "rate_limited",
    507: "quota_exceeded",
}


@pytest.mark.parametrize(
    "status,code", [(status, code) for status, codes in CASES.items() for code in codes.split()]
)
def test_every_pre_refactor_code_keeps_its_http_status(status, code):
    assert HubError(code, "redacted").http_status == status


def test_explicit_http_and_websocket_codes_override_defaults():
    error = HubError("synthetic", "redacted", http_status=418, ws_code=4444)
    assert (error.http_status, error.ws_code) == (418, 4444)


@pytest.mark.parametrize("code", ["note_queued", "note_generating", "note_waiting_transcript"])
def test_a_note_in_progress_says_when_to_ask_again_never_not_to(code):
    from orbit_hub.errors import render

    refusal = render(code, "redacted", numbers={"retry_after_ms": 45_000})["refusal"]
    assert refusal["retry"] == {"kind": "after_ms", "ms": 45_000}


def test_a_failed_note_is_not_advised_as_a_wait():
    from orbit_hub.errors import render

    assert render("note_failed", "redacted", numbers={})["refusal"]["retry"]["kind"] == "never"
