"""Files attached to a turn: upload, ceilings, and what reaches the agent.

The hub is the real app with its real store; Hermes is the loopback synthetic
gateway of `test_agents`, which stages images as tui_gateway does. Every file
is invented: tiny PNGs, PDFs written by `support_pdf` and short texts.
"""

import base64
import hashlib
import logging
import sqlite3
import struct
import time
import zlib
from urllib.parse import quote

import pytest
from conftest import device
from fastapi.testclient import TestClient
from support_pdf import pdf_bytes
from test_agents import SyntheticHermes, hub, send, socket, until

from orbit_hub import attachments
from orbit_hub.app import create_app
from orbit_hub.attachments import KIND_IMAGE, KIND_PDF, Stored, turn_parts

PNG = (
    b"\x89PNG\r\n\x1a\n"
    + struct.pack(">I", 13)
    + b"IHDR"
    + struct.pack(">IIBBBBB", 1, 1, 8, 2, 0, 0, 0)
    + struct.pack(">I", zlib.crc32(b"IHDR" + struct.pack(">IIBBBBB", 1, 1, 8, 2, 0, 0, 0)))
    + b"\x00\x00\x00\x00IEND\xaeB`\x82"
)
NOTES = "Lista de la excursión al Montseny:\n- botas\n- agua\n".encode()


def headers(paired, name, body):
    return {
        "Authorization": "Bearer " + paired["token"],
        "X-Orbit-Attachment-Name": quote(name, safe=""),
        "X-Orbit-SHA256": hashlib.sha256(body).hexdigest(),
    }


def upload(http, paired, name, body):
    return http.post("/v1/attachments", content=body, headers=headers(paired, name, body))


def uploaded(http, paired, name, body) -> dict:
    response = upload(http, paired, name, body)
    assert response.status_code == 200, response.json()
    return response.json()["attachment"]


def refusal(response) -> dict:
    return response.json()["refusal"]


# Upload ---------------------------------------------------------------------


@pytest.mark.parametrize(
    ("name", "body", "kind", "mime"),
    [
        ("foto.png", PNG, "image", "image/png"),
        (
            "informe.pdf",
            pdf_bytes(["Primera página del informe de la excursión."]),
            "pdf",
            "application/pdf",
        ),
        ("notas.md", NOTES, "text", "text/plain"),
    ],
    ids=["image", "pdf", "text"],
)
def test_an_upload_is_kept_encrypted_and_reads_back_whole(environment, name, body, kind, mime):
    settings, store, adapter = environment
    paired = device(store)
    with hub(settings, store, adapter) as http:
        attachment = uploaded(http, paired, name, body)
        read = http.get(
            f"/v1/attachments/{attachment['id']}",
            headers={"Authorization": "Bearer " + paired["token"]},
        )
    assert {key: attachment[key] for key in ("name", "kind", "mime", "bytes")} == {
        "name": name,
        "kind": kind,
        "mime": mime,
        "bytes": len(body),
    }
    assert read.status_code == 200
    assert read.content == body
    assert read.headers["content-type"].startswith(mime)
    # Neither the bytes nor the name are in the database in the clear.
    raw = (settings.data_dir / "hub.sqlite3").read_bytes()
    assert body[:24] not in raw and name.encode() not in raw


def test_another_device_cannot_read_an_attachment(environment):
    settings, store, adapter = environment
    owner, other = device(store), device(store, "otro")
    with hub(settings, store, adapter) as http:
        attachment = uploaded(http, owner, "foto.png", PNG)
        read = http.get(
            f"/v1/attachments/{attachment['id']}",
            headers={"Authorization": "Bearer " + other["token"]},
        )
    assert read.status_code == 404
    assert refusal(read)["code"] == "attachment_not_found"


