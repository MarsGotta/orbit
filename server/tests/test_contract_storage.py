"""The written storage contract says what the hub's database is, byte for byte.

`contracts/hub-storage-v1.md` is what another program needs to open, read and
write the hub's database without reading Python. Each test holds part of it to
the code:

- a database the hub made (`initialize`, then the hub built over it) has the
  tables its blocks write: every column with its type, null, key and default,
  in order, and every unique key, reference, check and index;
- every context the code seals a value with, read from the code itself, is the
  context of a column the contract seals, and every context it gives a column
  is one the code uses;
- every sealed value the hub wrote through its own code opens with nothing but
  the contract: its values, the context of its column filled from the same
  row, and AES-GCM from `cryptography`, written here without `orbit_hub`; and
  what the contract derives (hashes, HMACs) is what the hub derived;
- the hub reads the same rows sealed again here, in either form, as before;
- a serving hub holds the lock the contract names.

The contract is read only in its fixed forms: the `Values` and `Derived values`
tables, and one block per table headed by its name in code (`` `devices` ``)
with a row per column and a row per key, check or index. Prose is for people.
"""

import ast
import fcntl
import hashlib
import hmac
import json
import os
import re
import sqlite3
from collections import defaultdict
from dataclasses import dataclass, field
from pathlib import Path
from uuid import uuid4

import httpx
import pytest
from cryptography.exceptions import InvalidTag
from cryptography.hazmat.primitives.ciphers.aead import AESGCM
from support_distillation import distillation_client, prepare
from support_pdf import pdf_bytes

from orbit_hub import safety
from orbit_hub.ai.registry import Registry
from orbit_hub.app import create_app
from orbit_hub.assistant.agent_config import AgentConfigStore
from orbit_hub.assistant.skills import Skill
from orbit_hub.storage import Store
from orbit_hub.transcription_preferences import Preferences

# Re-exported for pytest: the HTTP fixture with an invented vault and meeting.
__all__ = ["distillation_client"]

ROOT = Path(__file__).resolve().parents[2]
CONTRACT = ROOT / "contracts" / "hub-storage-v1.md"
HUB = ROOT / "server" / "orbit_hub"
HEADING = re.compile(r"^#{2,6} (.+?)\s*$", re.MULTILINE)
TABLE_HEADING = re.compile(r"`([a-z][a-z0-9_]*)`")
INDEX = re.compile(r"(unique )?index `([a-z][a-z0-9_]*)`")
CODE = re.compile(r"`([^`]*)`")
KINDS = ("text", "json", "bytes")
# The values this test takes from the contract to open and derive what the hub wrote.
VALUES = (
    "database",
    "lock",
    "journal_mode",
    "user_version",
    "cipher",
    "key_bytes",
    "nonce_bytes",
    "tag_bytes",
    "mark",
    "key_id_label",
    "key_id_bytes",
    "key_check_plaintext",
    "fingerprint_label",
    "reminder_label",
)
# The columns whose values this test derives as the contract says, and compares.
DERIVED = {
    "devices.token_hash",
    "pair_codes.hash",
    "chunks.sha256",
    "chunks.size",
    "attachments.sha256",
    "attachments.size",
    "operations.fingerprint",
    "safety_operations.fingerprint",
    "safety_operations.target",
    "personal_reminders.key",
    "vault_working_copy.namespace",
    "vault_working_copy.id",
}
# Values of the contract that SQLite keeps in the database file itself.
PRAGMAS = ("user_version", "journal_mode")
DIGEST = "5eed" * 16  # an invented meeting's title digest
SKILL = "---\nname: weekly-summary\ndescription: Sums up the week.\n---\nRead the week.\n"


# --- Reading the contract ---------------------------------------------------------


def contract() -> str:
    return CONTRACT.read_text() if CONTRACT.exists() else ""


def absent() -> str:
    return "" if CONTRACT.exists() else " (contracts/hub-storage-v1.md does not exist)"


def sections(text: str) -> list[tuple[str, str]]:
    marks = list(HEADING.finditer(text))
    return [
        (mark.group(1), text[mark.end() : marks[i + 1].start() if i + 1 < len(marks) else None])
        for i, mark in enumerate(marks)
    ]


def rows(body: str) -> list[list[str]]:
    found = []
    for line in body.splitlines():
        line = line.strip()
        if line.startswith("|") and not set(line) <= set("|-: "):
            found.append([cell.strip() for cell in re.split(r"(?<!\\)\|", line.strip("|"))])
    return found


def code(cell: str) -> str | None:
    match = CODE.fullmatch(cell)
    return match.group(1) if match else None


def values_of(text: str) -> tuple[dict[str, str], list[str]]:
    found = {}
    for title, body in sections(text):
        if title == "Values":
            for cells in rows(body):
                if len(cells) >= 2 and code(cells[0]) and code(cells[1]) is not None:
                    found[code(cells[0])] = code(cells[1])
    missing = [name for name in VALUES if name not in found]
    return found, [f"the Values table lacks {missing}{absent()}"] if missing else []


