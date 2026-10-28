"""The assistant's folders through orbit-mcp: three read-only tools that ask the
Mac through the signed device bridge and say, in Spanish, what they found or
why they could not read it — the path asked for, where it led and every
folder, so the assistant can correct itself.

The hub here is the loopback fake of `support_orbit_mcp`; the replies are the
shapes the Mac sends (`contracts/hub-v1.md`, «Assistant folders»).
"""

import base64

from fastapi.responses import Response
from support_orbit_mcp import (
    BRIDGE_RESULT,
    FakeHub,
    answered,
    serving,
    text_of,
)

from orbit_hub.private_auth import canonical
from orbit_mcp import files

PROJECTS = "/Users/ejemplo/Proyectos"
NOTES = "/Users/ejemplo/Notas"
SECRET = "/Users/ejemplo/Privado/claves.txt"
PNG = b"\x89PNG\r\n\x1a\nimagen sintetica de una carpeta"


def refused(refusal: dict) -> Response:
    """What the hub relays when the Mac refuses (`bridge_route.py`)."""
    return Response(
        content=canonical(
            {
                "protocol": BRIDGE_RESULT,
                "state": "refused",
                "refusal": {**refusal, "retry": {"kind": "after_fix"}},
                "device_version": "0.9.1",
            }
        ),
        status_code=403,
        media_type="application/json",
    )


async def call(tmp_path, reply, tool, arguments):
    hub = FakeHub(device_replies=[reply])
    async with serving(tmp_path, hub) as (_client, server):
        result = await server.call_tool(tool, arguments)
    return hub, result


async def test_the_three_tools_exist_with_spanish_descriptions_and_their_parameters(tmp_path):
    async with serving(tmp_path, FakeHub()) as (_client, server):
        tools = {tool.name: tool for tool in await server.list_tools()}
    for name, parameters in {
        "orbit_files_list": {"path", "after_index", "device"},
        "orbit_files_search": {"query", "path", "content", "device"},
        "orbit_files_read": {"path", "after_index", "after_page", "device"},
    }.items():
        assert name in tools, name
        assert set(tools[name].input_schema["properties"]) == parameters, name
        description = tools[name].description
        assert "folder" in description and "Deimos" not in description, description


async def test_a_path_outside_the_folders_names_it_and_every_folder(tmp_path):
    hub, result = await call(
        tmp_path,
        refused(
            {
                "code": "outside_folders",
                "path": SECRET,
                "resolved": None,
                "folders": [PROJECTS, NOTES],
            }
        ),
        "orbit_files_read",
        {"path": SECRET},
    )
    body = text_of(result)
    assert result.is_error, body
    assert body.startswith("outside_folders: "), body
    assert SECRET in body and PROJECTS in body and NOTES in body, body
    assert "[local path]" not in body
    assert hub.device_requests[0]["capability"] == "files_read"
    assert hub.device_requests[0]["parameters"] == {"path": SECRET}


async def test_a_link_out_of_a_folder_says_where_it_leads(tmp_path):
    link = f"{PROJECTS}/atajo.txt"
    _hub, result = await call(
        tmp_path,
        refused(
            {"code": "outside_folders", "path": link, "resolved": SECRET, "folders": [PROJECTS]}
        ),
        "orbit_files_read",
        {"path": link},
    )
    body = text_of(result)
    assert result.is_error
    assert link in body and SECRET in body and "leads" in body, body


async def test_a_file_too_large_says_both_sizes(tmp_path):
    _hub, result = await call(
        tmp_path,
        refused(
            {
                "code": "file_too_large",
                "path": f"{PROJECTS}/manual.pdf",
                "content": "pdf",
                "bytes": 70_000_000,
                "max_bytes": 67_108_864,
            }
        ),
        "orbit_files_read",
        {"path": f"{PROJECTS}/manual.pdf"},
    )
    body = text_of(result)
    assert result.is_error
    assert "70000000" in body and "67108864" in body and "manual.pdf" in body, body


async def test_a_folder_whose_bookmark_no_longer_opens_says_why(tmp_path):
    _hub, result = await call(
        tmp_path,
        refused(
            {
                "code": "folder_unavailable",
                "path": NOTES,
                "cause": {
                    "kind": "bookmark_unresolvable",
                    "domain": "NSCocoaErrorDomain",
                    "code": 4,
                },
            }
        ),
        "orbit_files_list",
        {"path": NOTES},
    )
    body = text_of(result)
    assert result.is_error
    assert NOTES in body and "NSCocoaErrorDomain" in body and "Settings" in body, body


