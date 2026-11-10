"""Notes as a plugin of the hub: its manifest, its routes, its calls and its
meeting table.

Each call is the device's vault call with the same `call` tag, its other fields
in `input`, and is answered by the same function as its route: the route's body
as `value`, or its refusal unchanged.
"""

import json
from pathlib import Path

from ..distillation import mount as mount_distillations
from ..distillation import routes as distillation
from ..distillation.models import Review
from ..plugins import Call, Plugin, read_manifest
from . import routes as vault_routes
from .models import MAX_PAGE_SIZE, MAX_SCAN_ENTRIES, Configuration, Read, parse, refusal

MANIFEST = Path(__file__).with_name("orbit-plugin.json")
# The proposals name the meeting they were prepared from: they go with it.
RECORDING_TABLES = ("vault_distillations",)
# The access group the assistants see for Notes, and its orbit-mcp tools.
ASSISTANT_TOOLS = ("notes", ("orbit_notes_search", "orbit_note_read", "orbit_notes_index"))


def notes() -> Plugin:
    return Plugin(read_manifest(MANIFEST), mount, RECORDING_TABLES, ASSISTANT_TOOLS)


def mount(app, hub, identity, bounded_body, transport) -> dict[str, Call]:
    """The vault's routes, the signed read included, then the distillation's;
    and the calls that answer as they do."""
    store, distillations = hub.store, hub.distillations
    vault = vault_routes.mount(
        app, store, identity, bounded_body, hub.meetings_bridge, transport, distillations.model
    )
    app.state.distillations = distillations
    mount_distillations(app, distillations, identity, bounded_body)
    return calls(vault, distillations)


def calls(vault, service) -> dict[str, Call]:
    """Notes' calls by name. A model field is read as its route reads its body."""
    return {
        "config": takes({}, lambda _: vault_routes.configuration(vault)),
        "configure": takes(
            {"configuration": dict},
            lambda _, value: vault_routes.configure(vault, model(Configuration, value)),
        ),
        "disconnect": takes({}, lambda _: vault_routes.disconnect(vault)),
        "read": takes(
            {"request": dict}, lambda _, value: vault_routes.read(vault, model(Read, value))
        ),
        "list_distillations": takes(
            {"meeting_id": (str, type(None)), "page": int},
            lambda _, meeting, page: distillation.listing(service, meeting, page),
        ),
        "distillation": takes({"id": str}, lambda _, id: distillation.proposal(service, id)),
        "prepare_distillation": takes(
            {"meeting_id": str},
            lambda caller, meeting: distillation.prepare(service, caller, meeting),
        ),
        "review_distillation": takes(
            {"id": str, "review": dict},
            lambda _, id, value: distillation.review(service, id, model(Review, value)),
        ),
        "distillation_action": takes(
            {"id": str, "action": str},
            lambda caller, id, action: distillation.act(service, caller, id, action),
        ),
    }


def takes(fields: dict, answer) -> Call:
    """A call whose `input` has exactly `fields`, each of its type, given to
    `answer` in that order after the caller."""

    async def call(caller: str, input: dict):
        if set(input) != set(fields) or not all(
            isinstance(input[name], kind) and not isinstance(input[name], bool)
            for name, kind in fields.items()
        ):
            raise refusal(
                "vault_invalid_request",
                f"Check the Notes fields: 1 to {MAX_PAGE_SIZE} entries per page and a vault-relative path.",
                max_page_size=MAX_PAGE_SIZE,
                max_page=MAX_SCAN_ENTRIES,
            )
        return await answer(caller, *(input[name] for name in fields))

    return call


def model(kind, value: dict):
    """`value` read by `kind` as a route reads its JSON body, refusals included."""
    return parse(kind, json.dumps(value))