def derived_of(text: str) -> set[str]:
    return {
        code(cells[0])
        for title, body in sections(text)
        if title == "Derived values"
        for cells in rows(body)
        if code(cells[0]) and re.fullmatch(r"[a-z_][a-z0-9_]*\.[a-z_][a-z0-9_]*", code(cells[0]))
    }


@dataclass(frozen=True)
class Sealed:
    kind: str
    context: str
    retired: bool


@dataclass
class Table:
    columns: list = field(default_factory=list)  # (name, type, null, key, default)
    sealed: dict = field(default_factory=dict)  # column -> Sealed
    constraints: set = field(default_factory=set)
    problems: list = field(default_factory=list)


def names(text: str | None) -> str:
    """Columns as `a, b DESC`, however the contract spaces them."""
    parts = [" ".join(part.split()) for part in (text or "").split(",")]
    return ", ".join(re.sub(r" desc$", " DESC", part, flags=re.IGNORECASE) for part in parts)


def squeeze(text: str | None) -> str:
    return re.sub(r"\s+", "", text or "")


def reference(text: str | None) -> str:
    match = re.fullmatch(r"\s*(\w+)\s*\((.*)\)\s*", text or "")
    return f"{match.group(1)}({names(match.group(2))})" if match else (text or "")


def read_table(body: str) -> Table:
    table = Table()
    for cells in rows(body):
        cells += [""] * (6 - len(cells))
        first, kind = code(cells[0]), code(cells[1]) or ""
        index = INDEX.fullmatch(cells[0])
        if first is not None and re.fullmatch(r"[A-Z]+", kind):
            null, default = cells[2].lower(), code(cells[4]) or ""
            table.columns.append((first, kind, null, cells[3], default))
            if cells[5]:
                prose = CODE.sub("", cells[5])
                found = [word for word in KINDS if re.search(rf"\b{word}\b", prose)]
                context = CODE.search(cells[5])
                if not found or not context:
                    table.problems.append(f"`{first}` is sealed with no context or no plaintext")
                    continue
                retired = bool(re.search(r"\bretired\b", prose))
                table.sealed[first] = Sealed(found[0], context.group(1), retired)
        elif cells[0] == "unique":
            table.constraints.add(("unique", names(code(cells[1]))))
        elif cells[0] == "check":
            table.constraints.add(("check", squeeze(code(cells[1]))))
        elif cells[0] == "references":
            table.constraints.add(("references", names(code(cells[1])), reference(code(cells[2]))))
        elif index:
            kind = (index.group(1) or "") + "index"
            where = squeeze(code(cells[2]))
            table.constraints.add((kind, index.group(2), names(code(cells[1])), where))
    return table


def tables_of(text: str) -> tuple[dict[str, Table], list[str]]:
    found, problems = {}, []
    for title, body in sections(text):
        if heading := TABLE_HEADING.fullmatch(title):
            if heading.group(1) in found:
                problems.append(f"`{heading.group(1)}` has more than one block")
            found[heading.group(1)] = read_table(body)
    return found, problems


# --- Reading the database ---------------------------------------------------------


def checks(sql: str) -> list[str]:
    found = []
    for match in re.finditer(r"\bCHECK\s*\(", sql, re.IGNORECASE):
        depth = 1
        for at in range(match.end(), len(sql)):
            depth += {"(": 1, ")": -1}.get(sql[at], 0)
            if depth == 0:
                found.append(sql[match.end() : at])
                break
    return found


def connect(database: Path) -> sqlite3.Connection:
    db = sqlite3.connect(f"{database.resolve().as_uri()}?mode=ro", uri=True)
    db.row_factory = sqlite3.Row
    return db


def database_tables(database: Path) -> dict[str, Table]:
    """The schema SQLite reports for each table, in the contract's terms."""
    db, found = connect(database), {}
    try:
        objects = db.execute("SELECT type, name, sql FROM sqlite_master").fetchall()
        sql = {(kind, name): text for kind, name, text in objects}
        for kind, name, text in objects:
            if kind != "table" or name.startswith("sqlite_"):
                continue
            table = found[name] = Table()
            for _, column, declared, notnull, default, key in db.execute(
                f"PRAGMA table_info('{name}')"
            ):
                null = "no" if notnull else "yes"
                table.columns.append((column, declared, null, str(key or ""), default or ""))
            table.constraints |= {("check", squeeze(check)) for check in checks(text)}
            for _, index, unique, origin, _ in db.execute(f"PRAGMA index_list('{name}')"):
                keyed = [r for r in db.execute(f"PRAGMA index_xinfo('{index}')") if r["key"]]
                columns = ", ".join(r["name"] + (" DESC" if r["desc"] else "") for r in keyed)
                if origin == "u":
                    table.constraints.add(("unique", columns))
                elif origin == "c":
                    where = re.search(
                        r"\bWHERE\b(.*)$", sql[("index", index)], re.DOTALL | re.IGNORECASE
                    )
                    kind = "unique index" if unique else "index"
                    table.constraints.add((kind, index, columns, squeeze(where and where[1])))
            references = defaultdict(list)
            for row in db.execute(f"PRAGMA foreign_key_list('{name}')"):
                references[row["id"]].append(row)
            for parts in references.values():
                parts.sort(key=lambda row: row["seq"])
                target = f"{parts[0]['table']}({', '.join(row['to'] for row in parts)})"
                table.constraints.add(("references", ", ".join(r["from"] for r in parts), target))
    finally:
        db.close()
    return found