async def test_without_folders_it_says_where_to_add_them(tmp_path):
    _hub, result = await call(
        tmp_path, refused({"code": "no_folders"}), "orbit_files_search", {"query": "plan"}
    )
    body = text_of(result)
    assert result.is_error and "Add folder" in body, body


async def test_the_folders_are_listed_with_their_state(tmp_path):
    hub, result = await call(
        tmp_path,
        answered(
            {
                "entries": [
                    {"name": PROJECTS, "kind": "folder"},
                    {
                        "name": NOTES,
                        "kind": "folder",
                        "unavailable": {"kind": "not_a_folder"},
                    },
                ],
                "total": 2,
                "after_index": 0,
            }
        ),
        "orbit_files_list",
        {},
    )
    body = text_of(result)
    assert not result.is_error, body
    assert PROJECTS in body and NOTES in body and "cannot be read" in body, body
    assert hub.device_requests[0]["parameters"] == {}


async def test_a_folder_listing_gives_whole_paths_and_where_to_go_on(tmp_path):
    hub, result = await call(
        tmp_path,
        answered(
            {
                "path": PROJECTS,
                "entries": [
                    {"name": "informes", "kind": "folder"},
                    {"name": "plan.md", "kind": "file", "size_bytes": 2048},
                    {"name": "atajo", "kind": "link"},
                ],
                "total": 203,
                "after_index": 0,
                "next_after_index": 3,
            }
        ),
        "orbit_files_list",
        {"path": PROJECTS},
    )
    body = text_of(result)
    assert not result.is_error, body
    assert f"{PROJECTS}/plan.md" in body and "2048" in body, body
    assert f"{PROJECTS}/informes/" in body
    assert "link" in body
    assert "after_index=3" in body and "203" in body, body
    assert hub.device_requests[0]["parameters"] == {"path": PROJECTS}


async def test_a_search_says_its_matches_and_what_stopped_it(tmp_path):
    hub, result = await call(
        tmp_path,
        answered(
            {
                "query": "presupuesto",
                "content": True,
                "results": [
                    {
                        "path": f"{PROJECTS}/plan.md",
                        "kind": "file",
                        "size_bytes": 2048,
                        "line": 3,
                        "snippet": "El presupuesto del trimestre.",
                    }
                ],
                "visited": 20000,
                "files_read": 4,
                "bytes_read": 8192,
                "partial_files": 1,
                "stopped": {"cause": "visited", "limit": 20000},
            }
        ),
        "orbit_files_search",
        {"query": "presupuesto", "content": True},
    )
    body = text_of(result)
    assert not result.is_error, body
    assert f"{PROJECTS}/plan.md:3" in body and "El presupuesto del trimestre." in body, body
    assert "20000" in body and "narrower folder" in body, body
    assert hub.device_requests[0]["parameters"] == {"query": "presupuesto", "content": True}


async def test_a_text_file_arrives_with_where_to_go_on(tmp_path):
    _hub, result = await call(
        tmp_path,
        answered(
            {
                "path": f"{PROJECTS}/plan.md",
                "size_bytes": 120_000,
                "content": "text",
                "text": "# Plan\nEl presupuesto.",
                "after_index": 0,
                "next_after_index": 48_000,
            }
        ),
        "orbit_files_read",
        {"path": f"{PROJECTS}/plan.md"},
    )
    body = text_of(result)
    assert not result.is_error, body
    assert "# Plan\nEl presupuesto." in body
    assert "after_index=48000" in body and "120000" in body, body


async def test_a_pdf_arrives_as_its_pages_text(tmp_path):
    hub, result = await call(
        tmp_path,
        answered(
            {
                "path": f"{PROJECTS}/manual.pdf",
                "size_bytes": 120_000,
                "content": "pdf",
                "pages": [{"page": 3, "text": "Capítulo tres."}, {"page": 4, "text": ""}],
                "page_count": 12,
                "after_page": 2,
                "after_index": 0,
                "next_after_page": 4,
                "next_after_index": 0,
            }
        ),
        "orbit_files_read",
        {"path": f"{PROJECTS}/manual.pdf", "after_page": 2},
    )
    body = text_of(result)
    assert not result.is_error, body
    assert "Page 3 of 12" in body and "Capítulo tres." in body, body
    assert "no text" in body
    assert "after_page=4" in body, body
    assert hub.device_requests[0]["parameters"] == {
        "path": f"{PROJECTS}/manual.pdf",
        "after_page": 2,
    }


