"""The assistant's folders on the Mac, read through the device bridge, in Spanish.

Every answer names whole paths, so the assistant can ask for the next one
with what it was given, and says where to go on when there is more. A refusal
names the path asked for, where it led and every folder: the facts the
assistant needs to correct its request.
"""

from __future__ import annotations

from orbit_hub.meetings_bridge.page import MAX_ANSWER_CHARS

REFUSALS = frozenset(
    {"no_folders", "outside_folders", "folder_unavailable", "file_unavailable", "file_too_large"}
)

KINDS = {"text": "texto", "pdf": "PDF", "image": "imágenes"}

STOPS = {
    "results": "Hay más de {limit} resultados; afina la búsqueda.",
    "visited": "La búsqueda se detuvo tras recorrer {limit} entradas; busca en una carpeta más concreta.",
    "files_read": "La búsqueda se detuvo tras leer {limit} archivos; busca en una carpeta más concreta.",
    "bytes_read": "La búsqueda se detuvo tras leer {limit} bytes; busca en una carpeta más concreta.",
    "deadline": "La búsqueda se detuvo a los {limit} ms; busca en una carpeta más concreta.",
    "page_bytes": "Los resultados llenaron la respuesta ({limit} bytes); afina la búsqueda.",
}


def _joined(directory: str, name: str) -> str:
    return f"{directory.rstrip('/')}/{name}"


def folder_cause(cause) -> str:
    if not isinstance(cause, dict):
        return "causa desconocida"
    kind = cause.get("kind")
    if kind == "bookmark_unresolvable":
        return (
            "macOS no encuentra la carpeta: se borró o su disco no está conectado "
            f"({cause.get('domain')} {cause.get('code')})"
        )
    if kind == "not_a_folder":
        return "lo que hay en su lugar ya no es una carpeta"
    if kind == "access_denied":
        return f"macOS no deja abrirla (errno {cause.get('errno')})"
    if kind in {"path_too_long", "bookmark_too_large"}:
        what = "su ruta" if kind == "path_too_long" else "su marcador"
        return f"{what} ocupa {cause.get('bytes')} bytes; Orbit admite {cause.get('max_bytes')}"
    return f"causa {kind}"


def file_cause(cause) -> str:
    if not isinstance(cause, dict):
        return "causa desconocida"
    kind = cause.get("kind")
    sentences = {
        "not_found": "no existe",
        "not_a_file": "no es un archivo",
        "not_a_folder": "no es una carpeta",
        "not_text": "no es texto UTF-8 (es un archivo binario o de otra codificación)",
        "pdf_unreadable": "PDFKit no puede abrir el PDF (está dañado o protegido con contraseña)",
        "image_unreadable": "ImageIO no puede abrir la imagen",
        "changed": "cambió mientras se leía (un enlace apareció en su ruta); no se leyó nada, vuelve a pedirlo",
    }
    if kind in sentences:
        return sentences[kind]
    if kind == "image_does_not_fit":
        return (
            f"ni a su menor tamaño cabe en una captura: {cause.get('smallest_png_bytes')} bytes "
            f"de PNG frente a {cause.get('max_png_bytes')}"
        )
    if kind == "read_failed":
        return f"macOS no dejó leerlo (errno {cause.get('errno')})"
    if kind == "past_end":
        return f"after_index={cause.get('after_index')} pasa del final ({cause.get('size_bytes')} bytes)"
    if kind == "not_character_boundary":
        return f"after_index={cause.get('after_index')} cae dentro de un carácter; usa el que dio la respuesta anterior"
    if kind == "past_last_page":
        return f"after_page={cause.get('after_page')} pasa de la última página (tiene {cause.get('page_count')})"
    return f"causa {kind}"


def refusal_message(code: str, refusal) -> str:
    """The sentence for a folder refusal, with its path, folders and numbers."""
    refusal = refusal if isinstance(refusal, dict) else {}
    path = refusal.get("path")
    if code == "no_folders":
        return (
            "Quien usa Orbit no ha dado al asistente ninguna carpeta de este Mac. Puede añadirla "
            "en Orbit, en Ajustes › Carpetas del asistente › «Añadir carpeta»."
        )
    if code == "outside_folders":
        folders = refusal.get("folders")
        listed = ", ".join(folders) if isinstance(folders, list) and folders else "ninguna"
        resolved = refusal.get("resolved")
        leads = f", que lleva a {resolved}," if isinstance(resolved, str) else ""
        return (
            f"{path}{leads} está fuera de las carpetas que el asistente puede leer. "
            f"Las carpetas son: {listed}. Pide una ruta dentro de ellas."
        )
    if code == "folder_unavailable":
        return (
            f"La carpeta {path} no se puede abrir: {folder_cause(refusal.get('cause'))}. "
            "Quien usa Orbit puede quitarla y volver a añadirla en Ajustes."
        )
    if code == "file_unavailable":
        return f"{path}: {file_cause(refusal.get('cause'))}."
    kind = KINDS.get(refusal.get("content"), "archivos")
    return (
        f"{path} pesa {refusal.get('bytes')} bytes y Orbit lee {kind} de hasta "
        f"{refusal.get('max_bytes')} bytes, así que no lo abrió."
    )


