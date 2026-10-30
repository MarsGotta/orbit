"""Device-authenticated proposal API; assistant vault tools remain read-only."""

from fastapi import Request
from starlette.concurrency import run_in_threadpool

from ..vault.models import MAX_PAGE_SIZE, MAX_SCAN_ENTRIES, parse, refusal
from ..vault.routes import reply
from .models import MAX_PROPOSAL_BYTES, Review


def mount(app, service, identity, bounded_body):
    @app.get("/v1/vault/distillations")
    async def listing(
        request: Request, meeting_id: str | None = None, page: int = 1, page_size: int = 50
    ):
        identity(request)
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

    @app.post("/v1/recordings/{recording_id}/distillations")
    async def prepare(recording_id: str, request: Request):
        result = await run_in_threadpool(service.request, identity(request), recording_id)
        return reply(
            result
            if result is not None
            else {"prepared": False, "reason": "vault_or_model_not_configured"}
        )

    @app.get("/v1/vault/distillations/{proposal_id}")
    async def read(proposal_id: str, request: Request):
        identity(request)
        return reply(service.repository.public(await run_in_threadpool(service.get, proposal_id)))

    @app.patch("/v1/vault/distillations/{proposal_id}")
    async def review(proposal_id: str, request: Request):
        identity(request)
        payload = parse(Review, await bounded_body(request, MAX_PROPOSAL_BYTES))
        return reply(await run_in_threadpool(service.review, proposal_id, payload))

    @app.post("/v1/vault/distillations/{proposal_id}/prepare")
    async def prepare_again(proposal_id: str, request: Request):
        caller = identity(request)
        value = await run_in_threadpool(service.get, proposal_id)
        result = await run_in_threadpool(service.request, caller, value["meeting_id"])
        return reply(
            result
            if result is not None
            else {"prepared": False, "reason": "vault_or_model_not_configured"}
        )

    @app.post("/v1/vault/distillations/{proposal_id}/{action}")
    async def action(proposal_id: str, action: str, request: Request):
        caller = identity(request)
        if action not in {"apply", "discard", "undo"}:
            raise refusal("distillation_action_invalid", "Use apply, discard or undo.", 404)
        return reply(await run_in_threadpool(service.action, proposal_id, action, caller))
