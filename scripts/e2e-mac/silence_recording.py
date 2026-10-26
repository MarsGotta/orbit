"""Check 19: an unanswered silence question closes and saves a recording.

The Mac plays nothing. Every wait observes a fact, with its budget derived
from the production quiet/answer, sample delivery, drain and HTTP constants.
"""

from __future__ import annotations

from checks import _fact, _tree
from meeting_control_checks import listed
from meeting_controls import wait_hub

NAMES = {19: "A silent recording asks, closes without an answer and is saved with its hub chunks"}
NOT_COVERED = ()
QUESTION = "¿Sigue la reunión? La termino y la guardo en 30 s"


def bounds(rust_constant) -> dict[str, tuple[float, str]]:
    quiet = rust_constant("crates/orbit-core/src/silence.rs", "RECORDING_SILENCE_MS") / 1000
    answer = rust_constant("crates/orbit-core/src/silence.rs", "RECORDING_ANSWER_MS") / 1000
    delivery = (
        rust_constant("crates/orbit-core/src/audio_signal.rs", "SAMPLE_DELIVERY_TIMEOUT_MS") / 1000
    )
    drain = rust_constant("crates/orbit-core/src/lib.rs", "FINALIZING_TIMEOUT_MS") / 1000
    http = rust_constant("crates/orbit-daemon/src/hub.rs", "HUB_HTTP_TIMEOUT_SECONDS")
    # The real run re-asked every 3–16 s. That is not a PCM delivery bound.
    # Allow one answer interval for a positive post-expiry observation; absence
    # remains a test timeout, never permission for production to close.
    return {
        "silence-question": (
            quiet + delivery,
            "RECORDING_SILENCE_MS plus SAMPLE_DELIVERY_TIMEOUT_MS for the tick and pushed notice",
        ),
        "silence-saved": (
            2 * answer + delivery + drain + http,
            (
                "RECORDING_ANSWER_MS to answer, one more answer interval for a fresh quiet observation, "
                "SAMPLE_DELIVERY_TIMEOUT_MS for its tick, then FINALIZING_TIMEOUT_MS and HUB_HTTP_TIMEOUT_SECONDS for normal save"
            ),
        ),
        "silence-hub": (
            drain + http,
            "FINALIZING_TIMEOUT_MS plus HUB_HTTP_TIMEOUT_SECONDS for sealed chunks and closure to reach the test hub",
        ),
    }


def record_without_answer(test) -> None:
    with test.step("a silent recording ends and is saved without an answer", (19,), critical=False):
        ui, main = test.ui, test.main
        ui.press(main, "Grabar reunión", ("AXButton",), first=True)
        ui.press(main, "Solo mi micrófono", ("AXRadioButton",), contains=True)
        ui.press(main, "Empezar a grabar", ("AXButton",))
        recording = test.wait_mac("recording")
        evidence = test.ev["silence"] = {"recording": recording}
        evidence["question"] = test.wait_mac("silence-question", recording["id"])
        test.measure(
            "screen",
            ui.read_until(
                main,
                "silence-question",
                QUESTION,
                lambda text: QUESTION in text,
                test.bounds["screen"][0],
            ),
        )
        # No say, keep, finish or other input while the question waits.
        evidence["saved"] = test.wait_mac("silence-saved", recording["id"])
        evidence["hub"] = wait_hub(
            test,
            "silence-hub",
            recording["id"],
            lambda kept: (
                not kept["deleted"]
                and kept["rows"].get("chunks", 0) > 0
                and kept["rows"].get("recording_manifests", 0) > 0
            ),
        )
        ui.press(main, "Reuniones", ("AXLink",), first=True)
        title = evidence["saved"]["title"]
        test.measure(
            "screen",
            ui.read_nodes_until(
                main,
                "silence-meetings",
                None,
                lambda nodes: title in (listed(nodes) or []),
                test.bounds["screen"][0],
            ),
        )


def c19_saved(ev: dict) -> tuple[bool, str]:
    evidence = _fact(ev, "silence")
    recording = evidence["recording"]
    ident = recording.get("id")
    if not ident or ident in (
        ev.get("recording", {}).get("id"),
        ev.get("discard", {}).get("recording", {}).get("id"),
    ):
        return False, "the silence check did not start its own recording"
    question = evidence.get("question", {})
    if (
        question.get("code") != "recording_silence_question"
        or question.get("recording_id") != ident
    ):
        return False, "the current recording's silence question was not observed"
    if not any(QUESTION in node.text() for node in _tree(ev, "silence-question")):
        return False, "the silence question did not appear in the interface"
    saved = evidence.get("saved", {})
    if saved.get("recording_id") != ident or saved.get("capture") != "closed":
        return False, "the silent recording did not close through the normal save"
    kept = evidence.get("hub", {})
    chunks = kept.get("rows", {}).get("chunks", 0)
    if (
        chunks <= 0
        or kept.get("deleted")
        or kept.get("rows", {}).get("recording_manifests", 0) <= 0
    ):
        return False, "the closed silent recording has no chunks and closure on the hub"
    if saved.get("title") not in (listed(_tree(ev, "silence-meetings")) or []):
        return False, "the saved silent recording is missing from Reuniones"
    return True, f"question unanswered; recording closed, listed and saved with {chunks} hub chunks"


CHECKS = {19: c19_saved}