def schema_differences(written: dict, built: dict) -> list[str]:
    problems = []
    for name in sorted(built.keys() - written.keys()):
        problems.append(f"`{name}`: a table of the database with no block{absent()}")
    for name in sorted(written.keys() - built.keys()):
        problems.append(f"`{name}`: a block for a table the database does not have")
    for name in sorted(built.keys() & written.keys()):
        contract_table, database_table = written[name], built[name]
        problems += [f"`{name}`: {problem}" for problem in contract_table.problems]
        if contract_table.columns != database_table.columns:
            problems.append(
                f"`{name}` columns in order, (name, type, null, key, default):\n"
                f"  contract {contract_table.columns}\n  database {database_table.columns}"
            )
        if contract_table.constraints != database_table.constraints:
            extra = sorted(contract_table.constraints - database_table.constraints)
            missing = sorted(database_table.constraints - contract_table.constraints)
            problems.append(
                f"`{name}` keys, checks and indexes: the contract names {extra} that the"
                f" database does not have, and leaves out {missing}"
            )
        declared = {column: kind for column, kind, *_ in database_table.columns}
        for column, kind in declared.items():
            if kind == "BLOB" and column not in contract_table.sealed:
                problems.append(f"`{name}.{column}`: a BLOB column the contract does not seal")
        for column in contract_table.sealed:
            if declared.get(column) != "BLOB":
                problems.append(f"`{name}.{column}`: sealed, but not a BLOB column")
    return problems


# --- Reading the code: every context it seals or opens a value with ---------------


class Unreadable(Exception):
    """A context this test cannot read from the code; it fails, saying where."""


@dataclass
class Module:
    name: str
    path: Path
    tree: ast.Module
    constants: dict = field(default_factory=dict)
    functions: dict = field(default_factory=dict)
    modules: dict = field(default_factory=dict)  # local name -> module
    imported: dict = field(default_factory=dict)  # local name -> (module, function)


FUNCTION = (ast.FunctionDef, ast.AsyncFunctionDef, ast.Lambda)


def hub_modules() -> dict[str, Module]:
    found = {}
    for path in sorted(HUB.rglob("*.py")):
        parts = path.relative_to(HUB).with_suffix("").parts
        name = ".".join(parts[:-1] if parts[-1] == "__init__" else parts)
        module = found[name] = Module(name, path, ast.parse(path.read_text(), str(path)))
        for node in module.tree.body:
            if isinstance(node, ast.Assign) and [type(t) for t in node.targets] == [ast.Name]:
                try:
                    module.constants[node.targets[0].id] = ast.literal_eval(node.value)
                except (ValueError, TypeError, SyntaxError):
                    pass
            elif isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef)):
                module.functions[node.name] = node
    for module in found.values():
        package = module.name.split(".") if module.path.name == "__init__.py" else []
        package = package or module.name.split(".")[:-1]
        for node in ast.walk(module.tree):
            if not isinstance(node, ast.ImportFrom) or node.level - 1 > len(package):
                continue
            base = package[: len(package) - node.level + 1] if node.level else []
            target = base + (node.module.split(".") if node.module else [])
            if not node.level and target[:1] == ["orbit_hub"]:
                target = target[1:]
            for alias in node.names:
                local, dotted = alias.asname or alias.name, ".".join([*target, alias.name])
                if dotted in found:
                    module.modules[local] = dotted
                elif ".".join(target) in found:
                    module.imported[local] = (".".join(target), alias.name)
    return found


def calls(module: Module):
    """Each call of a module with the innermost function around it."""

    def visit(node, function):
        for child in ast.iter_child_nodes(node):
            if isinstance(child, ast.Call):
                yield child, function
            yield from visit(child, child if isinstance(child, FUNCTION) else function)

    yield from visit(module.tree, None)


def parameters(function) -> list[str]:
    arguments = function.args
    return [p.arg for p in (*arguments.posonlyargs, *arguments.args, *arguments.kwonlyargs)]


def sealed_with(call: ast.Call):
    """The context argument of `encrypt(data, context)` or `decrypt(data, context)`;
    the cipher's own `encrypt(nonce, data, associated)` is not one."""
    callee = call.func
    name = callee.attr if isinstance(callee, ast.Attribute) else getattr(callee, "id", None)
    if name not in ("encrypt", "decrypt") or len(call.args) + len(call.keywords) != 2:
        return None
    return (
        call.args[1]
        if len(call.args) == 2
        else {k.arg: k.value for k in call.keywords}.get("context")
    )


