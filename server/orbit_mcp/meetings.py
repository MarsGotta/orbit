"""What Deimos reads about the user's meetings: the listing, a note and a transcript.

Everything here turns a hub answer into text for Deimos, from numbers and
fixed words; no meeting content is logged or invented.

The bound on every answer is Hermes's, not Orbit's. Hermes hands a tool result
entire to Deimos up to 50 000 characters (`DEFAULT_MCP_RESULT_SIZE_CHARS`,
measured in the installed Hermes); above that Deimos only sees a
1 500-character preview and a file path. So a note or a transcript longer than
that is answered a page at a time, each page saying which characters it holds
of how many and where the next one starts, and the first lines of every answer
are the ones that matter if only the preview is read.
"""

from __future__ import annotations

from datetime import UTC, datetime

from orbit_hub.meetings_bridge.page import cut

from .hub import HubClientError

# Meetings per listing. What it protects: the same 50 000 characters. Why this
# size: the longest line a meeting can produce — a 256-character title, a
# 200-character note title, a 1 000-character short summary and the fixed
# words, about 1 700 characters — times 20 is 34 000, with room for the header
# and the continuation line. A listing with more says so and how to go on.
MEETINGS_PER_ANSWER = 20

TRANSCRIPT_WORDS = {
    "ready": "ready",
    "pending": "pending",
    "partial": "partial (failed chunks are missing)",
    "failed": "failed",
}

# Why a note could not be written, from the code and the numbers the hub's
# processing client recorded. Each sentence names the cause; a code that is
# not here is still shown, with every number it came with.
NOTE_CAUSES = {
    "provider_timeout": "the model took longer than {timeout_seconds} s to answer",
    "provider_unreachable": "could not connect to the model service ({error})",
    "provider_http_error": "the model service returned HTTP {status}",
    "provider_rejected_credential": ("the model service rejected the hub key (HTTP {status})"),
    "provider_context_exceeded": (
        "the transcript does not fit the model context "
        "({prompt_tokens} tokens versus {context_tokens})"
    ),
    "provider_refusal": "the model refused to write it",
    "incomplete_completion": (
        "the model exhausted its {max_completion_tokens} response-token limit "
        "({completion_tokens} used) before finishing it"
    ),
    "invalid_response": "the model service returned something that is not a response ({reason})",
    "invalid_json_content": "the model did not return valid JSON ({bytes} bytes)",
    "note_schema_mismatch": "the model response does not have the shape of a note (field {field})",
    "request_too_large": "the model request was {bytes} bytes and the maximum is {limit}",
    "response_too_large": "the model response exceeded {limit} bytes ({bytes} read)",
    "processing_not_configured": "the hub has no model configured to write notes",
    "note_template_not_found": "template «{template_id}» does not exist",
    "transcript_empty": "the transcript has no text ({segments} empty chunks)",
    "note_internal_error": "an internal hub failure while writing it",
    "quota_exceeded": (
        "the hub reached its storage quota: it uses {used_bytes} bytes of {max_bytes} and "
        "saving this ({bytes} bytes) reserves {reserved_bytes}"
    ),
}
_STATE_KEYS = frozenset({"cause", "attempt", "attempts", "retry_at_ms", "code", "retry"})

NOTE_REFUSALS = frozenset(
    {
        "note_failed",
        "note_queued",
        "note_generating",
        "note_waiting_transcript",
        "note_not_configured",
    }
)


def _is_int(value) -> bool:
    return type(value) is int


def utc_minute(ms: int) -> str:
    return datetime.fromtimestamp(ms / 1000, tz=UTC).strftime("%Y-%m-%d %H:%M UTC")


def utc_second(ms: int) -> str:
    return datetime.fromtimestamp(ms / 1000, tz=UTC).strftime("%Y-%m-%d %H:%M:%S UTC")


def audio_length(ms: int) -> str:
    if ms < 60_000:
        return f"{ms // 1000} s"
    return f"{ms // 60_000} min"


def attempts_text(count: int) -> str:
    return f"{count} attempt{'' if count == 1 else 's'}"


