"""What the installed Hermes asks Orbit, and what it accepts back, from its own models.

Hermes asks the person questions (an approval, a clarification, a password…)
as JSON-RPC requests from server to client, and only to a client that said
once per connection that it answers them (`client.capabilities`). The shapes
of those requests, of their answers and of their withdrawal (`request.cancel`)
are Pydantic models in the Hermes checkout (`tui_gateway/contracts`).

`contracts/hermes-server-requests.json` is generated from those models by
running this file with `--write`: Hermes's own interpreter imports them in a
child process and prints their JSON Schema. The hub's tests build every frame
Hermes sends from that file and validate every frame the hub sends back
against it, never against a shape written by hand;
`test_hermes_contract_current.py` regenerates it from the installed Hermes and
fails when Hermes changed.

    server/.venv/bin/python server/tests/support_hermes_requests.py --write

Nothing is written in the Hermes checkout: the child runs with `-I -B` and a
temporary `HERMES_HOME`.
"""

from __future__ import annotations

import asyncio
import json
import os
import subprocess
import sys
import tempfile
from pathlib import Path
from uuid import uuid4

from jsonschema import Draft202012Validator
from websockets.exceptions import ConnectionClosed

ROOT = Path(__file__).resolve().parents[2]
CONTRACT_PATH = ROOT / "contracts" / "hermes-server-requests.json"
# Empty only before the first `--write`.
CONTRACT = json.loads(CONTRACT_PATH.read_text(encoding="utf-8")) if CONTRACT_PATH.exists() else {}

# The requests a Hermes window answers (its docstring in
# tui_gateway/server_requests.py): every other attached client declines them
# with `not_shown_code`, so the window that shows the session still can.
WINDOW_REQUESTS = ("preview.act", "preview.read", "terminal.read", "tour", "window.read")
# JSON-RPC's "method not found": the answer of a client without a handler.
METHOD_NOT_FOUND = -32601

# ── The installed Hermes ─────────────────────────────────────────────────────

# Read-only: which environment Hermes's package manager committed for this
# checkout (the one its launcher runs). Only the standard library is imported.
FIND_ENVIRONMENT = """
import sys
from pathlib import Path
sys.path.insert(0, sys.argv[1])
from pm.environments import committed_venv
print(committed_venv(Path(sys.argv[1])) or "")
"""

# Runs inside Hermes's interpreter: its wire models as JSON Schema, without
# titles and descriptions (names of Python classes and prose, not the wire).
GENERATE = """
import json, re, sys
sys.path.insert(0, sys.argv[1])
import tui_gateway.contracts
from tui_gateway import server_requests
from tui_gateway.contracts import registry
from tui_gateway.contracts.server_requests import RequestCancelReason

def schema(model):
    raw = model.model_json_schema(mode="validation")
    defs = raw.pop("$defs", {})
    def clean(node, depth=0):
        if depth > 32:
            raise ValueError("recursive schema")
        if isinstance(node, list):
            return [clean(item, depth + 1) for item in node]
        if not isinstance(node, dict):
            return node
        if "$ref" in node:
            target = defs[node["$ref"].rsplit("/", 1)[1]]
            rest = {key: value for key, value in node.items() if key != "$ref"}
            return clean({**target, **rest}, depth + 1)
        cleaned = {}
        for key, value in node.items():
            if key in ("title", "description"):
                continue
            if key == "properties":
                cleaned[key] = {name: clean(item, depth + 1) for name, item in value.items()}
            else:
                cleaned[key] = clean(value, depth + 1)
        return cleaned
    return clean(raw)

schemas = {}
for name in ("client.capabilities", "request.answer", "clarify.lock"):
    contract = registry.METHODS[name]
    schemas[name + ".params"] = schema(contract.params)
    schemas[name + ".result"] = schema(contract.result)
for name, contract in registry.SERVER_REQUESTS.items():
    schemas["server_request." + name + ".params"] = schema(contract.params)
for name in ("approval", "clarify"):
    schemas["server_request." + name + ".result"] = schema(registry.SERVER_REQUESTS[name].result)
schemas["request.cancel.payload"] = schema(registry.EVENTS["request.cancel"].payload)

frame = server_requests.ServerRequest("session-example", "sudo", {}).frame()
frame["id"] = re.sub("[0-9a-f]{12}$", "0" * 12, frame["id"])
print(json.dumps({
    "server_requests": sorted(registry.SERVER_REQUESTS),
    "cancel_reasons": sorted(reason.value for reason in RequestCancelReason),
    # None: a Hermes older than "not shown here", which reads it as any error.
    "not_shown_code": getattr(server_requests, "NOT_SHOWN_CODE", None),
    "request_frame": frame,
    "schemas": schemas,
}, sort_keys=True))
"""


