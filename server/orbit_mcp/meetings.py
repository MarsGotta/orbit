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
    "ready": "lista",
    "pending": "pendiente",
    "partial": "parcial (faltan fragmentos que fallaron)",
    "failed": "fallida",
}

# Why a note could not be written, from the code and the numbers the hub's
# processing client recorded. Each sentence names the cause; a code that is
# not here is still shown, with every number it came with.
NOTE_CAUSES = {
    "provider_timeout": "el modelo tardó más de {timeout_seconds} s en contestar",
    "provider_unreachable": "no se pudo conectar con el servicio del modelo ({error})",
    "provider_http_error": "el servicio del modelo devolvió HTTP {status}",
    "provider_rejected_credential": (
        "el servicio del modelo rechazó la clave del hub (HTTP {status})"
    ),
    "provider_context_exceeded": (
        "la transcripción no cabe en el contexto del modelo "
        "({prompt_tokens} tokens frente a {context_tokens})"
    ),
    "provider_refusal": "el modelo se negó a escribirla",
    "incomplete_completion": (
        "el modelo agotó su límite de {max_completion_tokens} tokens de respuesta "
        "({completion_tokens} usados) antes de terminarla"
    ),
    "invalid_response": "el servicio del modelo contestó algo que no es una respuesta ({reason})",
    "invalid_json_content": "el modelo no devolvió JSON válido ({bytes} bytes)",
    "note_schema_mismatch": "la respuesta del modelo no tiene la forma de una nota (campo {field})",
    "request_too_large": "la petición al modelo ocupaba {bytes} bytes y el máximo es {limit}",
    "response_too_large": "la respuesta del modelo pasó de {limit} bytes ({bytes} leídos)",
    "processing_not_configured": "el hub no tiene configurado el modelo que escribe las notas",
    "note_template_not_found": "no existe la plantilla «{template_id}»",
    "transcript_empty": "la transcripción no tiene texto ({segments} fragmentos vacíos)",
    "note_internal_error": "un fallo interno del hub al escribirla",
    "quota_exceeded": (
        "el hub llegó a su cuota de almacenamiento: usa {used_bytes} bytes de {max_bytes} y "
        "guardar esto pedía {bytes} más"
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
    return f"{count} intento{'' if count == 1 else 's'}"


def cause_sentence(numbers: dict) -> str:
    """The recorded failure in words, with its numbers; never just «falló»."""
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
    return f"código {code}" + (f" ({extra})" if extra else "")


def note_phrase(note: dict) -> str:
    """One meeting's note in the listing: its real state, never an empty «sin resumen»."""
    state = note.get("state")
    title, short = note.get("title"), note.get("short_summary")
    written = isinstance(title, str) and isinstance(short, str)
    if state == "ready":
        if not written:
            raise HubClientError("invalid_response")
        return f"nota lista: «{title}» — {short}"
    if state == "not_requested":
        phrase = (
            "nota sin pedir: el Mac no la ha pedido, así que no existe "
            "(no quiere decir que la reunión no tenga contenido)"
        )
    elif state == "not_configured":
        phrase = "nota sin escribir: el hub no tiene configurado el modelo que escribe las notas"
    elif state == "waiting_transcript":
        phrase = "nota esperando a que termine la transcripción"
    elif state == "generating" and _is_int(note.get("attempt")):
        phrase = f"nota escribiéndose ahora (intento {note['attempt']})"
    elif state == "queued" and _is_int(note.get("retry_at_ms")):
        phrase = f"nota en cola, próximo intento {utc_second(note['retry_at_ms'])}"
        if _is_int(note.get("attempts")) and isinstance(note.get("cause"), str):
            phrase += (
                f", tras {attempts_text(note['attempts'])} fallido"
                f"{'' if note['attempts'] == 1 else 's'}; el último: {cause_sentence(note)}"
            )
    elif state == "failed" and _is_int(note.get("attempts")) and isinstance(note.get("cause"), str):
        phrase = (
            f"nota fallida tras {attempts_text(note['attempts'])}: {cause_sentence(note)}; "
            "el hub no la reintenta solo"
        )
    else:
        raise HubClientError("invalid_response")
    if written:
        phrase += f"; la escrita antes: «{title}» — {short}"
    return phrase


def render_meetings(payload: dict, since_ms: int) -> str:
    lines = [
        (
            f"Reuniones de Orbit desde {utc_minute(since_ms)}, de la que empezó más tarde a la "
            "que empezó antes (hora UTC):"
        )
    ]
    last_start = None
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
            last_start = started
        if capture == "open":
            received = meeting.get("last_audio_received_ms")
            began = utc_minute(started) if started is not None else "a una hora desconocida"
            arrived = f", el último a las {utc_second(received)}" if _is_int(received) else ""
            words = "en curso" if transcript == "pending" else TRANSCRIPT_WORDS[transcript]
            lines.append(
                f"- EN CURSO · «{title}» · empezó {began} · el Mac aún no la ha cerrado · "
                f"{audio_length(max(0, duration))} de audio recibido{arrived} · "
                f"grabación {recording} · transcripción {words} · {note_phrase(note)}"
            )
        else:
            lines.append(
                f"- «{title}» · {utc_minute(started)} · {max(0, duration) // 60_000} min · "
                f"grabación {recording} · transcripción {TRANSCRIPT_WORDS[transcript]} · "
                f"{note_phrase(note)}"
            )
    if len(lines) == 1:
        lines.append("Ninguna.")
    if payload["more"] and last_start is not None:
        lines.append(
            f"Hay más reuniones en ese periodo; se muestran las {MEETINGS_PER_ANSWER} que "
            f"empezaron más tarde. Sigue con before_ms={last_start}."
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
        return f"Página: caracteres {start}–{end} de {total}; es la última."
    return f"Página: caracteres {start}–{end} de {total}. Sigue con after_index={following}."


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
            "Transcripción parcial: la reunión sigue en curso; el Mac aún no la ha cerrado. "
            f"Quedan por transcribir {chunks} fragmentos que ya llegaron al hub, unos "
            f"{audio_length(audio)} de audio en la pista más atrasada; el audio que el Mac "
            "todavía no ha subido no cuenta aquí. El texto crece y puede reordenarse: "
            "vuelve a pedirlo desde el principio en vez de seguir un after_index antiguo."
        )
    if payload.get("speaker_names") == "tracks":
        lines.append(
            "Hablantes sin nombre: el Mac aún no le ha dicho al hub quién es su dueño. "
            "«Micrófono local» es el micrófono del Mac de quien usa Orbit; «Persona 1», "
            "«Persona 2»… "
            "son voces separadas sin nombre; «Otra persona», audio aún sin voz identificada."
        )
    final = payload.get("final_pass")
    if isinstance(final, dict) and final.get("error"):
        lines.append(
            "Borrador: falló la pasada final. "
            + cause_sentence({**final["error"], "cause": final["error"]["code"]})
        )
    body = "\n".join([*lines, shown]) if lines else shown
    groups = payload.get("incomplete_groups") or []
    if groups:
        missing = []
        for group in groups:
            detail = group["cause"]
            if _is_int(group.get("provider_status")):
                detail = f"HTTP {group['provider_status']} del servicio"
            elif _is_int(group.get("timeout_ms")):
                detail = f"plazo de {group['timeout_ms']} ms vencido"
            missing.append(
                f"{group['track']}, fragmentos {group['first_index']}–{group['last_index']}: "
                f"{detail}; {attempts_text(group['attempts'])}"
            )
        body += (
            "\n\nTranscripción parcial. Falta: "
            + "; ".join(missing)
            + ". No se reintentará automáticamente; el audio original se conserva."
        )
    return body


def _note_text(payload: dict) -> str:
    note = payload["note"]
    try:
        title, short = str(note["title"]), str(note["short_summary"])
        header = (
            f"Nota «{title}» de la grabación {payload.get('recording_id')} · "
            f"plantilla {note['template_name']} · escrita por {note['model']} el "
            f"{utc_minute(int(note['generated_at_ms']))} · revisión {payload.get('revision')}"
        )
        lines = [header]
        if note.get("final_pass_error"):
            failure = note["final_pass_error"]
            lines.append(
                "Nota escrita con el borrador: "
                + cause_sentence({**failure, "cause": failure["code"]})
            )
        if payload["state"] != "ready":
            lines.append(
                "No es la versión definitiva: "
                + note_phrase({"state": payload["state"], **(payload.get("progress") or {})})
                + "."
            )
        edited = payload["edited"]
        changed = [
            word
            for word, flag in (
                ("el título", edited.get("title")),
                ("las secciones", edited.get("sections")),
            )
            if flag
        ]
        if changed:
            lines.append(
                f"Quien usa Orbit editó {' y '.join(changed)}; lo que sigue es su versión."
            )
        if payload.get("superseded_edits"):
            lines.append(
                "Al reescribir la nota quedaron apartadas ediciones anteriores de quien usa "
                "Orbit; "
                "la app se las muestra."
            )
        lines.append(f"Resumen corto: {short}")
        lines.append(
            f"Escrita a partir de {note['segments']} fragmentos de transcripción, "
            f"{int(note['duration_ms']) // 60_000} min de reunión"
            + (f", en {note['parts']} partes." if int(note["parts"]) > 1 else ".")
        )
        lines.append("\n# Resumen por secciones")
        for section in note["sections"]:
            lines.append(f"## {section['name']}\n{section['markdown']}")
        lines.append("\n# Tareas")
        for item in note["action_items"] or []:
            owner = item.get("owner") or "Sin responsable"
            due = f" (plazo: {item['due']})" if item.get("due") else ""
            mark = "hecha" if item.get("done") is True else "pendiente"
            lines.append(f"- [{mark}] {owner}: {item['text']}{due}")
        if not note["action_items"]:
            lines.append("Ninguna.")
        lines.append("\n# Decisiones")
        lines.extend(f"- {decision}" for decision in note["decisions"] or [])
        if not note["decisions"]:
            lines.append("Ninguna.")
        lines.append("\n# Temas abiertos")
        lines.extend(f"- {topic}" for topic in note["open_topics"] or [])
        if not note["open_topics"]:
            lines.append("Ninguno.")
    except (KeyError, TypeError, ValueError):
        raise HubClientError("invalid_response") from None
    owner = payload.get("owner_name")
    lines.append(
        f"\n# Notas privadas de {owner}" if isinstance(owner, str) else "\n# Notas privadas"
    )
    lines.append(payload["private_notes"] or "Ninguna.")
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
        lines.append(f"Continuación de la nota de la grabación {payload.get('recording_id')}.")
    return "\n".join([*lines, shown])


def refusal_sentence(code: str, refusal) -> str:
    """The numbers of a note or page refusal, in words; empty when there are none."""
    if not isinstance(refusal, dict):
        return ""
    attempts = refusal.get("attempts")
    parts = []
    if code == "note_failed" and isinstance(refusal.get("cause"), str):
        parts.append(f"Causa: {cause_sentence(refusal)}.")
        if _is_int(attempts):
            parts.append(f"{attempts_text(attempts)}.")
    elif code == "note_queued":
        if _is_int(refusal.get("retry_at_ms")):
            parts.append(f"Próximo intento: {utc_second(refusal['retry_at_ms'])}.")
        if isinstance(refusal.get("cause"), str):
            parts.append(f"Último fallo: {cause_sentence(refusal)}.")
        if _is_int(attempts):
            parts.append(f"{attempts_text(attempts)} hasta ahora.")
    elif code == "note_generating" and _is_int(refusal.get("attempt")):
        parts.append(f"Intento {refusal['attempt']} en curso.")
    elif code == "note_waiting_transcript":
        if _is_int(refusal.get("pending_chunks")):
            parts.append(f"Quedan {refusal['pending_chunks']} fragmentos por transcribir.")
        if refusal.get("capture") == "open":
            parts.append("La reunión sigue en curso: el Mac aún no la ha cerrado.")
        elif refusal.get("capture") == "closed":
            parts.append("La grabación ya está cerrada.")
        if _is_int(refusal.get("retry_at_ms")):
            parts.append(f"El hub vuelve a mirarla a las {utc_second(refusal['retry_at_ms'])}.")
    elif code == "after_index_out_of_range":
        if _is_int(refusal.get("after_index")) and _is_int(refusal.get("total_chars")):
            parts.append(
                f"Pediste after_index={refusal['after_index']} y el texto tiene "
                f"{refusal['total_chars']} caracteres."
            )
    return " ".join(parts)
