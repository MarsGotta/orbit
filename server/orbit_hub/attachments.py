"""Files the user attaches to a chat turn.

The Mac reads each file by its path and uploads it here over HTTP
(`POST /v1/attachments`), as it does audio: a chat frame is at most 128 KiB.
The hub keeps the file encrypted under an id, unbound, until a `chat.send`
names that id; the turn then binds it to its message and so to its session,
the conversation's single source. What reaches the agent depends on the file:

- an image goes as an image to an agent that takes images (Hermes through
  `image.attach_bytes`, an ACP agent as an image content block);
- a PDF goes as its text, extracted here with pypdfium2, and the pages that
  have almost no text (a scan, a figure) as images;
- a text file is named in the turn and read with the `orbit_attachment` tool
  of orbit-mcp when the agent needs it.

Anything else cannot reach an agent in a form it reads, so the upload is
refused at once, naming the file and the type: the user hears it while the
attachment is still a chip in the composer, before sending.
"""

from __future__ import annotations

import hashlib
import json
import struct
import threading
import unicodedata
import zlib
from dataclasses import dataclass
from uuid import uuid4

from .errors import HubError

# One file. What it protects: the hub's memory, since an upload is read
# whole before it is encrypted and stored, and the Mac's, which reads the
# file by path and holds it while it uploads. Why 20 MiB: Hermes refuses an
# image over 25 MiB (`_ATTACH_BYTES_MAX_BYTES`, tui_gateway/
# prompt_attachments.py, read 2026-10-22), and an image passes to it whole;
# 20 MiB holds a phone photo or a long PDF with margin. What it is not: a
# limit on a conversation or on the text a PDF yields.
MAX_ATTACHMENT_BYTES = 20 * 1024 * 1024
# One message's files. What it protects: one turn's prompt, which carries
# every image at once, and the agent's context. Why 10: a handful of files is
# what a person attaches to one question; it was not measured. It does not
# limit the files of a conversation.
MAX_MESSAGE_ATTACHMENTS = 10
# One message's bytes, added over its files. What it protects: the hub's
# memory while it builds one turn (every file decrypted, images base64-encoded
# for the agent: 32 MiB become about 43 MiB in an ACP prompt line). Why
# 32 MiB: one full-size file and a few small ones; not measured. It does not
# limit a single file, which has its own ceiling.
MAX_MESSAGE_ATTACHMENT_BYTES = 32 * 1024 * 1024
# One conversation's bytes, every message's files added. What it protects: the
# hub's disk under its 10 GiB quota (`Store(quota_bytes=…)`), so one
# conversation cannot fill it with files. Why 256 MiB: a dozen full-size files
# or hundreds of photos; not measured. It is not the hub's quota, which still
# applies to everything.
MAX_SESSION_ATTACHMENT_BYTES = 256 * 1024 * 1024
# A file name as the user gave it. It bounds what the chip, the turn and every
# refusal echo back; 255 is the longest name macOS allows for one file.
MAX_NAME_CHARS = 255
# How long an uploaded file waits for the `chat.send` that names it. A chip the
# user removes, or a message never sent, leaves a file nobody will bind; it is
# deleted after this long. One hour outlasts any composing of one message.
UNBOUND_TTL_MS = 60 * 60 * 1000
# A page with fewer characters of text than this is a scan or a figure: it
# goes to the agent as an image. 32 is a page number and a header; a page of
# prose has hundreds. Not measured against real documents.
MIN_PAGE_TEXT_CHARS = 32
# PDF pages sent as images in one turn. What it protects: the agent's
# context, since each image costs it about 1,600 tokens at the size below.
# Why 8: the first pages of a scanned document; the rest is named and the
# agent asks for it. Not measured.
MAX_PDF_PAGE_IMAGES = 8
# The longest side of a rendered page, in pixels. Agents shrink larger images
# anyway (Claude to 1,568 px on its long side), so a larger render only costs
# bytes.
PAGE_IMAGE_LONG_SIDE = 1568
# Characters of PDF text inside one turn, every PDF of the message added. What
# it protects: the agent's context, the same budget as a page of a meeting
# read (`meetings_bridge.page.MAX_ANSWER_CHARS`, about 12,600 tokens). The rest
# stays one `orbit_attachment` call away, from the index the turn names.
MAX_TURN_PDF_CHARS = 48_000