def hermes_checkout() -> Path:
    return Path(os.environ.get("ORBIT_HERMES_CHECKOUT") or Path.home() / ".hermes" / "hermes-agent")


def _environment(home: str | None) -> dict:
    environment = {key: value for key, value in os.environ.items() if key != "HERMES_HOME"}
    if home is not None:
        environment["HERMES_HOME"] = home
    return environment


def hermes_python() -> tuple[Path | None, str]:
    """Hermes's interpreter (its committed environment), or None and why not."""
    checkout = hermes_checkout()
    if not (checkout / "tui_gateway" / "contracts").is_dir():
        return None, f"Hermes is not installed here: no checkout at {checkout}"
    found = subprocess.run(
        [sys.executable, "-I", "-B", "-c", FIND_ENVIRONMENT, str(checkout)],
        capture_output=True,
        text=True,
        timeout=60,
        env=_environment(None),
        check=False,
    )
    venv = found.stdout.strip()
    if found.returncode != 0 or not venv:
        return None, (
            f"Hermes at {checkout} names no committed environment "
            f"(pm.environments.committed_venv): {found.stderr.strip()[-300:]}"
        )
    python = Path(venv) / "bin" / "python"
    if not python.exists():
        return None, f"Hermes's environment has no interpreter at {python}"
    return python, ""


def run_in_hermes(
    python: Path, code: str, *arguments: str, stdin: str = "", checkout: Path | None = None
) -> str:
    """`code` in Hermes's interpreter, with its checkout (the installed one by
    default) on its path; its output."""
    with tempfile.TemporaryDirectory(prefix="orbit-hermes-home-") as home:
        done = subprocess.run(
            [str(python), "-I", "-B", "-c", code, str(checkout or hermes_checkout()), *arguments],
            input=stdin,
            capture_output=True,
            text=True,
            timeout=120,
            env=_environment(home),
            cwd=home,
            check=False,
        )
    if done.returncode != 0:
        raise AssertionError(f"Hermes's interpreter failed: {done.stderr.strip()[-2000:]}")
    return done.stdout


def generate(python: Path, checkout: Path | None = None, **facts) -> str:
    """The contract file's text, from the installed Hermes (or the one at
    `checkout`), with `facts` about it beside what its models declare."""
    generated = json.loads(run_in_hermes(python, GENERATE, checkout=checkout))
    generated.update(facts)
    return json.dumps(generated, indent=2, sort_keys=True, ensure_ascii=False) + "\n"


# Validates instances with the real models (what Hermes runs on what it reads).
VALIDATE_WITH_MODELS = """
import json, sys
sys.path.insert(0, sys.argv[1])
import tui_gateway.contracts
from tui_gateway import server_requests
from tui_gateway.contracts import registry
problems = []
for name, instance in json.loads(sys.stdin.read()):
    try:
        if name == "response":
            if not server_requests.is_response_frame(instance):
                problems.append([name, "not a response frame"])
            continue
        kind, _, part = name.rpartition(".")
        if kind.startswith("server_request."):
            contract = registry.SERVER_REQUESTS[kind.removeprefix("server_request.")]
        else:
            contract = registry.METHODS[kind]
        getattr(contract, part).model_validate(instance)
    except Exception as error:
        problems.append([name, str(error)[:500]])
print(json.dumps(problems))
"""


def validate_with_hermes(
    python: Path, instances: list[tuple[str, object]], checkout: Path | None = None
) -> list:
    """Problems the installed Hermes's models (or those at `checkout`) find in
    `instances` (`(schema name, instance)`, or `("response", frame)`)."""
    return json.loads(
        run_in_hermes(python, VALIDATE_WITH_MODELS, stdin=json.dumps(instances), checkout=checkout)
    )


# ── Frames, from the contract ───────────────────────────────────────────────


def validate(name: str, instance, contract: dict = CONTRACT) -> None:
    """Raise unless `instance` fits the installed Hermes's model `name`
    (or that of the Hermes `contract` was generated from)."""
    Draft202012Validator(contract["schemas"][name]).validate(instance)


def problems(name: str, instance, contract: dict = CONTRACT) -> list[str]:
    validator = Draft202012Validator(contract["schemas"][name])
    return [error.message for error in validator.iter_errors(instance)]


def _value(name: str, schema: dict):
    if "enum" in schema:
        return schema["enum"][0]
    if "anyOf" in schema:
        branches = [branch for branch in schema["anyOf"] if branch.get("type") != "null"]
        return _value(name, branches[0])
    kind = schema.get("type")
    if kind == "string":
        return f"example-{name}"
    if kind == "integer":
        return 1
    if kind == "boolean":
        return False
    if kind == "array":
        return []
    if kind == "object":
        return {key: _value(key, schema["properties"][key]) for key in schema.get("required", [])}
    return None