def shapes(node, module: Module, function) -> set[str]:
    """What a context expression can be: its text, with `{}` where a value goes."""
    if isinstance(node, ast.Constant) and isinstance(node.value, str):
        return {node.value}
    if isinstance(node, ast.JoinedStr):
        return {"".join(p.value if isinstance(p, ast.Constant) else "{}" for p in node.values)}
    if isinstance(node, ast.BinOp) and isinstance(node.op, ast.Add):
        left, right = piece(node.left, module, function), piece(node.right, module, function)
        return {a + b for a in left for b in right}
    if isinstance(node, ast.IfExp):
        return shapes(node.body, module, function) | shapes(node.orelse, module, function)
    if isinstance(node, ast.Call) and getattr(node.func, "attr", None) == "format":
        found = shapes(node.func.value, module, function)
        return {re.sub(r"\{[^{}]*\}", "{}", text) for text in found}
    if isinstance(node, ast.Call) and getattr(node.func, "id", None) in module.functions:
        helper = module.functions[node.func.id]
        returned = [n.value for n in ast.walk(helper) if isinstance(n, ast.Return) and n.value]
        if returned:
            return set().union(*(shapes(value, module, helper) for value in returned))
    if isinstance(node, ast.Name):
        return named(node.id, module, function)
    raise Unreadable(f"`{ast.unparse(node)}` is not a context this test can read")


def piece(node, module: Module, function) -> set[str]:
    try:
        return shapes(node, module, function)
    except Unreadable:
        return {"{}"}


def named(name: str, module: Module, function) -> set[str]:
    if function is not None:
        bound = [
            node.value
            for node in ast.walk(function)
            if isinstance(node, ast.Assign)
            and any(isinstance(t, ast.Name) and t.id == name for t in node.targets)
        ]
        if bound:
            return set().union(*(shapes(value, module, function) for value in bound))
        for loop in (node for node in ast.walk(function) if isinstance(node, ast.For)):
            targets = loop.target.elts if isinstance(loop.target, ast.Tuple) else [loop.target]
            labels = [getattr(target, "id", None) for target in targets]
            items = module.constants.get(getattr(loop.iter, "id", None))
            if name in labels and isinstance(items, (tuple, list)):
                at = labels.index(name)
                found = {item[at] if isinstance(loop.target, ast.Tuple) else item for item in items}
                if all(isinstance(text, str) for text in found):
                    return found
    if isinstance(module.constants.get(name), str):
        return {module.constants[name]}
    raise Unreadable(f"`{name}` is not a context this test can read")


def calls_of(modules: dict, home: Module, function, parameter: str):
    """Each call of a function that passes its context on, with that argument."""
    at, identity = parameters(function).index(parameter), (home.name, function.name)
    for module in modules.values():
        aliases = {local for local, name in module.modules.items() if name == home.name}
        imported = {local for local, target in module.imported.items() if target == identity}
        for call, caller in calls(module):
            callee = call.func
            direct = isinstance(callee, ast.Name) and (
                (module is home and callee.id == function.name) or callee.id in imported
            )
            through = (
                isinstance(callee, ast.Attribute)
                and callee.attr == function.name
                and getattr(callee.value, "id", None) in aliases
            )
            if direct or through:
                given = {k.arg: k.value for k in call.keywords}
                argument = call.args[at] if at < len(call.args) else given.get(parameter)
                yield module, caller, call, argument


def code_contexts() -> tuple[dict[str, set[str]], list[str]]:
    """Every context shape the hub's code seals or opens with, and where; and what
    it could not read. A function that passes its context argument on to
    `encrypt` or `decrypt` is followed to each of its callers."""
    modules, found, unreadable, passing = hub_modules(), defaultdict(set), [], set()
    pending = [
        (context, module, function, call)
        for module in modules.values()
        for call, function in calls(module)
        if (context := sealed_with(call)) is not None
    ]
    while pending:
        context, module, function, call = pending.pop()
        where = f"{module.path.relative_to(ROOT)}:{call.lineno}"
        if (
            isinstance(context, ast.Name)
            and isinstance(function, (ast.FunctionDef, ast.AsyncFunctionDef))
            and context.id in parameters(function)
        ):
            if (module.name, function.name) in passing:
                continue
            passing.add((module.name, function.name))
            callers = list(calls_of(modules, module, function, context.id))
            if not callers:
                unreadable.append(f"{where}: `{function.name}` passes its context on to no caller")
            for caller_module, caller, caller_call, argument in callers:
                if argument is None:
                    unreadable.append(f"{where}: a call of `{function.name}` gives no context")
                else:
                    pending.append((argument, caller_module, caller, caller_call))
            continue
        try:
            for shape in shapes(context, module, function):
                if not shape.replace("{}", ""):
                    raise Unreadable(f"`{ast.unparse(context)}` has no fixed text")
                found[shape].add(where)
        except Unreadable as failure:
            unreadable.append(f"{where}: {failure}")
    return found, unreadable


def matches(shape: str, context: str) -> bool:
    """Whether a context the contract writes is one this shape of the code can be.
    A `{}` of the code stands for a value (`{}` in the contract) or one fixed word."""
    pattern = r"(?:\{\}|[\w.-]+)".join(re.escape(part) for part in shape.split("{}"))
    return re.fullmatch(pattern, re.sub(r"\{\w+\}", "{}", context)) is not None