def cause_sentence(numbers: dict) -> str:
    """The recorded failure in words, with its numbers; never just “failed”."""
    code = numbers.get("cause")
    template = NOTE_CAUSES.get(code)
    if template is not None:
        try:
            return template.format_map(numbers)
        except (KeyError, ValueError, IndexError):
            pass
    extra = ", ".join(
        f"{key}: {value}" for key, value in sorted(numbers.items()) if key not in _STATE_KEYS
    )
    return f"code {code}" + (f" ({extra})" if extra else "")


def note_phrase(note: dict) -> str:
    """One meeting's note in the listing: its real state, never an empty “no summary”."""
    state = note.get("state")
    title, short = note.get("title"), note.get("short_summary")
    written = isinstance(title, str) and isinstance(short, str)
    if state == "ready":
        if not written:
            raise HubClientError("invalid_response")
        return f"note ready: «{title}» — {short}"
    if state == "not_requested":
        phrase = (
            "note not requested: the Mac has not requested it, so it does not exist "
            "(this does not mean the meeting has no content)"
        )
    elif state == "not_configured":
        phrase = "note not written: the hub has no model configured to write notes"
    elif state == "waiting_transcript":
        phrase = "note waiting for transcription to finish"
    elif state == "generating" and _is_int(note.get("attempt")):
        phrase = f"note being written now (attempt {note['attempt']})"
    elif state == "queued" and _is_int(note.get("retry_at_ms")):
        phrase = f"note queued, next attempt {utc_second(note['retry_at_ms'])}"
        if _is_int(note.get("attempts")) and isinstance(note.get("cause"), str):
            phrase += (
                f", after {attempts_text(note['attempts'])} failed"
                f"{'' if note['attempts'] == 1 else 's'}; the last: {cause_sentence(note)}"
            )
    elif state == "failed" and _is_int(note.get("attempts")) and isinstance(note.get("cause"), str):
        phrase = (
            f"note failed after {attempts_text(note['attempts'])}: {cause_sentence(note)}; "
            "the hub will not retry automatically"
        )
    else:
        raise HubClientError("invalid_response")
    if written:
        phrase += f"; previously written: «{title}» — {short}"
    return phrase


def render_meetings(payload: dict, since_ms: int) -> str:
    lines = [(f"Orbit meetings since {utc_minute(since_ms)}, from latest to earliest start (UTC):")]
    last_start = last_recording = None
    for meeting in payload["meetings"]:
        if not isinstance(meeting, dict):
            raise HubClientError("invalid_response")
        recording, title = meeting.get("recording_id"), meeting.get("title")
        started, duration = meeting.get("started_at_ms"), meeting.get("duration_ms")
        capture, transcript = meeting.get("capture"), meeting.get("transcript")
        note = meeting.get("note")
        if (
            not isinstance(recording, str)
            or not isinstance(title, str)
            or not (started is None or _is_int(started))
            or not _is_int(duration)
            or capture not in {"open", "closed"}
            or transcript not in TRANSCRIPT_WORDS
            or not isinstance(note, dict)
        ):
            raise HubClientError("invalid_response")
        if started is not None:
            last_start, last_recording = started, recording
        if capture == "open":
            received = meeting.get("last_audio_received_ms")
            began = utc_minute(started) if started is not None else "at an unknown time"
            arrived = f", last at {utc_second(received)}" if _is_int(received) else ""
            words = "in progress" if transcript == "pending" else TRANSCRIPT_WORDS[transcript]
            lines.append(
                f"- IN PROGRESS · «{title}» · started {began} · the Mac has not closed it yet · "
                f"{audio_length(max(0, duration))} of audio received{arrived} · "
                f"recording {recording} · transcription {words} · {note_phrase(note)}"
            )
        else:
            lines.append(
                f"- «{title}» · {utc_minute(started)} · {max(0, duration) // 60_000} min · "
                f"recording {recording} · transcription {TRANSCRIPT_WORDS[transcript]} · "
                f"{note_phrase(note)}"
            )
    if len(lines) == 1:
        lines.append("None.")
    if payload["more"] and last_start is not None:
        lines.append(
            f"There are more meetings in that period; showing the {MEETINGS_PER_ANSWER} that "
            f"started latest. Continue with before_ms={last_start}, before_id={last_recording}."
        )
    return "\n".join(lines)