def test_a_file_over_its_ceiling_is_refused_with_its_name_size_and_ceiling(environment):
    settings, store, adapter = environment
    paired = device(store)
    body = b"a" * (attachments.MAX_ATTACHMENT_BYTES + 1)
    with hub(settings, store, adapter) as http:
        response = upload(http, paired, "vídeo largo.txt", body)
    assert response.status_code == 413
    assert refusal(response) == {
        "code": "attachments_too_large",
        "ceiling": "file",
        "name": "vídeo largo.txt",
        "bytes": attachments.MAX_ATTACHMENT_BYTES + 1,
        "max_bytes": attachments.MAX_ATTACHMENT_BYTES,
        "retry": {"kind": "after_fix"},
    }
    assert "vídeo largo.txt" in response.json()["message"]


@pytest.mark.parametrize(
    ("name", "body", "seen"),
    [
        ("fotos.zip", b"PK\x03\x04" + b"\x00" * 64, "application/zip"),
        ("IMG_0001.HEIC", b"\x00\x00\x00\x18ftypheic" + b"\x00" * 64, "image/heic"),
        ("programa.bin", bytes(range(256)), "application/octet-stream"),
    ],
    ids=["zip", "heic", "binary"],
)
def test_a_type_no_agent_reads_is_refused_at_upload_naming_it(environment, name, body, seen):
    settings, store, adapter = environment
    paired = device(store)
    with hub(settings, store, adapter) as http:
        response = upload(http, paired, name, body)
    assert response.status_code == 415
    assert refusal(response) == {
        "code": "attachment_type_unsupported",
        "name": name,
        "mime": seen,
        "retry": {"kind": "after_fix"},
    }


def test_a_pdf_pdfium_cannot_open_is_refused_at_upload(environment):
    settings, store, adapter = environment
    paired = device(store)
    with hub(settings, store, adapter) as http:
        response = upload(http, paired, "roto.pdf", b"%PDF-1.4\nesto no es un PDF")
    assert response.status_code == 422
    assert refusal(response) == {
        "code": "attachment_unreadable",
        "name": "roto.pdf",
        "cause": "pdf_damaged",
        "retry": {"kind": "after_fix"},
    }


def test_a_digest_that_does_not_match_is_refused(environment):
    settings, store, adapter = environment
    paired = device(store)
    sent = headers(paired, "foto.png", PNG)
    sent["X-Orbit-SHA256"] = hashlib.sha256(b"otra cosa").hexdigest()
    with hub(settings, store, adapter) as http:
        response = http.post("/v1/attachments", content=PNG, headers=sent)
    assert response.status_code == 400
    assert refusal(response)["code"] == "invalid_attachment"


def test_a_file_never_sent_is_deleted_after_an_hour(environment):
    settings, store, _ = environment
    owner = device(store)["device_id"]
    stale = store.attachments.put(owner, "vieja.txt", NOTES, now_ms=0)
    fresh = store.attachments.put(owner, "nueva.txt", NOTES, now_ms=attachments.UNBOUND_TTL_MS + 1)
    with sqlite3.connect(settings.data_dir / "hub.sqlite3") as db:
        kept = {row[0] for row in db.execute("SELECT id FROM attachments")}
    assert kept == {fresh["id"]}
    assert stale["id"] not in kept


def test_a_file_never_sent_is_deleted_at_its_time_without_another_upload(environment, monkeypatch):
    """The Mac that uploaded it may never upload again: the hub deletes it
    on its own, one sweep for every file, while it runs."""
    settings, store, adapter = environment
    monkeypatch.setattr(attachments, "UNBOUND_SWEEP_SECONDS", 0.01)
    owner = device(store)["device_id"]
    clock = [1000.0]
    waiting = store.attachments.put(owner, "olvidada.txt", NOTES, now_ms=1_000_000)
    app = create_app(settings, store=store, assistant=adapter, speech=adapter, now=lambda: clock[0])

    def kept():
        with sqlite3.connect(settings.data_dir / "hub.sqlite3") as db:
            return {row[0] for row in db.execute("SELECT id FROM attachments")}

    with TestClient(app, base_url="https://localhost"):
        # Swept within its hour: still there.
        time.sleep(0.1)
        assert kept() == {waiting["id"]}
        clock[0] += attachments.UNBOUND_TTL_MS / 1000 + 1
        deadline = time.monotonic() + 5
        while kept() and time.monotonic() < deadline:
            time.sleep(0.02)
        assert kept() == set()