KIND_IMAGE, KIND_PDF, KIND_TEXT = "image", "pdf", "text"
# The images an agent can take: what Hermes stages (`_allowed_image_extensions`
# without BMP, which no ACP agent names) and what the Claude and Codex
# adapters pass on.
_IMAGES = (
    (b"\x89PNG\r\n\x1a\n", "image/png"),
    (b"\xff\xd8\xff", "image/jpeg"),
    (b"GIF87a", "image/gif"),
    (b"GIF89a", "image/gif"),
)
# Types recognised only to name them in a refusal.
_NAMED = (
    (b"PK\x03\x04", "application/zip"),
    (b"\x1f\x8b", "application/gzip"),
    (b"7z\xbc\xaf\x27\x1c", "application/x-7z-compressed"),
    (b"\xd0\xcf\x11\xe0", "application/x-ole-storage"),
    (b"ID3", "audio/mpeg"),
    (b"fLaC", "audio/flac"),
    (b"OggS", "audio/ogg"),
    (b"BM", "image/bmp"),
    (b"II*\x00", "image/tiff"),
    (b"MM\x00*", "image/tiff"),
)
# pdfium is not thread-safe: one document is open at a time in this process.
_PDFIUM = threading.Lock()

SCHEMA = """
CREATE TABLE IF NOT EXISTS attachments (
    id TEXT PRIMARY KEY, device_id TEXT NOT NULL REFERENCES devices(id),
    session_id TEXT REFERENCES sessions(id), message_id INTEGER REFERENCES messages(id),
    kind TEXT NOT NULL, mime TEXT NOT NULL, size INTEGER NOT NULL, sha256 TEXT NOT NULL,
    pages INTEGER, name BLOB NOT NULL, body BLOB NOT NULL, extract BLOB,
    created_ms INTEGER NOT NULL);
CREATE INDEX IF NOT EXISTS attachments_session ON attachments(session_id);
CREATE INDEX IF NOT EXISTS attachments_message ON attachments(message_id);
CREATE INDEX IF NOT EXISTS attachments_unbound ON attachments(device_id, message_id);
"""


def limits() -> dict:
    """The ceilings, as `ready` tells the Mac."""
    return {
        "max_file_bytes": MAX_ATTACHMENT_BYTES,
        "max_message_files": MAX_MESSAGE_ATTACHMENTS,
        "max_message_bytes": MAX_MESSAGE_ATTACHMENT_BYTES,
        "max_session_bytes": MAX_SESSION_ATTACHMENT_BYTES,
    }


def valid_name(name: str) -> str:
    """The file's name, refused when it cannot be shown or echoed."""
    name = unicodedata.normalize("NFC", name).strip()
    if (
        not 1 <= len(name) <= MAX_NAME_CHARS
        or any(unicodedata.category(character) == "Cc" for character in name)
        or "/" in name
    ):
        raise HubError(
            "invalid_attachment",
            f"El nombre del adjunto no vale: 1 a {MAX_NAME_CHARS} caracteres, sin control ni «/»",
            numbers={"field": "name", "characters": len(name), "max_characters": MAX_NAME_CHARS},
        )
    return name


def classify(name: str, data: bytes) -> tuple[str, str]:
    """`(kind, mime)` of a file an agent can read, by its content, never its name.

    Refused with the file's name and what it looks like otherwise.
    """
    for magic, mime in _IMAGES:
        if data.startswith(magic):
            return KIND_IMAGE, mime
    if data[:4] == b"RIFF" and data[8:12] == b"WEBP":
        return KIND_IMAGE, "image/webp"
    if data.startswith(b"%PDF-"):
        return KIND_PDF, "application/pdf"
    if b"\x00" not in data:
        try:
            data.decode("utf-8")
        except UnicodeDecodeError:
            pass
        else:
            return KIND_TEXT, "text/plain"
    raise HubError(
        "attachment_type_unsupported",
        f"«{name}» es {_seen_type(data)}: ningún agente lo lee. Se pueden adjuntar imágenes "
        "PNG, JPEG, GIF o WebP, PDF y ficheros de texto",
        http_status=415,
        numbers={"name": name, "mime": _seen_type(data)},
    )