async def test_an_image_arrives_as_an_image(tmp_path):
    _hub, result = await call(
        tmp_path,
        answered(
            {
                "path": f"{PROJECTS}/foto.heic",
                "size_bytes": 4_000_000,
                "content": "image",
                "width": 1024,
                "height": 768,
                "mime_type": "image/png",
                "sha256": "a" * 64,
                "source_width": 4032,
                "source_height": 3024,
                "data_base64": base64.b64encode(PNG).decode("ascii"),
            }
        ),
        "orbit_files_read",
        {"path": f"{PROJECTS}/foto.heic"},
    )
    assert not result.is_error, text_of(result)
    images = [block for block in result.content if block.type == "image"]
    assert len(images) == 1 and images[0].mime_type == "image/png"
    assert base64.b64decode(images[0].data) == PNG
    assert "4032 × 3024" in text_of(result)


async def test_parameters_are_checked_before_the_mac_is_asked(tmp_path):
    hub = FakeHub()
    async with serving(tmp_path, hub) as (_client, server):
        relative = await server.call_tool("orbit_files_read", {"path": "Proyectos/plan.md"})
        long_query = await server.call_tool("orbit_files_search", {"query": "x" * 257})
    assert relative.is_error and "path" in text_of(relative), text_of(relative)
    assert long_query.is_error and "256" in text_of(long_query), text_of(long_query)
    assert hub.device_requests == []


async def test_what_leaves_the_translator_is_the_shared_contract(tmp_path):
    """`contracts/fixtures/device-bridge-request-v1.json` pins the parameters,
    types included; the daemon's suite feeds the same entries to its handler."""
    import json
    from pathlib import Path

    document = json.loads(
        (
            Path(__file__).resolve().parents[2] / "contracts/fixtures/device-bridge-request-v1.json"
        ).read_text()
    )
    contracted = {entry["capability"]: entry["parameters"] for entry in document["requests"]}
    calls = {
        "files_list": ("orbit_files_list", {"path": PROJECTS, "after_index": 200}),
        "files_search": (
            "orbit_files_search",
            {"query": "presupuesto", "path": PROJECTS, "content": True},
        ),
        "files_read": (
            "orbit_files_read",
            {"path": f"{PROJECTS}/manual.pdf", "after_index": 0, "after_page": 2},
        ),
    }
    for capability, (tool, arguments) in calls.items():
        hub = FakeHub(device_replies=[refused({"code": "no_folders"})])
        async with serving(tmp_path, hub) as (_client, server):
            await server.call_tool(tool, arguments)
        sent = hub.device_requests[0]
        assert sent["capability"] == capability
        assert json.dumps(sent["parameters"], sort_keys=True) == json.dumps(
            contracted[capability], sort_keys=True
        ), capability


def test_a_listing_asked_past_its_end_says_so_instead_of_an_impossible_range():
    # The helper answers an `after_index` past the end with no entries and the
    # real total (`AssistantFiles.listNow` clamps the start); the text must
    # not read «entradas 101–100 de 5», a range that cannot exist.
    text = files.render_list(
        {
            "path": PROJECTS,
            "entries": [],
            "total": 5,
            "after_index": 100,
            "next_after_index": None,
        }
    )
    assert "101–100" not in text, text
    assert "5 entries" in text and "after_index=100" in text, text


async def test_a_folder_request_that_ran_out_of_time_does_not_ask_to_check_a_result(tmp_path):
    _hub, result = await call(
        tmp_path,
        refused(
            {"code": "timeout", "operation": "files", "waited_ms": 20_004, "deadline_ms": 20_000}
        ),
        "orbit_files_read",
        {"path": f"{PROJECTS}/plan.md"},
    )
    body = text_of(result)
    assert result.is_error
    assert body.startswith("timeout: "), body
    assert "check its result" not in body, body
    assert "assistant's folders" in body and "20000" in body, body


async def test_a_helper_restart_during_a_folder_request_is_not_called_a_source_read(tmp_path):
    _hub, result = await call(
        tmp_path,
        refused({"code": "read_interrupted", "cause": {"kind": "helper_restarted"}}),
        "orbit_files_search",
        {"query": "plan"},
    )
    body = text_of(result)
    assert result.is_error
    assert "source" not in body, body
    assert "assistant's folders" in body, body