def request_params(method: str, session_id: str, *, contract: dict = CONTRACT, **values) -> dict:
    """The params of a `method` request Hermes would send, validated."""
    schema = contract["schemas"][f"server_request.{method}.params"]
    params = {
        name: _value(name, schema["properties"][name])
        for name in schema.get("required", [])
        if name != "session_id"
    }
    params.update(values)
    params["session_id"] = session_id
    validate(f"server_request.{method}.params", params, contract)
    return params


def request_frame(method: str, session_id: str, *, contract: dict = CONTRACT, **values) -> dict:
    """A whole request frame as Hermes writes it (`ServerRequest.frame`)."""
    template = contract["request_frame"]
    frame = {key: value for key, value in template.items() if key not in ("method", "params")}
    frame["id"] = template["id"][:-12] + uuid4().hex[:12]
    frame["method"] = method
    frame["params"] = request_params(method, session_id, contract=contract, **values)
    return frame


def cancel_event(frame: dict, reason: str) -> dict:
    """Hermes withdrawing request `frame` (`request.cancel`)."""
    payload = {"id": frame["id"], "method": frame["method"], "reason": reason}
    validate("request.cancel.payload", payload)
    return {
        "jsonrpc": "2.0",
        "method": "event",
        "params": {
            "type": "request.cancel",
            "session_id": frame["params"]["session_id"],
            "payload": payload,
        },
    }


def is_response(frame: dict) -> bool:
    """What Hermes takes as a client's answer (`server_requests.is_response_frame`)."""
    return "method" not in frame and "id" in frame and ("result" in frame or "error" in frame)


# ── A Hermes peer that speaks only what the contract says ───────────────────