def _seen_type(data: bytes) -> str:
    if data[4:8] == b"ftyp":
        brand = data[8:12]
        if brand in {b"heic", b"heix", b"mif1", b"msf1", b"hevc"}:
            return "image/heic"
        return "video/mp4"
    for magic, mime in _NAMED:
        if data.startswith(magic):
            return mime
    return "application/octet-stream"


@dataclass(frozen=True)
class Stored:
    """One attachment as a turn or a read needs it."""

    id: str
    name: str
    kind: str
    mime: str
    size: int
    pages: int | None
    body: bytes
    # The text of each page of a PDF; None for other kinds.
    page_texts: tuple[str, ...] | None

    def public(self) -> dict:
        return _public(self.id, self.name, self.kind, self.mime, self.size, self.pages)


def _public(attachment_id, name, kind, mime, size, pages) -> dict:
    value = {"id": attachment_id, "name": name, "kind": kind, "mime": mime, "bytes": size}
    if pages is not None:
        value["pages"] = pages
    return value


def pdf_pages(name: str, data: bytes) -> tuple[str, ...]:
    """The text of each page, or a refusal naming the file when pdfium cannot open it."""
    import pypdfium2

    with _PDFIUM:
        try:
            document = pypdfium2.PdfDocument(data)
        except pypdfium2.PdfiumError as exc:
            locked = "password" in str(exc).lower()
            raise HubError(
                "attachment_unreadable",
                f"«{name}» "
                + ("está protegido con contraseña" if locked else "no es un PDF que se pueda leer"),
                http_status=422,
                numbers={"name": name, "cause": "pdf_locked" if locked else "pdf_damaged"},
            ) from None
        try:
            texts = []
            for index in range(len(document)):
                page = document[index]
                text_page = page.get_textpage()
                texts.append(text_page.get_text_range().replace("\r\n", "\n").strip())
                text_page.close()
                page.close()
            return tuple(texts)
        finally:
            document.close()


def render_pages(data: bytes, indexes: list[int]) -> list[bytes]:
    """PNG images of the pages at `indexes`, the long side PAGE_IMAGE_LONG_SIDE pixels."""
    import pypdfium2

    images = []
    with _PDFIUM:
        document = pypdfium2.PdfDocument(data)
        try:
            for index in indexes:
                page = document[index]
                width, height = page.get_size()
                scale = PAGE_IMAGE_LONG_SIDE / max(width, height, 1)
                bitmap = page.render(scale=scale, rev_byteorder=True)
                images.append(
                    _png(
                        bitmap.buffer, bitmap.width, bitmap.height, bitmap.stride, bitmap.n_channels
                    )
                )
                bitmap.close()
                page.close()
        finally:
            document.close()
    return images


def _png(buffer, width: int, height: int, stride: int, channels: int) -> bytes:
    """A PNG of pdfium's bitmap (RGB or RGBA rows, `stride` bytes apart)."""
    raw = memoryview(buffer)
    color = {3: 2, 4: 6}[channels]
    rows = bytearray()
    for row in range(height):
        start = row * stride
        rows += b"\x00" + raw[start : start + width * channels]

    def chunk(kind: bytes, payload: bytes) -> bytes:
        body = kind + payload
        return struct.pack(">I", len(payload)) + body + struct.pack(">I", zlib.crc32(body))

    header = struct.pack(">IIBBBBB", width, height, 8, color, 0, 0, 0)
    return (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", header)
        + chunk(b"IDAT", zlib.compress(bytes(rows), 6))
        + chunk(b"IEND", b"")
    )