# --- Opening what the hub wrote, as the contract says --------------------------------


class Seal:
    """The two sealed forms as the contract writes them, with `cryptography` alone."""

    def __init__(self, key: bytes, value: dict):
        assert value["cipher"] == "AES-256-GCM", value["cipher"]
        assert len(key) == int(value["key_bytes"]) == 32
        self.cipher = AESGCM(key)
        self.nonce, self.tag = int(value["nonce_bytes"]), int(value["tag_bytes"])
        label = value["key_id_label"].encode()
        key_id = hmac.digest(key, label, "sha256")[: int(value["key_id_bytes"])]
        self.header = value["mark"].encode() + key_id

    def seal(self, plaintext: bytes, context: str, *, marked: bool) -> bytes:
        header, nonce = self.header if marked else b"", os.urandom(self.nonce)
        return header + nonce + self.cipher.encrypt(nonce, plaintext, header + context.encode())

    def open(self, sealed: bytes, context: str) -> tuple[bytes, str]:
        """The plaintext and the form that opened it: marked first, if it can be one."""
        header, end = self.header, len(self.header) + self.nonce
        if len(sealed) >= end + self.tag and sealed.startswith(header):
            nonce, rest = sealed[len(header) : end], sealed[end:]
            try:
                return self.cipher.decrypt(nonce, rest, header + context.encode()), "marked"
            except InvalidTag:
                pass
        opened = self.cipher.decrypt(sealed[: self.nonce], sealed[self.nonce :], context.encode())
        return opened, "today"


def fill(context: str, row: dict) -> str:
    return re.sub(r"\{(\w+)\}", lambda match: str(row[match.group(1)]), context)


def readable(plaintext: bytes, kind: str) -> bool:
    try:
        json.loads(plaintext) if kind == "json" else plaintext.decode() if kind == "text" else 0
    except ValueError:
        return False
    return True


@dataclass
class Opened:
    row: dict
    context: str
    plaintext: bytes
    form: str


def open_all(database: Path, blocks: dict, seal: Seal) -> tuple[dict, list[str]]:
    """Every value of every column the contract seals, opened as it says."""
    found, problems, db = defaultdict(list), [], connect(database)
    try:
        for table, block in sorted(blocks.items()):
            for column, sealed in block.sealed.items():
                query = f'SELECT rowid AS rowid, * FROM "{table}" WHERE "{column}" IS NOT NULL'
                for row in map(dict, db.execute(query)):
                    try:
                        context = fill(sealed.context, row)
                        plaintext, form = seal.open(row[column], context)
                    except KeyError as missing:
                        problems.append(f"{table}.{column}: its context names {missing}, no column")
                        break
                    except InvalidTag:
                        problems.append(f"{table}.{column}: a value does not open with `{context}`")
                        break
                    if not readable(plaintext, sealed.kind):
                        problems.append(f"{table}.{column}: a value is not {sealed.kind}")
                    found[(table, column)].append(Opened(row, context, plaintext, form))
    finally:
        db.close()
    return found, problems


def unsealed_writes(database: Path, blocks: dict) -> list[str]:
    """BLOB columns holding values that the contract does not seal."""
    problems, db = [], connect(database)
    try:
        for table, built in sorted(database_tables(database).items()):
            for column, kind, *_ in built.columns:
                sealed = column in blocks.get(table, Table()).sealed
                query = f'SELECT count(*) FROM "{table}" WHERE "{column}" IS NOT NULL'
                if kind == "BLOB" and not sealed and (count := db.execute(query).fetchone()[0]):
                    problems.append(
                        f"{table}.{column}: the hub sealed {count} values here, and"
                        f" the contract does not seal this column{absent()}"
                    )
    finally:
        db.close()
    return problems


# --- The hub writing every sealed column ------------------------------------------


@dataclass
class Written:
    owner: dict
    meeting: str
    session: str
    attachment: str
    proposal: str
    code: str
    operation: tuple
    control: tuple
    target: tuple