def out_of_range(after_index: int, total: int) -> HubClientError:
    return HubClientError(
        "after_index_out_of_range",
        detail={
            "refusal": {
                "code": "after_index_out_of_range",
                "after_index": after_index,
                "total_chars": total,
            }
        },
    )


def page_line(start: int, length: int, total: int, following: int | None) -> str:
    end = start + length
    if following is None:
        return f"Page: characters {start}–{end} of {total}; this is the last page."
    return f"Page: characters {start}–{end} of {total}. Continue with after_index={following}."


def render_transcript(payload: dict, after_index: int) -> str:
    text = payload["text"]
    total = payload.get("total_chars")
    if total is None and payload.get("next_after_index") is None:
        total = after_index + len(text)
    shown = text
    following = payload.get("next_after_index")
    lines = []
    if after_index or following is not None:
        if total is None:
            raise HubClientError("invalid_response")
        lines.append(page_line(after_index, len(shown), total, following))
    if payload.get("capture") == "open":
        chunks, audio = payload.get("pending_chunks"), payload.get("pending_audio_ms")
        if not _is_int(chunks) or not _is_int(audio):
            raise HubClientError("invalid_response")
        lines.append(
            "Partial transcript: the meeting is still in progress; the Mac has not closed it yet. "
            f"Still awaiting transcription: {chunks} chunks that already reached the hub, about "
            f"{audio_length(audio)} of audio on the most delayed track; audio the Mac "
            "has not uploaded yet is not counted here. The text grows and may be reordered: "
            "request it again from the beginning instead of continuing an old after_index."
        )
    if payload.get("speaker_names") == "tracks":
        lines.append(
            "Unnamed speakers: the Mac has not told the hub who its owner is yet. "
            "“Local microphone” is the Orbit user’s Mac microphone; “Person 1”, "
            "“Person 2”… "
            "are separate unnamed voices; “Another person” is audio without an identified voice yet."
        )
    final = payload.get("final_pass")
    if isinstance(final, dict) and final.get("error"):
        lines.append(
            "Draft: the final pass failed. "
            + cause_sentence({**final["error"], "cause": final["error"]["code"]})
        )
    body = "\n".join([*lines, shown]) if lines else shown
    groups = payload.get("incomplete_groups") or []
    if groups:
        missing = []
        for group in groups:
            detail = group["cause"]
            if _is_int(group.get("provider_status")):
                detail = f"HTTP {group['provider_status']} from the service"
            elif _is_int(group.get("timeout_ms")):
                detail = f"deadline of {group['timeout_ms']} ms exceeded"
            missing.append(
                f"{group['track']}, chunks {group['first_index']}–{group['last_index']}: "
                f"{detail}; {attempts_text(group['attempts'])}"
            )
        body += (
            "\n\nPartial transcript. Missing: "
            + "; ".join(missing)
            + ". It will not retry automatically; the original audio is retained."
        )
    return body