def pdf_text(page_texts: tuple[str, ...]) -> str:
    """A PDF's text as the agent reads it: each page under its number."""
    return "\n\n".join(
        f"--- página {number} ---\n{text}" for number, text in enumerate(page_texts, 1)
    )


def _size(size: int) -> str:
    if size < 1024:
        return f"{size} B"
    if size < 1024 * 1024:
        return f"{size / 1024:.0f} KB"
    return f"{size / (1024 * 1024):.1f} MB".replace(".", ",")


@dataclass(frozen=True)
class TurnImage:
    """An image the turn carries, with the attachment it comes from."""

    attachment_id: str
    name: str
    mime: str
    data: bytes


def turn_parts(attachments: list[Stored], images: bool) -> tuple[str, list[TurnImage]]:
    """What a turn says about its attachments, and the images it carries.

    `images`: the agent takes images in a turn. Without it every image is
    named and read with `orbit_attachment`. Runs pdfium: call it off the loop.
    """
    if not attachments:
        return "", []
    lines = [
        (
            f"[Adjuntos de este mensaje: {len(attachments)}. Cada uno se lee con la "
            "herramienta orbit_attachment de Orbit y su id]"
        )
    ]
    carried: list[TurnImage] = []
    texts: list[str] = []
    budget = MAX_TURN_PDF_CHARS
    page_images = MAX_PDF_PAGE_IMAGES
    for number, attachment in enumerate(attachments, 1):
        head = f"{number}. «{attachment.name}» · "
        tail = f" · {_size(attachment.size)} · id {attachment.id}: "
        if attachment.kind == KIND_IMAGE:
            if images:
                carried.append(
                    TurnImage(attachment.id, attachment.name, attachment.mime, attachment.body)
                )
                said = "va como imagen en este mensaje."
            else:
                said = "este agente no recibe imágenes en el mensaje; míralo con orbit_attachment."
            lines.append(f"{head}imagen {attachment.mime}{tail}{said}")
        elif attachment.kind == KIND_PDF:
            pages = attachment.page_texts or ()
            whole = pdf_text(pages)
            shown = whole[:budget]
            budget -= len(shown)
            said = "su texto va abajo"
            if len(shown) < len(whole):
                said += (
                    f" hasta el carácter {len(shown)} de {len(whole)}; el resto, con "
                    f"orbit_attachment y after_index {len(shown)}"
                )
            if shown:
                texts.append(f"[Texto de «{attachment.name}»]\n{shown}")
            empty = [index for index, text in enumerate(pages) if len(text) < MIN_PAGE_TEXT_CHARS]
            if empty and images and page_images > 0:
                chosen = empty[:page_images]
                page_images -= len(chosen)
                for index, png in zip(chosen, render_pages(attachment.body, chosen), strict=True):
                    carried.append(
                        TurnImage(
                            attachment.id,
                            f"{attachment.name} · página {index + 1}",
                            "image/png",
                            png,
                        )
                    )
                said += "; " + _pages_said(chosen, len(empty))
            elif empty:
                said += f"; {len(empty)} páginas casi sin texto no van como imagen"
            lines.append(f"{head}PDF de {len(pages)} páginas{tail}{said}.")
        else:
            lines.append(f"{head}texto{tail}léelo con orbit_attachment cuando lo necesites.")
    return "\n".join([*lines, *texts]), carried


def _pages_said(chosen: list[int], empty: int) -> str:
    numbers = ", ".join(str(index + 1) for index in chosen)
    said = f"las páginas casi sin texto ({numbers}) van como imagen"
    if empty > len(chosen):
        said += f"; otras {empty - len(chosen)} no, por el techo de {MAX_PDF_PAGE_IMAGES}"
    return said