async def write_everything(distillation_client) -> Written:
    """A row in every sealed column, each written by the hub's own code.

    The fixture leaves an invented meeting with its audio, groups, final pass,
    speakers, owner, private notes and note, a vault folder and a model; the
    rest is written here through the hub's routes and repositories."""
    client, _, meeting, _, _, app, owner = distillation_client
    store, device = app.state.store, owner["device_id"]
    template = {
        "name": "Cedar review",
        "context": "An invented pilot.",
        "sections": [{"name": "Plan", "instructions": "What was planned."}],
    }
    for method, path, body, status in (
        ("PUT", f"/v1/recordings/{meeting}/title", {"title": "Cedar review"}, 200),
        ("PUT", f"/v1/reminders/{DIGEST}", {"text": "Bring the plan.", "edited_at_ms": 2000}, 200),
        (
            "POST",
            "/v1/meeting-dictionaries/orders",
            {"action": "save", "dictionary_id": str(uuid4()), "name": "Cedar terms"},
            200,
        ),
        ("PUT", "/v1/note-templates/cedar-review", template, 201),
        ("POST", "/v1/vault/read", {"operation": "read", "path": "Projects/cedar-pilot.md"}, 200),
    ):
        response = await client.request(method, path, json=body)
        assert response.status_code == status, (path, response.text[:400])
    note = f"/v1/recordings/{meeting}/note"
    revision = (await client.get(note)).json()["revision"]
    edits = {"base_revision": revision, "title": "Cedar, edited", "done": {"task-1": True}}
    assert (await client.put(note + "/edits", json=edits)).status_code == 200
    assert (await client.post(note, json={})).status_code == 202  # the edits are set aside
    proposal = (await prepare(client, meeting))["id"]
    pdf = pdf_bytes(["An invented page about the Cedar pilot."])
    response = await client.post(
        "/v1/attachments",
        content=pdf,
        headers={
            "X-Orbit-Attachment-Name": "cedar-plan.pdf",
            "X-Orbit-SHA256": hashlib.sha256(pdf).hexdigest(),
        },
    )
    assert response.status_code == 200, response.text[:400]
    attachment = response.json()["attachment"]["id"]
    configs = AgentConfigStore(store)
    configs.set_personality("default", "Answers in short sentences.")
    configs.put_skill("default", Skill("weekly-summary", "Sums up the week.", SKILL))
    configs.set_working_folder("default", "/home/example/project")
    Preferences(store).save("whisper", "invented-transcription-key")
    session = store.chat.create_session(device, "default", "rt", "stored", "epoch", 2000)
    store.chat.append_message(
        session, "user", "What starts?", now_ms=2000, attachments=[attachment]
    )
    store.chat.append_message(session, "assistant", "Cedar.", now_ms=2001, work={"tools": 1})
    operation = (str(uuid4()), {"method": "chat.send", "session_id": session, "text": "Hello"})
    store.operations.reserve(device, *operation)
    store.operations.finish(device, operation[0], {"id": operation[0], "status": "accepted"})
    target = ("chat.interrupt", session, "invented-turn", "")
    control = (str(uuid4()), {"method": "chat.interrupt", "session_id": session})
    key = safety.target_key(*target)
    safety.reserve(
        store,
        device,
        *control,
        target=key,
        session_id=session,
        turn_id=target[2],
        live_targets={key},
        validate=lambda: None,
    )
    safety.finish(store, device, key, {"id": control[0], "data": {"status": "interrupted"}})
    code = store.devices.create_code(2000)
    return Written(owner, meeting, session, attachment, proposal, code, operation, control, target)


def reads(store: Store, written: Written) -> dict:
    """What the hub reads through its own repositories, for what no route shows
    here: an agent's personality, skills and folder (the Hermes `default` agent
    keeps its own in Hermes), the devices, an operation's receipt and the AI
    settings (the fixture's registry is not one `GET /v1/settings/ai` reads)."""
    configs, device = AgentConfigStore(store), written.owner["device_id"]
    return {
        "personality": configs.personality("default"),
        "skills": configs.skills("default"),
        "working folder": configs.working_folder("default"),
        "devices": store.devices.all(),
        "operation": store.operations.get(device, written.operation[0]),
        "AI settings": Registry(store).read(),
    }


def hexdigest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def keyed(key: bytes, label: str, data: bytes) -> str:
    return hmac.new(hmac.digest(key, label.encode(), "sha256"), data, "sha256").hexdigest()


def derived_differences(database, opened, value, key, written: Written) -> list[str]:
    """Where what the contract derives differs from what the hub stored."""
    db, problems = connect(database), []

    def one(query, *arguments):
        return dict(db.execute(query, arguments).fetchone() or {})

    def compare(name, stored, derived):
        if stored != derived:
            problems.append(f"{name}: the hub stored {stored!r}, the contract derives {derived!r}")

    canonical = json.dumps(written.operation[1], sort_keys=True, separators=(",", ":")).encode()
    control = json.dumps(written.control[1], sort_keys=True, separators=(",", ":")).encode()
    safety_row = one("SELECT * FROM safety_operations WHERE operation_id=?", written.control[0])
    try:
        device = one("SELECT token_hash FROM devices WHERE id=?", written.owner["device_id"])
        token = hexdigest(written.owner["token"].encode())
        compare("devices.token_hash", device.get("token_hash"), token)
        code = one("SELECT hash FROM pair_codes WHERE hash=?", hexdigest(written.code.encode()))
        compare("pair_codes.hash", code.get("hash"), hexdigest(written.code.encode()))
        for table, column in (("chunks", "body"), ("attachments", "body")):
            for item in opened.get((table, column), []):
                compare(f"{table}.sha256", item.row["sha256"], hexdigest(item.plaintext))
                compare(f"{table}.size", item.row["size"], len(item.plaintext))
        stored = one("SELECT fingerprint FROM operations WHERE id=?", written.operation[0])
        derived = keyed(key, value["fingerprint_label"], canonical)
        compare("operations.fingerprint", stored.get("fingerprint"), derived)
        derived = keyed(key, value["fingerprint_label"], control)
        compare("safety_operations.fingerprint", safety_row.get("fingerprint"), derived)
        spaced = json.dumps(list(written.target), separators=(", ", ": "), ensure_ascii=True)
        compare("safety_operations.target", safety_row.get("target"), hexdigest(spaced.encode()))
        stored = one("SELECT key FROM personal_reminders")
        derived = keyed(key, value["reminder_label"], DIGEST.encode())
        compare("personal_reminders.key", stored.get("key"), derived)
        configuration = opened.get(("vault_configuration", "body"), [])
        folder = json.loads(configuration[0].plaintext) if configuration else {}
        identity = [folder.get(name, "") for name in ("kind", "path", "url", "vault")]
        namespace = hexdigest(json.dumps(identity, separators=(", ", ": ")).encode())
        for item in opened.get(("vault_working_copy", "body"), []):
            path = json.loads(item.plaintext)["path"]
            compare("vault_working_copy.namespace", item.row["namespace"], namespace)
            compare("vault_working_copy.id", item.row["id"], hexdigest(path.encode()))
    finally:
        db.close()
    return problems