# What a turn says of its files -------------------------------------------------


def stored_pdf(attachment_id: str, name: str, chars: int) -> Stored:
    page = ("Presupuesto de la excursión al lago. " * (chars // 37 + 1))[:chars]
    return Stored(
        id=attachment_id,
        name=name,
        kind=KIND_PDF,
        mime="application/pdf",
        size=10_000,
        pages=1,
        body=b"%PDF-1.4",
        page_texts=(page,),
    )


def line_of(said: str, start: str) -> str:
    return next(line for line in said.splitlines() if line.startswith(start))


def test_a_pdf_past_the_turn_s_text_budget_is_not_said_to_go_below():
    # The first PDF uses the whole turn budget; none of the second's text goes.
    first = stored_pdf(
        "a0000000-0000-4000-8000-000000000001", "uno.pdf", attachments.MAX_TURN_PDF_CHARS
    )
    second = stored_pdf("a0000000-0000-4000-8000-000000000002", "dos.pdf", 500)
    said, carried = turn_parts([first, second], images=False)
    assert carried == []
    assert "[Texto de «dos.pdf»]" not in said
    line = line_of(said, "2. «dos.pdf»")
    assert "su texto va abajo" not in line, line
    assert "«dos.pdf» · PDF of 1 page ·" in line, line
    assert "orbit_attachment" in line, line
    assert f"{attachments.MAX_TURN_PDF_CHARS}" in line, line


def test_an_image_for_an_agent_without_images_is_not_sent_to_orbit_attachment():
    # orbit_attachment hands such an agent a note in place of the image
    # (`assistant/openai.py`), so the turn must not tell it to look there.
    image = Stored(
        id="a0000000-0000-4000-8000-000000000003",
        name="mapa.png",
        kind=KIND_IMAGE,
        mime="image/png",
        size=len(PNG),
        pages=None,
        body=PNG,
        page_texts=None,
    )
    said, carried = turn_parts([image], images=False)
    assert carried == []
    line = line_of(said, "1. «mapa.png»")
    assert "míralo con orbit_attachment" not in line, line
    assert "it cannot view it" in line, line


# Turns ----------------------------------------------------------------------


def test_an_image_reaches_hermes_with_image_attach_bytes_before_the_prompt(environment, tmp_path):
    settings, store, adapter = environment
    paired = device(store)
    auth = {"Authorization": "Bearer " + paired["token"]}
    with SyntheticHermes(tmp_path) as hermes:
        configured = hermes.settings(settings)
        with hub(configured, store, adapter) as http, socket(http, paired) as ws:
            ready = ws.receive_json()["data"]
            image = uploaded(http, paired, "foto del mapa.png", PNG)
            turn = send(ws, "¿Qué ruta marca?", attachments=[image["id"]])
            done = until(ws, turn, "chat.completed", "error")
            page = http.get(f"/v1/sessions/{done['data']['session_id']}/messages", headers=auth)
    assert ready["attachments"] == attachments.limits()
    assert done["event"] == "chat.completed", done
    methods = [method for _, method, _ in hermes.calls]
    assert methods == ["session.create", "image.attach_bytes", "prompt.submit"]
    [(_, _, staged)] = [call for call in hermes.calls if call[1] == "image.attach_bytes"]
    assert staged["content_base64"] == "data:image/png;base64," + base64.b64encode(PNG).decode()
    assert staged["ext"] == "png"
    [(_, _, prompt)] = [call for call in hermes.calls if call[1] == "prompt.submit"]
    assert "¿Qué ruta marca?" in prompt["text"]
    assert "«foto del mapa.png» · image image/png" in prompt["text"]
    assert image["id"] in prompt["text"]
    [user, _answer] = page.json()["messages"]
    assert user["text"] == "¿Qué ruta marca?"
    assert user["attachments"] == [image]


def test_an_image_hermes_refuses_stops_the_turn_and_names_the_file(environment, tmp_path):
    settings, store, adapter = environment
    paired = device(store)
    records: list[logging.LogRecord] = []
    handler = logging.Handler()
    handler.emit = records.append
    logging.getLogger("orbit_hub").addHandler(handler)
    with SyntheticHermes(tmp_path) as hermes:
        hermes.image_refusal = 4018
        configured = hermes.settings(settings)
        with hub(configured, store, adapter) as http, socket(http, paired) as ws:
            image = uploaded(http, paired, "panorámica.png", PNG)
            turn = send(ws, "Mira", attachments=[image["id"]])
            session = until(ws, turn, "chat.session")["data"]["session_id"]
            refused = until(ws, turn, "error")
            # Nothing reached the agent: the conversation takes the next turn.
            hermes.image_refusal = None
            again = uploaded(http, paired, "panorámica.png", PNG)
            retried = until(
                ws,
                send(ws, "Mira", session_id=session, attachments=[again["id"]]),
                "chat.completed",
                "error",
            )
    assert refused["data"]["code"] == "attachment_refused"
    assert "panorámica.png" in refused["data"]["message"]
    assert "25 MiB" in refused["data"]["message"]
    assert refused["data"]["refusal"]["upstream_code"] == 4018
    assert retried["event"] == "chat.completed", retried
    assert [method for _, method, _ in hermes.calls].count("prompt.submit") == 1
    logging.getLogger("orbit_hub").removeHandler(handler)
    # The file's name goes to the Mac, never to the hub's log.
    [logged] = [
        record for record in records if getattr(record, "event", None) == "chat_send_rejected"
    ]
    assert logged.numbers == {"upstream_code": 4018}
    assert all("panorámica" not in str(record.__dict__) for record in records)


def test_an_image_hermes_does_not_confirm_stops_the_turn_and_names_the_file(environment, tmp_path):
    settings, store, adapter = environment
    paired = device(store)
    with SyntheticHermes(tmp_path) as hermes:
        hermes.image_reply = {"attached": False}
        configured = hermes.settings(settings)
        with hub(configured, store, adapter) as http, socket(http, paired) as ws:
            image = uploaded(http, paired, "panorámica.png", PNG)
            refused = until(ws, send(ws, "Mira", attachments=[image["id"]]), "error")
    assert refused["data"]["code"] == "attachment_refused"
    assert "panorámica.png" in refused["data"]["message"]
    # The Mac words the refusal from its numbers: the file goes there too.
    assert refused["data"]["refusal"]["name"] == "panorámica.png"
    assert "prompt.submit" not in [method for _, method, _ in hermes.calls]


def test_a_pdf_goes_as_its_text_and_its_pages_without_text_as_images(environment, tmp_path):
    settings, store, adapter = environment
    paired = device(store)
    document = pdf_bytes(["Presupuesto de la excursión: 120 euros por persona.", ""])
    with SyntheticHermes(tmp_path) as hermes:
        configured = hermes.settings(settings)
        with hub(configured, store, adapter) as http, socket(http, paired) as ws:
            pdf = uploaded(http, paired, "presupuesto.pdf", document)
            done = until(ws, send(ws, "Resume", attachments=[pdf["id"]]), "chat.completed", "error")
    assert done["event"] == "chat.completed", done
    assert pdf["pages"] == 2
    [(_, _, prompt)] = [call for call in hermes.calls if call[1] == "prompt.submit"]
    assert "«presupuesto.pdf» · PDF of 2 pages" in prompt["text"]
    assert "--- page 1 ---\nPresupuesto de la excursión: 120 euros por persona." in prompt["text"]
    assert "pages with almost no text (2) are included as images" in prompt["text"]
    [(_, _, page)] = [call for call in hermes.calls if call[1] == "image.attach_bytes"]
    png = base64.b64decode(page["content_base64"].removeprefix("data:image/png;base64,"))
    assert png.startswith(b"\x89PNG\r\n\x1a\n")
    width, height = struct.unpack(">II", png[16:24])
    assert max(width, height) == attachments.PAGE_IMAGE_LONG_SIDE


def test_a_text_file_is_named_for_orbit_attachment(environment, tmp_path):
    settings, store, adapter = environment
    paired = device(store)
    with SyntheticHermes(tmp_path) as hermes:
        configured = hermes.settings(settings)
        with hub(configured, store, adapter) as http, socket(http, paired) as ws:
            notes = uploaded(http, paired, "lista.md", NOTES)
            until(ws, send(ws, "", attachments=[notes["id"]]), "chat.completed")
    [(_, _, prompt)] = [call for call in hermes.calls if call[1] == "prompt.submit"]
    assert (
        f"«lista.md» · texto · {len(NOTES)} B · id {notes['id']}: read it with orbit_attachment"
        in (prompt["text"])
    )
    assert "botas" not in prompt["text"]
    assert "image.attach_bytes" not in [method for _, method, _ in hermes.calls]


def test_more_files_than_a_message_takes_are_refused_with_the_count(environment, tmp_path):
    settings, store, adapter = environment
    paired = device(store)
    with SyntheticHermes(tmp_path) as hermes:
        configured = hermes.settings(settings)
        with hub(configured, store, adapter) as http, socket(http, paired) as ws:
            ids = [
                uploaded(http, paired, f"nota {n}.txt", NOTES)["id"]
                for n in range(attachments.MAX_MESSAGE_ATTACHMENTS + 1)
            ]
            refused = until(ws, send(ws, "Todas", attachments=ids), "error")
    assert refused["data"]["refusal"] == {
        "code": "attachments_too_many",
        "files": attachments.MAX_MESSAGE_ATTACHMENTS + 1,
        "max_files": attachments.MAX_MESSAGE_ATTACHMENTS,
        "retry": {"kind": "after_fix"},
    }
    assert [method for _, method, _ in hermes.calls] == []


def test_a_message_over_its_bytes_is_refused_with_its_sum(environment, tmp_path, monkeypatch):
    monkeypatch.setattr(attachments, "MAX_MESSAGE_ATTACHMENT_BYTES", 2 * len(NOTES) - 1)
    settings, store, adapter = environment
    paired = device(store)
    with SyntheticHermes(tmp_path) as hermes:
        configured = hermes.settings(settings)
        with hub(configured, store, adapter) as http, socket(http, paired) as ws:
            ids = [uploaded(http, paired, f"nota {n}.txt", NOTES)["id"] for n in range(2)]
            refused = until(ws, send(ws, "Las dos", attachments=ids), "error")
    assert refused["data"]["refusal"] == {
        "code": "attachments_too_large",
        "ceiling": "message",
        "files": 2,
        "bytes": 2 * len(NOTES),
        "max_bytes": 2 * len(NOTES) - 1,
        "retry": {"kind": "after_fix"},
    }


def test_a_conversation_over_its_bytes_is_refused_with_what_it_holds(
    environment, tmp_path, monkeypatch
):
    monkeypatch.setattr(attachments, "MAX_SESSION_ATTACHMENT_BYTES", len(NOTES) + 1)
    settings, store, adapter = environment
    paired = device(store)
    with SyntheticHermes(tmp_path) as hermes:
        configured = hermes.settings(settings)
        with hub(configured, store, adapter) as http, socket(http, paired) as ws:
            first = uploaded(http, paired, "una.txt", NOTES)["id"]
            done = until(ws, send(ws, "Una", attachments=[first]), "chat.completed")
            session = done["data"]["session_id"]
            second = uploaded(http, paired, "otra.txt", NOTES)["id"]
            refused = until(ws, send(ws, "Otra", session_id=session, attachments=[second]), "error")
    assert refused["data"]["refusal"] == {
        "code": "attachments_too_large",
        "ceiling": "session",
        "bytes": len(NOTES),
        "session_bytes": len(NOTES),
        "max_bytes": len(NOTES) + 1,
        "retry": {"kind": "after_fix"},
    }


def test_an_attachment_is_sent_once_and_only_by_its_device(environment, tmp_path):
    settings, store, adapter = environment
    owner, other = device(store), device(store, "otro")
    with SyntheticHermes(tmp_path) as hermes:
        configured = hermes.settings(settings)
        with hub(configured, store, adapter) as http:
            image = uploaded(http, owner, "foto.png", PNG)
            with socket(http, other) as ws:
                foreign = until(ws, send(ws, "Mía", attachments=[image["id"]]), "error")
            with socket(http, owner) as ws:
                until(ws, send(ws, "Una vez", attachments=[image["id"]]), "chat.completed")
                again = until(ws, send(ws, "Otra vez", attachments=[image["id"]]), "error")
    for refused in (foreign, again):
        assert refused["data"]["refusal"] == {
            "code": "attachment_not_found",
            "attachment_id": image["id"],
            "retry": {"kind": "after_fix"},
        }


def test_a_turn_without_attachments_keeps_the_0_9_0_shape(environment, tmp_path):
    """A 0.9.0 Mac sends no `attachments` and reads pages without them: its
    turns and its conversation's page are what they were."""
    settings, store, adapter = environment
    paired = device(store)
    auth = {"Authorization": "Bearer " + paired["token"]}
    with SyntheticHermes(tmp_path) as hermes:
        configured = hermes.settings(settings)
        with hub(configured, store, adapter) as http, socket(http, paired) as ws:
            ws.receive_json()
            done = until(ws, send(ws, "Hola"), "chat.completed")
            page = http.get(f"/v1/sessions/{done['data']['session_id']}/messages", headers=auth)
    [(_, _, prompt)] = [call for call in hermes.calls if call[1] == "prompt.submit"]
    assert set(prompt) == {"session_id", "text"}
    assert "Adjuntos" not in prompt["text"]
    # No attachments anywhere. The answer adds only its turn's `work`
    # summary, which a Mac up to 0.11 ignores (its `HubMessage` does not
    # refuse unknown fields).
    assert [set(message) for message in page.json()["messages"]] == [
        {"id", "role", "text", "at_ms", "kind"},
        {"id", "role", "text", "at_ms", "kind", "work"},
    ]


def test_an_empty_text_needs_an_attachment(environment, tmp_path):
    settings, store, adapter = environment
    paired = device(store)
    with hub(settings, store, adapter) as http, socket(http, paired) as ws:
        refused = until(ws, send(ws, ""), "error")
    assert refused["data"]["code"] == "invalid_request"


# «Adjuntar ventana» ---------------------------------------------------------


def put_capture(http, paired, capture_id, request_id, body=PNG):
    return http.put(
        f"/v1/captures/{capture_id}",
        content=body,
        headers={
            "Authorization": "Bearer " + paired["token"],
            "Content-Type": "image/png",
            "X-Orbit-Request": request_id,
            "X-Orbit-SHA256": hashlib.sha256(body).hexdigest(),
        },
    )


def test_a_window_capture_becomes_an_attachment_once(environment):
    settings, store, adapter = environment
    owner, other = device(store), device(store, "otro")
    capture_id, request_id = "11111111-1111-4111-8111-111111111111", "window-synthetic"
    ask = {"capture_id": capture_id, "request_id": request_id, "name": "Notas — Lista.png"}
    with hub(settings, store, adapter) as http:
        assert put_capture(http, owner, capture_id, request_id).status_code == 200
        foreign = http.post(
            "/v1/attachments/capture",
            json=ask,
            headers={"Authorization": "Bearer " + other["token"]},
        )
        kept = http.post(
            "/v1/attachments/capture",
            json=ask,
            headers={"Authorization": "Bearer " + owner["token"]},
        )
        again = http.post(
            "/v1/attachments/capture",
            json=ask,
            headers={"Authorization": "Bearer " + owner["token"]},
        )
        read = http.get(
            f"/v1/attachments/{kept.json()['attachment']['id']}",
            headers={"Authorization": "Bearer " + owner["token"]},
        )
    assert foreign.status_code == 404
    assert refusal(foreign)["code"] == "capture_missing"
    assert kept.status_code == 200, kept.json()
    assert {key: kept.json()["attachment"][key] for key in ("name", "kind", "mime", "bytes")} == {
        "name": "Notas — Lista.png",
        "kind": "image",
        "mime": "image/png",
        "bytes": len(PNG),
    }
    assert read.content == PNG
    assert again.status_code == 404
    assert refusal(again) == {
        "code": "capture_missing",
        "capture_id": capture_id,
        "retry": {"kind": "never"},
    }
