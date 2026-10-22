"""`orbit_attachment`: the agent reads a file the user attached.

The translator runs against the real hub app over loopback with the real
signed private route, as in `test_deimos_meeting_reads`. The files are invented.
"""

import base64

from conftest import device
from support_orbit_mcp import DEFAULT_DEVICE, text_of
from support_pdf import pdf_bytes
from test_attachments import NOTES, PNG
from test_deimos_meeting_reads import deimos

from orbit_hub.meetings_bridge.page import MAX_ANSWER_CHARS


async def test_an_attached_text_reads_a_page_at_a_time(environment, tmp_path):
    _, store, _ = environment
    owner = device(store)["device_id"]
    line = "Paso del itinerario por el valle y vuelta al refugio.\n"
    text = line * (MAX_ANSWER_CHARS // len(line) + 20)
    attachment = store.attachments.put(owner, "itinerario.txt", text.encode(), now_ms=1)
    async with deimos(environment, tmp_path, owner) as server:
        first = await server.call_tool(
            "orbit_attachment", {"device": DEFAULT_DEVICE, "attachment_id": attachment["id"]}
        )
        following = int(text_of(first).split("sigue con after_index ")[1].split(".")[0])
        second = await server.call_tool(
            "orbit_attachment",
            {"device": DEFAULT_DEVICE, "attachment_id": attachment["id"], "after_index": following},
        )
    assert not first.is_error and not second.is_error, text_of(first)
    head, page = text_of(first).split("\n", 1)
    assert head.startswith("Adjunto «itinerario.txt» (text/plain, ")
    assert page == text[:following]
    assert text_of(second).split("\n", 1)[1] == text[following:]
    assert "hasta el final" in text_of(second)


async def test_an_attached_image_comes_back_as_an_image(environment, tmp_path):
    _, store, _ = environment
    owner = device(store)["device_id"]
    attachment = store.attachments.put(owner, "foto.png", PNG, now_ms=1)
    async with deimos(environment, tmp_path, owner) as server:
        result = await server.call_tool(
            "orbit_attachment", {"device": DEFAULT_DEVICE, "attachment_id": attachment["id"]}
        )
    assert not result.is_error, text_of(result)
    [image, said] = result.content
    assert (image.type, image.mime_type) == ("image", "image/png")
    assert base64.b64decode(image.data) == PNG
    assert said.text == f"Adjunto «foto.png» (image/png, {len(PNG)} bytes)."


async def test_an_attached_pdf_reads_as_its_text_by_page(environment, tmp_path):
    _, store, _ = environment
    owner = device(store)["device_id"]
    document = pdf_bytes(["Primera parada: el lago.", "Segunda parada: la ermita."])
    attachment = store.attachments.put(owner, "ruta.pdf", document, now_ms=1)
    async with deimos(environment, tmp_path, owner) as server:
        result = await server.call_tool(
            "orbit_attachment", {"device": DEFAULT_DEVICE, "attachment_id": attachment["id"]}
        )
    assert not result.is_error, text_of(result)
    assert (
        "--- página 1 ---\nPrimera parada: el lago.\n\n--- página 2 ---\nSegunda parada: la ermita."
        in text_of(result)
    )


async def test_an_attachment_of_another_device_is_not_found(environment, tmp_path):
    _, store, _ = environment
    owner, other = device(store)["device_id"], device(store, "otro")["device_id"]
    attachment = store.attachments.put(other, "ajena.txt", NOTES, now_ms=1)
    async with deimos(environment, tmp_path, owner) as server:
        result = await server.call_tool(
            "orbit_attachment", {"device": DEFAULT_DEVICE, "attachment_id": attachment["id"]}
        )
    assert result.is_error
    assert text_of(result).startswith("attachment_not_found: Ese adjunto no está en el hub")