# --- The tests ---------------------------------------------------------------------


async def test_a_new_database_has_the_schema_the_contract_writes(environment):
    settings, store, adapter = environment
    create_app(settings, store=store, assistant=adapter, speech=adapter)  # its domains' tables
    text = contract()
    value, problems = values_of(text)
    written, duplicated = tables_of(text)
    database = settings.data_dir / value.get("database", "hub.sqlite3")
    problems += duplicated + schema_differences(written, database_tables(database))
    db = connect(database)
    try:
        pragmas = {name: str(db.execute(f"PRAGMA {name}").fetchone()[0]) for name in PRAGMAS}
    finally:
        db.close()
    problems += [
        f"{name}: the database has {found}, the contract {value.get(name)}"
        for name, found in pragmas.items()
        if value.get(name) != found
    ]
    assert not problems, "\n".join(problems)


def test_every_context_the_code_seals_with_is_a_column_of_the_contract():
    found, unreadable = code_contexts()
    assert not unreadable, "contexts this test cannot read from the code:\n" + "\n".join(unreadable)
    text = contract()
    written, mark = tables_of(text)[0], values_of(text)[0].get("mark")
    sealed = {
        f"{table}.{column}": entry
        for table, block in written.items()
        for column, entry in block.sealed.items()
    }
    problems = [
        f"the code seals with `{shape}` ({', '.join(sorted(found[shape]))}), and no column of"
        f" the contract has that context{absent()}"
        for shape in sorted(found)
        if not any(matches(shape, s.context) for s in sealed.values() if not s.retired)
    ]
    for name, entry in sorted(sealed.items()):
        used = [shape for shape in found if matches(shape, entry.context)]
        if entry.retired and used:
            problems.append(f"{name} is retired in the contract, and the code uses {used}")
        elif not entry.retired and not used:
            problems.append(f"{name} is sealed with `{entry.context}`, which the code never uses")
        if mark and entry.context.startswith(mark):
            problems.append(f"{name}: a context may not begin with the key mark")
    assert not problems, "\n".join(problems)


async def test_what_the_hub_wrote_opens_by_the_contract_alone(environment, distillation_client):
    settings, _, _ = environment
    written = await write_everything(distillation_client)
    text = contract()
    value, problems = values_of(text)
    blocks, _ = tables_of(text)
    database = settings.data_dir / value.get("database", "hub.sqlite3")
    problems += unsealed_writes(database, blocks)
    missing = sorted(DERIVED - derived_of(text))
    if missing:
        problems.append(f"the Derived values table lacks {missing}{absent()}")
    if not set(VALUES) - value.keys():
        key = settings.key_file.read_bytes()
        opened, failures = open_all(database, blocks, Seal(key, value))
        problems += failures
        for table, block in sorted(blocks.items()):
            for column, sealed in block.sealed.items():
                if not sealed.retired and not opened.get((table, column)):
                    problems.append(f"{table}.{column}: this test made the hub write no value")
        problems += [
            f"{table}.{column}: the hub wrote a marked value; the contract says it writes"
            " today's form"
            for (table, column), items in sorted(opened.items())
            if any(item.form != "today" for item in items)
        ]
        [check] = opened.get(("storage_key_check", "body"), [None])
        if check is None or check.plaintext != value["key_check_plaintext"].encode():
            problems.append("storage_key_check.body does not hold the key check plaintext")
        problems += derived_differences(database, opened, value, key, written)
    assert not problems, "\n".join(problems)