def _note_text(payload: dict) -> str:
    note = payload["note"]
    try:
        title, short = str(note["title"]), str(note["short_summary"])
        header = (
            f"Note «{title}» from recording {payload.get('recording_id')} · "
            f"template {note['template_name']} · written by {note['model']} at "
            f"{utc_minute(int(note['generated_at_ms']))} · revision {payload.get('revision')}"
        )
        lines = [header]
        if note.get("final_pass_error"):
            failure = note["final_pass_error"]
            lines.append(
                "Note written from the draft: "
                + cause_sentence({**failure, "cause": failure["code"]})
            )
        if payload["state"] != "ready":
            lines.append(
                "This is not the final version: "
                + note_phrase({"state": payload["state"], **(payload.get("progress") or {})})
                + "."
            )
        edited = payload["edited"]
        changed = [
            word
            for word, flag in (
                ("the title", edited.get("title")),
                ("the sections", edited.get("sections")),
            )
            if flag
        ]
        if changed:
            lines.append(f"The Orbit user edited {' and '.join(changed)}; their version follows.")
        if payload.get("superseded_edits"):
            lines.append(
                "Rewriting the note set aside previous edits by the user of "
                "Orbit; "
                "the app shows them to the user."
            )
        lines.append(f"Short summary: {short}")
        lines.append(
            f"Written from {note['segments']} transcript chunks, "
            f"{int(note['duration_ms']) // 60_000} minutes of meeting"
            + (f", in {note['parts']} parts." if int(note["parts"]) > 1 else ".")
        )
        lines.append("\n# Summary by section")
        for section in note["sections"]:
            lines.append(f"## {section['name']}\n{section['markdown']}")
        lines.append("\n# Action items")
        for item in note["action_items"] or []:
            owner = item.get("owner") or "Unassigned"
            due = f" (due: {item['due']})" if item.get("due") else ""
            mark = "done" if item.get("done") is True else "pending"
            lines.append(f"- [{mark}] {owner}: {item['text']}{due}")
        if not note["action_items"]:
            lines.append("None.")
        lines.append("\n# Decisions")
        lines.extend(f"- {decision}" for decision in note["decisions"] or [])
        if not note["decisions"]:
            lines.append("None.")
        lines.append("\n# Open questions")
        lines.extend(f"- {topic}" for topic in note["open_topics"] or [])
        if not note["open_topics"]:
            lines.append("None.")
    except (KeyError, TypeError, ValueError):
        raise HubClientError("invalid_response") from None
    owner = payload.get("owner_name")
    lines.append(f"\n# Private notes by {owner}" if isinstance(owner, str) else "\n# Private notes")
    lines.append(payload["private_notes"] or "None.")
    return "\n".join(lines)


def render_note(payload: dict, after_index: int) -> str:
    text = _note_text(payload)
    if after_index > len(text):
        raise out_of_range(after_index, len(text))
    shown = cut(text[after_index:])
    end = after_index + len(shown)
    following = end if end < len(text) else None
    if not after_index and following is None:
        return shown
    lines = [page_line(after_index, len(shown), len(text), following)]
    if after_index:
        lines.append(f"Continuation of the note for recording {payload.get('recording_id')}.")
    return "\n".join([*lines, shown])


def refusal_sentence(code: str, refusal) -> str:
    """The numbers of a note or page refusal, in words; empty when there are none."""
    if not isinstance(refusal, dict):
        return ""
    attempts = refusal.get("attempts")
    parts = []
    if code == "note_failed" and isinstance(refusal.get("cause"), str):
        parts.append(f"Cause: {cause_sentence(refusal)}.")
        if _is_int(attempts):
            parts.append(f"{attempts_text(attempts)}.")
    elif code == "note_queued":
        if _is_int(refusal.get("retry_at_ms")):
            parts.append(f"Next attempt: {utc_second(refusal['retry_at_ms'])}.")
        if isinstance(refusal.get("cause"), str):
            parts.append(f"Last failure: {cause_sentence(refusal)}.")
        if _is_int(attempts):
            parts.append(f"{attempts_text(attempts)} so far.")
    elif code == "note_generating" and _is_int(refusal.get("attempt")):
        parts.append(f"Attempt {refusal['attempt']} in progress.")
    elif code == "note_waiting_transcript":
        if _is_int(refusal.get("pending_chunks")):
            parts.append(f"Remaining: {refusal['pending_chunks']} chunks to transcribe.")
        if refusal.get("capture") == "open":
            parts.append("The meeting is still in progress: the Mac has not closed it yet.")
        elif refusal.get("capture") == "closed":
            parts.append("The recording is already closed.")
        if _is_int(refusal.get("retry_at_ms")):
            parts.append(f"The hub checks it again at {utc_second(refusal['retry_at_ms'])}.")
    elif code == "after_index_out_of_range":
        if _is_int(refusal.get("after_index")) and _is_int(refusal.get("total_chars")):
            parts.append(
                f"You requested after_index={refusal['after_index']} and the text has "
                f"{refusal['total_chars']} characters."
            )
    return " ".join(parts)