class AttachmentRepository:
    """The `attachments` table: encrypted name, body and extracted text per id."""

    def __init__(self, store):
        self.store = store

    def put(self, device: str, name: str, data: bytes, now_ms: int) -> dict:
        """Store one upload unbound and return it as the Mac's chip shows it.

        Refused when it is too large, of a type no agent reads, or a PDF
        pdfium cannot open; each refusal names the file. Runs pdfium: call it
        off the loop.
        """
        name = valid_name(name)
        if len(data) > MAX_ATTACHMENT_BYTES:
            raise too_large_file(name, len(data))
        kind, mime = classify(name, data)
        pages = pdf_pages(name, data) if kind == KIND_PDF else None
        attachment_id = str(uuid4())
        store = self.store
        extract = None
        if pages is not None:
            extract = store.encrypt(
                json.dumps(pages, ensure_ascii=False).encode(),
                f"attachment:{attachment_id}:extract",
            )
        with store.transaction() as db:
            # A file that waited longer than an hour for its message will not get one.
            db.execute(
                "DELETE FROM attachments WHERE device_id=? AND message_id IS NULL"
                " AND created_ms < ?",
                (device, now_ms - UNBOUND_TTL_MS),
            )
            store.ensure_capacity(len(data) + (len(extract) if extract else 0) + 1024)
            db.execute(
                "INSERT INTO attachments(id,device_id,kind,mime,size,sha256,pages,name,body,"
                "extract,created_ms) VALUES (?,?,?,?,?,?,?,?,?,?,?)",
                (
                    attachment_id,
                    device,
                    kind,
                    mime,
                    len(data),
                    hashlib.sha256(data).hexdigest(),
                    None if pages is None else len(pages),
                    store.encrypt(name.encode(), f"attachment:{attachment_id}:name"),
                    store.encrypt(data, f"attachment:{attachment_id}:body"),
                    extract,
                    now_ms,
                ),
            )
        return _public(
            attachment_id, name, kind, mime, len(data), None if pages is None else len(pages)
        )

    def admit(self, device: str, session_id: str | None, ids: list[str]) -> list[Stored]:
        """The attachments a turn names, decrypted, or the refusal of the
        ceiling it crosses: per message (count, bytes) or per session."""
        if len(ids) > MAX_MESSAGE_ATTACHMENTS:
            raise HubError(
                "attachments_too_many",
                f"Este mensaje lleva {len(ids)} adjuntos; el techo es "
                f"{MAX_MESSAGE_ATTACHMENTS} por mensaje",
                numbers={"files": len(ids), "max_files": MAX_MESSAGE_ATTACHMENTS},
            )
        if len(set(ids)) != len(ids):
            repeated = next(attachment for attachment in ids if ids.count(attachment) > 1)
            raise HubError(
                "invalid_attachment",
                "El mensaje nombra dos veces el mismo adjunto",
                numbers={"field": "attachments", "attachment_id": repeated},
            )
        store = self.store
        rows = {}
        with store.read_snapshot() as db:
            for attachment_id in ids:
                row = db.execute(
                    "SELECT id,kind,mime,size,pages,name,message_id FROM attachments"
                    " WHERE id=? AND device_id=?",
                    (attachment_id, device),
                ).fetchone()
                if row is None or row["message_id"] is not None:
                    raise HubError(
                        "attachment_not_found",
                        "Un adjunto del mensaje ya no está en el hub (caducó tras una hora sin "
                        "enviarse o ya se envió); vuelve a adjuntarlo",
                        http_status=404,
                        numbers={"attachment_id": attachment_id},
                    )
                rows[attachment_id] = row
            total = sum(row["size"] for row in rows.values())
            if total > MAX_MESSAGE_ATTACHMENT_BYTES:
                raise HubError(
                    "attachments_too_large",
                    f"Los {len(ids)} adjuntos del mensaje suman {_size(total)}; el techo es "
                    f"{_size(MAX_MESSAGE_ATTACHMENT_BYTES)} por mensaje",
                    http_status=413,
                    numbers={
                        "ceiling": "message",
                        "files": len(ids),
                        "bytes": total,
                        "max_bytes": MAX_MESSAGE_ATTACHMENT_BYTES,
                    },
                )
            if session_id is not None:
                held = db.execute(
                    "SELECT COALESCE(SUM(size),0) FROM attachments WHERE session_id=?"
                    " AND device_id=?",
                    (session_id, device),
                ).fetchone()[0]
                if held + total > MAX_SESSION_ATTACHMENT_BYTES:
                    raise HubError(
                        "attachments_too_large",
                        f"Esta conversación ya guarda {_size(held)} de adjuntos y este mensaje "
                        f"añade {_size(total)}; el techo es "
                        f"{_size(MAX_SESSION_ATTACHMENT_BYTES)} por conversación. Abre una "
                        "conversación nueva para adjuntar más",
                        http_status=413,
                        numbers={
                            "ceiling": "session",
                            "bytes": total,
                            "session_bytes": held,
                            "max_bytes": MAX_SESSION_ATTACHMENT_BYTES,
                        },
                    )
        return [self._stored(attachment_id) for attachment_id in ids]

    def _stored(self, attachment_id: str) -> Stored:
        store = self.store
        with store.read_snapshot() as db:
            row = db.execute(
                "SELECT id,kind,mime,size,pages,name,body,extract FROM attachments WHERE id=?",
                (attachment_id,),
            ).fetchone()
        texts = None
        if row["extract"] is not None:
            texts = tuple(
                json.loads(store.decrypt(row["extract"], f"attachment:{attachment_id}:extract"))
            )
        return Stored(
            id=row["id"],
            name=store.decrypt(row["name"], f"attachment:{attachment_id}:name").decode(),
            kind=row["kind"],
            mime=row["mime"],
            size=row["size"],
            pages=row["pages"],
            body=store.decrypt(row["body"], f"attachment:{attachment_id}:body"),
            page_texts=texts,
        )

    def read(self, device: str, attachment_id: str) -> Stored:
        """One of the device's attachments, bound or not; `attachment_not_found` otherwise."""
        with self.store.read_snapshot() as db:
            found = db.execute(
                "SELECT 1 FROM attachments WHERE id=? AND device_id=?", (attachment_id, device)
            ).fetchone()
        if found is None:
            raise HubError(
                "attachment_not_found",
                "Ese adjunto no está en el hub",
                http_status=404,
                numbers={"attachment_id": attachment_id},
            )
        return self._stored(attachment_id)

    @staticmethod
    def bind(db, session_id: str, message_id: int, ids: list[str]) -> None:
        """Bind a turn's attachments to its message, inside the message's own
        transaction; one another turn took meanwhile rolls the message back."""
        for attachment_id in ids:
            bound = db.execute(
                "UPDATE attachments SET session_id=?, message_id=? WHERE id=?"
                " AND message_id IS NULL"
                " AND device_id=(SELECT device_id FROM sessions WHERE id=?)",
                (session_id, message_id, attachment_id, session_id),
            ).rowcount
            if bound != 1:
                raise HubError(
                    "attachment_not_found",
                    "Un adjunto del mensaje ya se envió con otro; vuelve a adjuntarlo",
                    http_status=404,
                    numbers={"attachment_id": attachment_id},
                )

    def of_messages(self, db, message_ids: list[int]) -> dict[int, list[dict]]:
        """Each message's attachments as the conversation's page lists them."""
        found: dict[int, list[dict]] = {}
        if not message_ids:
            return found
        marks = ",".join("?" * len(message_ids))
        rows = db.execute(
            f"SELECT id,message_id,kind,mime,size,pages,name FROM attachments"
            f" WHERE message_id IN ({marks}) ORDER BY created_ms, id",
            message_ids,
        ).fetchall()
        for row in rows:
            name = self.store.decrypt(row["name"], f"attachment:{row['id']}:name").decode()
            found.setdefault(row["message_id"], []).append(
                _public(row["id"], name, row["kind"], row["mime"], row["size"], row["pages"])
            )
        return found


def too_large_file(name: str, size: int) -> HubError:
    return HubError(
        "attachments_too_large",
        f"«{name}» pesa {_size(size)}; el techo es {_size(MAX_ATTACHMENT_BYTES)} por fichero",
        http_status=413,
        numbers={
            "ceiling": "file",
            "name": name,
            "bytes": size,
            "max_bytes": MAX_ATTACHMENT_BYTES,
        },
    )