def _size(entry: dict) -> str:
    size = entry.get("size_bytes")
    return f" ({size} bytes)" if type(size) is int else ""


def render_list(result: dict) -> str:
    directory = result.get("path")
    entries = result["entries"]
    if directory is None:
        lines = []
        for entry in entries:
            unavailable = entry.get("unavailable")
            note = (
                f" — no se puede leer ahora: {folder_cause(unavailable)}"
                if isinstance(unavailable, dict)
                else ""
            )
            lines.append(f"- {entry['name']}/{note}")
        if not lines:
            return "Quien usa Orbit no ha dado al asistente ninguna carpeta."
        return "Carpetas que el asistente puede leer en este Mac:\n" + "\n".join(lines)
    start = result["after_index"]
    total = result["total"]
    lines = []
    shown = 0
    budget = MAX_ANSWER_CHARS - 400
    for entry in entries:
        full = _joined(directory, entry["name"])
        if entry["kind"] == "folder":
            line = f"- {full}/"
        elif entry["kind"] == "link":
            line = f"- {full} (enlace; léelo para saber adónde lleva)"
        elif entry["kind"] == "other":
            line = f"- {full} (ni archivo ni carpeta)"
        else:
            line = f"- {full}{_size(entry)}"
        if lines and budget - len(line) - 1 < 0:
            break
        budget -= len(line) + 1
        lines.append(line)
        shown += 1
    following = result.get("next_after_index")
    if shown < len(entries):
        following = start + shown
    if total == 0:
        return f"{directory} está vacía."
    if not entries and start >= total:
        return (
            f"{directory} tiene {total} {'entrada' if total == 1 else 'entradas'}: "
            f"after_index={start} queda después de la última; usa uno menor que {total}."
        )
    header = f"Contenido de {directory} (entradas {start + 1}–{start + shown} de {total}):"
    text = header + "\n" + "\n".join(lines)
    if type(following) is int:
        text += f"\nHay más: sigue con after_index={following}."
    return text


def render_search(result: dict) -> str:
    where = result.get("path") or "todas las carpetas"
    how = "nombres y texto" if result["content"] else "nombres"
    hits = result["results"]
    lines = []
    for hit in hits:
        place = hit["path"] + (f":{hit['line']}" if type(hit.get("line")) is int else "")
        if isinstance(hit.get("snippet"), str):
            lines.append(f"- {place} — {hit['snippet']}")
        elif hit["kind"] == "folder":
            lines.append(f"- {place}/")
        else:
            lines.append(f"- {place}{_size(hit)}")
    header = (
        f"Búsqueda de «{result['query']}» en {where} ({how}): {len(hits)} "
        f"{'resultado' if len(hits) == 1 else 'resultados'}; "
        f"{result['visited']} entradas recorridas"
    )
    if result["content"]:
        header += f", {result['files_read']} archivos y {result['bytes_read']} bytes leídos"
    text = header + ".\n" + ("\n".join(lines) if lines else "Nada coincide.")
    if result["partial_files"]:
        text += (
            f"\n{result['partial_files']} archivos se leyeron sólo en su primera parte; una "
            "coincidencia más adentro no aparece."
        )
    stopped = result.get("stopped")
    if isinstance(stopped, dict) and stopped.get("cause") in STOPS:
        text += "\n" + STOPS[stopped["cause"]].format(limit=stopped.get("limit"))
    return text


def render_read(result: dict) -> str:
    path = result["path"]
    size = result["size_bytes"]
    if result["content"] == "text":
        start = result["after_index"]
        body = result["text"]
        end = start + len(body.encode("utf-8"))
        header = f"{path} · bytes {start}–{end} de {size}:\n"
        following = result.get("next_after_index")
        footer = f"\nSigue con after_index={following}." if type(following) is int else ""
        return header + body + footer
    if result["content"] == "pdf":
        count = result["page_count"]
        parts = [f"{path} · PDF de {count} páginas ({size} bytes):"]
        for page in result["pages"]:
            if page["text"].strip():
                parts.append(f"[Página {page['page']} de {count}]\n{page['text']}")
            else:
                parts.append(f"[Página {page['page']} de {count}: sin texto]")
        following = result.get("next_after_page")
        if type(following) is int:
            parts.append(
                f"Sigue con after_page={following} y after_index={result.get('next_after_index') or 0}."
            )
        return "\n".join(parts)
    return (
        f"{path}: imagen de {result['source_width']} × {result['source_height']} píxeles, "
        f"enviada a {result['width']} × {result['height']} ({size} bytes en el Mac)."
    )
