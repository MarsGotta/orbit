"""Hub-owned preparation and review operations, shared by HTTP and the note worker."""

import asyncio
from threading import RLock

from pydantic import ValidationError
from starlette.concurrency import run_in_threadpool

from ..ai.consumers import FunctionProcessing
from ..ai.registry import Registry
from ..errors import HubError
from ..meetings import ownership
from ..meetings import repository as meetings
from ..processing import ProcessingError
from ..vault.models import refusal
from ..vault.privacy import private_read
from ..vault.schema import load
from ..vault.service import Vault, connector
from . import effects, material, validation, writing
from .models import CONNECTOR_JOB_SECONDS
from .patches import render
from .repository import Proposals, digest, namespace


class Distillations:
    def __init__(self, store, now, default_language=None, processing=None, changed=None):
        self.store, self.now, self.default_language = store, now, default_language
        self.vault, self.registry, self.repository = Vault(store), Registry(store), Proposals(store)
        self.changed = changed or (lambda device, recording: None)
        self.model = FunctionProcessing(self.registry, processing, function="vault")
        self.lock, self.preparation = RLock(), asyncio.Lock()
        self.repository.recover()

    def context(self):
        config = self.vault.configuration()
        if config is None:
            raise refusal(
                "vault_not_connected", "Connect a vault before preparing a proposal.", 409
            )
        return config

    def get(self, identity):
        value = self.repository.get(identity)
        if value["namespace"] != namespace(self.context()):
            raise refusal(
                "distillation_vault_changed",
                "This proposal belongs to another vault connection.",
                409,
            )
        return value

    def list(self, meeting=None, page=1, page_size=50):
        values = self.repository.list(
            namespace(self.context()), meeting, (page - 1) * page_size, page_size + 1
        )
        return {
            "entries": [self.repository.public(v) for v in values[:page_size]],
            "next_page": page + 1 if len(values) > page_size else None,
        }

    def resolve(self, caller, recording):
        with self.store.read_snapshot() as db:
            device = ownership.device_for(db, recording, caller)
            meetings.require(db, device, recording)
        return device

    def request(self, caller, recording):
        device = self.resolve(caller, recording)
        config = self.vault.configuration()
        if config is None or not self.model.configured:
            return None
        value = self.repository.create(device, recording, config, effects.timestamp(self.now()))
        value["status"] = "queued"
        self.repository.save(value)
        return self.repository.public(value)

    async def prepare(self, caller, recording, queued=None):
        device = await run_in_threadpool(self.resolve, caller, recording)
        config = await run_in_threadpool(self.vault.configuration)
        configured = await run_in_threadpool(lambda: self.model.configured)
        if config is None or not configured:
            return None
        async with self.preparation:
            if queued is None:
                value = await run_in_threadpool(
                    self.repository.create, device, recording, config, effects.timestamp(self.now())
                )
            else:
                value = await run_in_threadpool(self.repository.get, queued["id"])
                if value["status"] != "queued":
                    return self.repository.public(value)
                value["status"] = "preparing"
                await run_in_threadpool(self.repository.save, value)
            try:
                facts = await run_in_threadpool(
                    material.read, self.store, caller, recording, self.default_language
                )
                snapshot, schema = await run_in_threadpool(self._snapshot, config)
                async with self.model.session() as client:
                    with private_read():
                        answer = await client.complete_json(
                            purpose="vault_distillation",
                            messages=material.messages(snapshot, schema, facts),
                            schema_name="orbit_distillation",
                            schema=material.answer_schema(),
                        )
                value = await run_in_threadpool(
                    self._build, value, answer.content, snapshot, schema, facts
                )
            except ProcessingError as exc:
                value.update(status="failed", error=exc.as_dict())
            except ValidationError as failure:
                problem = failure.errors(include_input=False, include_context=False)[0]
                field = (
                    problem["loc"][0]
                    if problem["loc"] and problem["loc"][0] in {"changes", "quotes", "summary"}
                    else "response"
                )
                value.update(
                    status="failed",
                    changes=[],
                    conflicts=[],
                    error={"code": "invalid_model_shape", "field": field, "rule": problem["type"]},
                )
            except ValueError as exc:
                # Only our own validation codes, never provider text or raw input.
                reason = str(exc)
                safe = (
                    reason
                    if reason and all(c.isalnum() or c in "_:," for c in reason)
                    else "invalid_proposal"
                )
                value.update(status="failed", changes=[], conflicts=[], error={"code": safe[:160]})
            except HubError as exc:
                value.update(status="failed", error={"code": exc.code, **(exc.numbers or {})})
            except OSError:
                value.update(status="failed", error={"code": "vault_io_failed"})
            except Exception:  # noqa: BLE001 — a failed job never kills the note worker.
                value.update(status="failed", error={"code": "distillation_internal_error"})
            await run_in_threadpool(self.repository.save, value)
            self.changed(device, recording)
            return self.repository.public(value)

    def _snapshot(self, config):
        remote = connector(config, budget_seconds=CONNECTOR_JOB_SECONDS)
        try:
            return validation.snapshot(remote)
        finally:
            remote.close()

    def _build(self, value, content, snapshot, schema, facts):
        from ..app import hub_version

        value = validation.build(value, content, snapshot, schema, facts)
        value["by"] = "orbit-hub/" + hub_version()
        value["new_source"], value["source_raw"] = effects.source_for(
            value, schema, facts, value["by"]
        )
        paths = {c["path"] for c in value["changes"]} | {
            "SCHEMA.md",
            "index.md",
            "log.md",
            value["new_source"]["path"],
        }
        validation.validate_source(value["source_raw"], value["new_source"]["path"], schema)
        value["paths"] = sorted(snapshot)
        value["read_set"] = {
            p: {"hash": digest(item["raw"]), "version": item.get("version")}
            for p, item in snapshot.items()
        }
        value["snapshot"] = {p: snapshot.get(p, {"raw": None, "version": None}) for p in paths}
        validation.validate_links(value["changes"], set(snapshot) | paths)
        value["status"] = "proposed"
        return value

    def review(self, identity, request):
        with self.lock:
            value = self.get(identity)
            self._editable(value)
            if request.revision != value["revision"]:
                raise refusal(
                    "stale_revision", "The proposal changed; read it before editing.", 409
                )
            changes = {c["id"]: c for c in value["changes"]}
            conflicts = {c["id"]: c for c in value["conflicts"]}
            try:
                self._edit(changes, conflicts, request)
                validation.exclude_links(changes.values())
                schema = load(value["snapshot"]["SCHEMA.md"]["raw"] or "")
                for change in changes.values():
                    raw = render(change, conflicts)
                    _, change["warnings"] = validation.validate(change["path"], raw, schema)
                validation.validate_links(
                    [c for c in changes.values() if c["include"]],
                    set(value["paths"]) | {c["path"] for c in changes.values() if c["include"]},
                )
            except (KeyError, ValueError, IndexError, TypeError):
                raise refusal(
                    "distillation_edit_invalid",
                    "The edited change, quote, type or link is invalid.",
                    400,
                ) from None
            value["revision"] += 1
            self.repository.save(value)
            return self.repository.public(value)

    @staticmethod
    def _edit(changes, conflicts, request):
        for key, choice in request.decisions.items():
            conflicts[key]["choice"] = choice
        for key, edit in request.changes.items():
            change = changes[key]
            if edit.include is not None:
                change["include"] = edit.include
            for name, after in edit.properties.items():
                prop = next((p for p in change["properties"] if p["key"] == name), None)
                if prop is None:
                    raise ValueError("unknown_property")
                prop["after"] = after
                for conflict in conflicts.values():
                    if conflict["path"] == change["path"] and conflict["property"] == name:
                        conflict["incoming"] = after
            for number, text in edit.lines.items():
                validation._new_links(text)
                if not number.isdecimal():
                    raise ValueError("invalid_line_index")
                change["lines"][int(number)]["after"] = text
                change["lines"][int(number)]["unfiltered_after"] = text
                conflict = conflicts.get(f"{key}:lines:{int(number)}")
                if conflict:
                    conflict["incoming"] = text

    @staticmethod
    def _editable(value):
        if value["status"] != "proposed":
            raise refusal(
                "distillation_not_proposed",
                "Only a prepared proposal can be edited or applied.",
                409,
            )

    def action(self, identity, action, caller):
        with self.lock:
            config = self.context()
            value = self.repository.get(identity)
            if value["namespace"] != namespace(config):
                raise refusal(
                    "distillation_vault_changed",
                    "This proposal belongs to another vault connection.",
                    409,
                )
            if (action, value["status"]) in {
                ("apply", "applied"),
                ("discard", "discarded"),
                ("undo", "undone"),
            }:
                return self.repository.public(value)
            if action == "discard":
                if value["status"] not in {"queued", "proposed", "failed", "stale"}:
                    self._editable(value)
                value["status"] = "discarded"
                self.repository.save(value)
                return self.repository.public(value)
            with self.store.read_snapshot() as db:
                owner = meetings.owner_name(self.store, db, caller)
            human = effects.actor(owner, caller)
            at = effects.timestamp(self.now())
            remote = connector(config, budget_seconds=CONNECTOR_JOB_SECONDS)
            try:
                if action == "apply":
                    self._editable(value)
                    # All contradictions, even on excluded notes, need an explicit decision.
                    if any(c["choice"] is None for c in value["conflicts"]):
                        raise refusal(
                            "distillation_undecided",
                            "Decide every contradiction before applying.",
                            409,
                        )
                    schema = load(value["snapshot"]["SCHEMA.md"]["raw"] or "")
                    journal = effects.plan(value, schema, value["by"], human, at)
                    writing.apply(remote, value, journal, self.repository)
                elif action == "undo":
                    if value["status"] not in {
                        "applied",
                        "applying",
                        "undoing",
                        "recovery_required",
                    }:
                        raise refusal(
                            "distillation_not_applied", "The proposal has not been applied.", 409
                        )
                    writing.undo(remote, value, self.repository, value["by"], at)
            except ValueError:
                raise refusal(
                    "distillation_validation_failed",
                    "The final notes fail schema or source validation; prepare again.",
                    409,
                ) from None
            finally:
                remote.close()
            return self.repository.public(value)

    async def run_once(self):
        """Durable ready-note scan: no model/vault means no proposal and no job."""
        config = await run_in_threadpool(self.vault.configuration)
        configured = await run_in_threadpool(lambda: self.model.configured)
        if config is None or not configured:
            return False
        queued = await run_in_threadpool(self.repository.queued, namespace(config))
        if queued is not None:
            await self.prepare(queued["device_id"], queued["meeting_id"], queued=queued)
            return True
        candidate = await run_in_threadpool(self._candidate, namespace(config))
        if candidate is None:
            return False
        await self.prepare(candidate["device_id"], candidate["recording"])
        return True

    def _candidate(self, namespace_id):
        with self.store.read_snapshot() as db:
            return db.execute(
                "SELECT device_id,recording FROM recording_notes n WHERE state='ready' AND NOT EXISTS (SELECT 1 FROM vault_distillations p WHERE p.device_id=n.device_id AND p.recording=n.recording AND p.namespace=?) ORDER BY updated LIMIT 1",
                (namespace_id,),
            ).fetchone()