class ContractPeer:
    """Hermes's gateway WebSocket as the contract declares it.

    Every request the hub sends is recorded in `requests`, every answer to a
    request of Hermes in `responses`; what does not fit the installed Hermes's
    models is recorded in `violations`, which a test asserts empty. Requests
    are pushed to the hub with `push`. `capabilities` is how `client.capabilities`
    is answered: `"answers"` as the installed Hermes, `"unknown"` as a Hermes
    older than the method (-32601), or an error code. `contract` is the Hermes
    it speaks for: the installed one by default.
    """

    def __init__(self, capabilities="answers", contract: dict = CONTRACT):
        self.capabilities = capabilities
        self.contract = contract
        self.requests: list[dict] = []
        self.responses: list[dict] = []
        self.violations: list[str] = []
        self.answer_status = "ok"
        self.sessions = 0
        self.connections = 0
        # Request id -> its method, for the requests Hermes holds open.
        self.opened: dict[str, str] = {}
        # Request id -> the question ids not locked yet, for open clarifications.
        self.open_clarifications: dict[str, list[str]] = {}
        self._socket = None
        self._changed = asyncio.Condition()

    async def serve(self, ws):
        self._socket = ws
        self.connections += 1
        await ws.send(
            json.dumps(
                {
                    "jsonrpc": "2.0",
                    "method": "event",
                    "params": {
                        "type": "gateway.ready",
                        "payload": {"replay_epoch": f"contract-epoch-{self.connections}"},
                    },
                }
            )
        )
        try:
            async for raw in ws:
                frame = json.loads(raw)
                if is_response(frame):
                    self._close(frame.get("id"))
                    await self._record(self.responses, frame)
                    continue
                await self._record(self.requests, frame)
                reply = self._reply(frame)
                await ws.send(json.dumps({"jsonrpc": "2.0", "id": frame.get("id"), **reply}))
        except ConnectionClosed:
            return
        except Exception as error:  # noqa: BLE001 - a peer bug fails the test that met it
            self.violations.append(f"the synthetic Hermes failed: {error!r}")

    async def _record(self, into: list, frame: dict) -> None:
        async with self._changed:
            into.append(frame)
            self._changed.notify_all()

    def _check(self, name: str, instance) -> None:
        for problem in problems(name, instance, self.contract):
            self.violations.append(f"{name}: {problem}")

    def _close(self, request_id) -> str | None:
        """Hermes's side of an answered request: it is no longer open."""
        self.open_clarifications.pop(request_id, None)
        return self.opened.pop(request_id, None)

    def _answer(self, params: dict) -> dict:
        """`request.answer`: its result is checked against the model of the
        request it answers; an id Hermes does not hold open is `expired`."""
        self._check("request.answer.params", params)
        asked = self._close(params.get("id"))
        if asked is None:
            self.violations.append(f"request.answer for no open request: {params.get('id')}")
        elif f"server_request.{asked}.result" not in self.contract["schemas"]:
            self.violations.append(f"request.answer to a {asked} request, which Orbit refuses")
        else:
            self._check(f"server_request.{asked}.result", params.get("result"))
        result = {"status": self.answer_status if asked else "expired"}
        validate("request.answer.result", result, self.contract)
        return result

    def _reply(self, frame: dict) -> dict:
        method, params = frame.get("method"), frame.get("params") or {}
        if method == "client.capabilities":
            self._check("client.capabilities.params", params)
            if self.capabilities == "answers":
                result = {"server_requests": self.contract["server_requests"]}
                schema = self.contract["schemas"]["client.capabilities.result"]
                if "declines_not_shown" in schema["properties"]:
                    result["declines_not_shown"] = True
                validate("client.capabilities.result", result, self.contract)
                return {"result": result}
            code = METHOD_NOT_FOUND if self.capabilities == "unknown" else self.capabilities
            return {"error": {"code": code, "message": "synthetic refusal"}}
        if method == "session.create":
            self.sessions += 1
            return {
                "result": {
                    "session_id": f"live-{self.sessions}",
                    "stored_session_id": f"stored-{self.sessions}",
                }
            }
        if method == "session.resume":
            self.sessions += 1
            return {
                "result": {"session_id": f"live-{self.sessions}", "resumed": params["session_id"]}
            }
        if method == "request.answer":
            return {"result": self._answer(params)}
        if method == "clarify.lock":
            self._check("clarify.lock.params", params)
            return {"result": self._lock(params)}
        if method == "approval.respond":
            return {"result": {"resolved": 1}}
        if method == "clarify.respond":
            return {"result": {"status": "ok", "remaining": []}}
        if method == "prompt.submit":
            return {"result": {"status": "streaming"}}
        if method == "session.interrupt":
            return {"result": {"status": "interrupted"}}
        if method == "session.status":
            return {"result": {"output": "Agent Running: Yes"}}
        return {"error": {"code": METHOD_NOT_FOUND, "message": f"unknown method: {method}"}}

    def _lock(self, params: dict) -> dict:
        # Only a clarification with `questions` locks; a single question in
        # the older shape has none, and Hermes answers its lock `expired`.
        remaining = self.open_clarifications.get(params.get("request_id"))
        if remaining is None:
            result = {"status": "expired"}
        else:
            remaining[:] = [qid for qid in remaining if qid != params.get("question_id")]
            if not remaining:
                self._close(params["request_id"])
            result = {"status": "ok", "remaining": list(remaining)}
        validate("clarify.lock.result", result, self.contract)
        return result

    async def push(self, frame: dict) -> None:
        """Send `frame` to the hub, as Hermes would."""
        method, params = frame.get("method"), frame.get("params") or {}
        if method == "event":
            if params.get("type") == "request.cancel":
                self._close(params["payload"]["id"])
        else:
            self.opened[frame["id"]] = method
            if method == "clarify" and params.get("questions"):
                self.open_clarifications[frame["id"]] = [
                    question["qid"] for question in params["questions"]
                ]
        await self._socket.send(json.dumps(frame))

    async def drop(self) -> None:
        """Hermes's side of the socket goes away."""
        await self._socket.close()

    async def wait_for(self, read, predicate, timeout=10, what="the hub"):
        """The first item of `read()` that matches `predicate`, waiting for it."""
        try:
            async with asyncio.timeout(timeout):
                async with self._changed:
                    while True:
                        for item in read():
                            if predicate(item):
                                return item
                        await self._changed.wait()
        except TimeoutError as error:
            raise AssertionError(f"{what} never sent it within {timeout} s") from error

    async def request(self, method, timeout=10):
        return await self.wait_for(
            lambda: self.requests,
            lambda frame: frame.get("method") == method,
            timeout,
            what=f"the hub's {method}",
        )

    async def response(self, request_id, timeout=10):
        return await self.wait_for(
            lambda: self.responses,
            lambda frame: frame.get("id") == request_id,
            timeout,
            what=f"an answer to {request_id}",
        )

    def answers_to(self, request_id: str) -> list[dict]:
        return [frame for frame in self.responses if frame.get("id") == request_id]


def methods(frames: list[dict]) -> list[str]:
    return [frame.get("method") for frame in frames]


if __name__ == "__main__":
    python, why = hermes_python()
    if python is None:
        raise SystemExit(why)
    text = generate(python)
    if sys.argv[1:] == ["--write"]:
        CONTRACT_PATH.write_text(text, encoding="utf-8")
        print(f"wrote {CONTRACT_PATH}")
    else:
        sys.stdout.write(text)