async def test_the_hub_reads_what_the_contract_seals(environment, distillation_client):
    settings, _, adapter = environment
    written = await write_everything(distillation_client)
    client = distillation_client[0]
    text = contract()
    value, problems = values_of(text)
    assert not problems, "\n".join(problems)
    blocks, _ = tables_of(text)
    database = settings.data_dir / value["database"]
    seal = Seal(settings.key_file.read_bytes(), value)
    paths = [
        f"/v1/sessions/{written.session}/messages",
        f"/v1/recordings/{written.meeting}",
        f"/v1/recordings/{written.meeting}/turns",
        f"/v1/recordings/{written.meeting}/note",
        f"/v1/reminders/{DIGEST}",
        f"/v1/attachments/{written.attachment}",
        f"/v1/vault/distillations/{written.proposal}",
        "/v1/note-templates",
        "/v1/meeting-dictionaries",
        "/v1/settings/transcription",
        "/v1/vault/config",
    ]
    before = {path: await client.get(path) for path in paths}
    assert all(r.status_code == 200 for r in before.values()), {
        path: r.text[:200] for path, r in before.items() if r.status_code != 200
    }
    kept = reads(distillation_client[5].state.store, written)
    opened, problems = open_all(database, blocks, seal)
    assert opened and not problems, "\n".join(problems) or f"nothing sealed to read{absent()}"
    for marked in (False, True):
        form = "marked" if marked else "today's form"
        with sqlite3.connect(database) as db:
            for (table, column), items in opened.items():
                for item in items:
                    item.row[column] = seal.seal(item.plaintext, item.context, marked=marked)
                    db.execute(
                        f'UPDATE "{table}" SET "{column}"=? WHERE rowid=?',
                        (item.row[column], item.row["rowid"]),
                    )
        db.close()
        try:
            store = Store(settings.data_dir, settings.key_file)  # opens the key check first
        except ValueError as refused:
            problems.append(f"the hub refuses the database sealed in the {form}: {refused}")
            break
        try:
            for (table, column), items in sorted(opened.items()):
                try:
                    same = all(
                        store.decrypt(i.row[column], i.context) == i.plaintext for i in items
                    )
                except InvalidTag:
                    same = False
                if not same:
                    problems.append(f"{table}.{column}: the hub does not open the {form}")
            problems += [
                f"the hub's {name}, sealed in the {form}: {again!r}"
                for name, again in reads(store, written).items()
                if again != kept[name]
            ]
            app = create_app(
                settings, store=store, assistant=adapter, speech=adapter, now=lambda: 2000
            )
            token = {"Authorization": f"Bearer {written.owner['token']}"}
            async with httpx.AsyncClient(
                transport=httpx.ASGITransport(app), base_url="https://localhost", headers=token
            ) as fresh:
                for path, response in before.items():
                    again = await fresh.get(path)
                    if (again.status_code, again.content) != (200, response.content):
                        problems.append(f"GET {path}, sealed in the {form}: {again.text[:300]}")
            app.state.notes.executor.shutdown(wait=True)
        finally:
            store.close()
    assert not problems, "\n".join(problems)


async def test_a_serving_hub_holds_the_lock_the_contract_names(environment):
    settings, store, adapter = environment
    value, problems = values_of(contract())
    assert not problems, "\n".join(problems)
    app = create_app(settings, store=store, assistant=adapter, speech=adapter)
    lock = settings.data_dir / value["lock"]
    async with app.router.lifespan_context(app):
        with lock.open("rb") as other, pytest.raises(BlockingIOError):
            fcntl.flock(other, fcntl.LOCK_EX | fcntl.LOCK_NB)
    with lock.open("rb") as other:
        fcntl.flock(other, fcntl.LOCK_EX | fcntl.LOCK_NB)


def test_storage_blocks_are_read_as_written():
    """The reader of the contract, on a contract made up for it."""
    text = """
## Values

| name | value |
|---|---|
| `mark` | `XYZ1` |

#### `items`

| column | type | null | key | default | sealed |
|---|---|---|---|---|---|
| `id` | `TEXT` | yes | 1 | | |
| `state` | `TEXT` | no | | `'new'` | |
| `body` | `BLOB` | no | | | json, `item:{id}` |
| `old` | `BLOB` | yes | | | retired: text, `item-old:{id}` |

| kind | columns | detail |
|---|---|---|
| unique | `state,  body` | |
| references | `id` | `owners( id )` |
| check | `id = 1` | |
| index `items_recent` | `state, id desc` | `body IS NULL` |

## Derived values

| column | how |
|---|---|
| `items.id` | a UUID |
"""
    tables, problems = tables_of(text)
    item = tables["items"]
    assert not problems and not item.problems
    assert item.columns == [
        ("id", "TEXT", "yes", "1", ""),
        ("state", "TEXT", "no", "", "'new'"),
        ("body", "BLOB", "no", "", ""),
        ("old", "BLOB", "yes", "", ""),
    ]
    assert item.sealed == {
        "body": Sealed("json", "item:{id}", False),
        "old": Sealed("text", "item-old:{id}", True),
    }
    assert item.constraints == {
        ("unique", "state, body"),
        ("references", "id", "owners(id)"),
        ("check", "id=1"),
        ("index", "items_recent", "state, id DESC", "bodyISNULL"),
    }
    assert values_of(text)[0] == {"mark": "XYZ1"} and derived_of(text) == {"items.id"}
    assert matches("note-{}:{}:{}", "note-edits:{device_id}:{recording}")
    assert not matches("note-{}:{}:{}", "note-edits:{device_id}")
    assert fill("item:{id}", {"id": "a1"}) == "item:a1"
