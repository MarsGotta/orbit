"""Device-authenticated proposal API; assistant vault tools remain read-only.

Each route is answered by the function of its Notes call, which the plugin
call answers with too.
"""

from fastapi import Request
from starlette.concurrency import run_in_threadpool

from ..vault.models import MAX_PAGE_SIZE, MAX_SCAN_ENTRIES, parse, refusal
from ..vault.routes import reply
from .models import MAX_PROPOSAL_BYTES, Review

ACTIONS = frozenset({"apply", "discard", "undo"})


async def listing(service, meeting_id: str | None, page: int, page_size: int = 50):
    if (
        not 1 <= page <= MAX_SCAN_ENTRIES
        or not 1 <= page_size <= MAX_PAGE_SIZE
        or (page - 1) * page_size >= MAX_SCAN_ENTRIES
    ):
        raise refusal(
            "vault_invalid_request",
            f"Use pages within {MAX_SCAN_ENTRIES} entries, with 1 to {MAX_PAGE_SIZE} per page.",
            max_entries=MAX_SCAN_ENTRIES,
            max_page_size=MAX_PAGE_SIZE,
        )
    return reply(await run_in_threadpool(service.list, meeting_id, page, page_size))


async def prepare(service, caller: str, meeting_id: str):
    result = await run_in_threadpool(service.request, caller, meeting_id)
    return reply(
        result
        if result is not None
        else {"prepared": False, "reason": "vault_or_model_not_configured"}
    )


async def proposal(service, proposal_id: str):
    return reply(service.repository.public(await run_in_threadpool(service.get, proposal_id)))


async def review(service, proposal_id: str, payload: Review):
    return reply(await run_in_threadpool(service.review, proposal_id, payload))


async def act(service, caller: str, proposal_id: str, action: str):
    """`prepare` queues a new proposal for the same meeting; the others decide this one."""
    if action == "prepare":
        value = await run_in_threadpool(service.get, proposal_id)
        return await prepare(service, caller, value["meeting_id"])
    if action not in ACTIONS:
        raise refusal("distillation_action_invalid", "Use apply, discard or undo.", 404)
    return reply(await run_in_threadpool(service.action, proposal_id, action, caller))


def mount(app, service, identity, bounded_body):
    @app.get("/v1/vault/distillations")
    async def listing_route(
        request: Request, meeting_id: str | None = None, page: int = 1, page_size: int = 50
    ):
        identity(request)
        return await listing(service, meeting_id, page, page_size)

    @app.post("/v1/recordings/{recording_id}/distillations")
    async def prepare_route(recording_id: str, request: Request):
        return await prepare(service, identity(request), recording_id)

    @app.get("/v1/vault/distillations/{proposal_id}")
    async def read_route(proposal_id: str, request: Request):
        identity(request)
        return await proposal(service, proposal_id)

    @app.patch("/v1/vault/distillations/{proposal_id}")
    async def review_route(proposal_id: str, request: Request):
        identity(request)
        payload = parse(Review, await bounded_body(request, MAX_PROPOSAL_BYTES))
        return await review(service, proposal_id, payload)

    @app.post("/v1/vault/distillations/{proposal_id}/prepare")
    async def prepare_again_route(proposal_id: str, request: Request):
        return await act(service, identity(request), proposal_id, "prepare")

    @app.post("/v1/vault/distillations/{proposal_id}/{action}")
    async def action_route(proposal_id: str, action: str, request: Request):
        return await act(service, identity(request), proposal_id, action)
