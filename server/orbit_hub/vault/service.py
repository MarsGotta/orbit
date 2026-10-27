"""One read-only connector contract and the vault's encrypted, hub-owned settings."""

import json
from threading import BoundedSemaphore
from typing import Protocol

from . import index
from .fast_note_sync import FastNoteSync
from .folder import Folder
from .markdown import document
from .models import MAX_READS, MAX_SCAN_BYTES, Configuration, Read, refusal
from .paths import checked
from .syntax import activities
from .working_copy import WorkingCopy


class Connector(Protocol):
    def close(self) -> None: ...
    def catalog(self) -> list[dict]: ...
    def read(self, path: str) -> str: ...


def connector(config: Configuration) -> Connector:
    return Folder(config) if config.kind == "folder" else FastNoteSync(config)


def page(entries, request):
    start = (request.page - 1) * request.page_size
    return {
        "entries": [
            {k: v for k, v in entry.items() if k != "stamp"}
            for entry in entries[start : start + request.page_size]
        ],
        "next_page": request.page + 1 if start + request.page_size < len(entries) else None,
    }


class Vault:
    def __init__(self, store):
        self.store = store
        self.copy = WorkingCopy(store)
        self.read_slots = BoundedSemaphore(MAX_READS)
        with store.transaction() as db:
            db.execute(
                "CREATE TABLE IF NOT EXISTS vault_configuration (id INTEGER PRIMARY KEY CHECK(id=1), body BLOB NOT NULL)"
            )

    def configuration(self):
        with self.store.read_snapshot() as db:
            row = db.execute("SELECT body FROM vault_configuration WHERE id=1").fetchone()
        if row is None:
            return None
        return Configuration.model_validate_json(
            self.store.decrypt(row[0], "vault-configuration:v1")
        )

    def public(self):
        config = self.configuration()
        return config.public() if config else {"configured": False}

    def configure(self, config):
        previous = self.configuration()
        if (
            config.kind == "fast_note_sync"
            and config.token is None
            and previous
            and (previous.kind, previous.url, previous.vault)
            == (
                config.kind,
                config.url,
                config.vault,
            )
        ):
            config = config.model_copy(update={"token": previous.token})
        reader = connector(config)
        # An actual listing read validates the path, credential and selected vault.
        # Empty vaults are valid. Failed reads never replace the saved connector.
        try:
            if isinstance(reader, FastNoteSync):
                reader.page(page=1, pageSize=1)
            else:
                reader.catalog()
        finally:
            reader.close()
        values = config.model_dump(exclude={"token"})
        if config.token:
            values["token"] = config.token.get_secret_value()
        sealed = self.store.encrypt(json.dumps(values).encode(), "vault-configuration:v1")
        with self.store.transaction() as db:
            if previous and (previous.kind, previous.path, previous.url, previous.vault) != (
                config.kind,
                config.path,
                config.url,
                config.vault,
            ):
                self.copy.clear(db)
            db.execute(
                "INSERT INTO vault_configuration VALUES(1,?) ON CONFLICT(id) DO UPDATE SET body=excluded.body",
                (sealed,),
            )
        return config.public()

    def disconnect(self):
        with self.store.transaction() as db:
            self.copy.clear(db)
            db.execute("DELETE FROM vault_configuration")
        return {"configured": False}

    def read(self, request: Read):
        if not self.read_slots.acquire(blocking=False):
            raise refusal(
                "vault_busy",
                f"El hub ya está leyendo {MAX_READS} peticiones del vault; espera a que terminen.",
                429,
                in_flight=MAX_READS,
                max=MAX_READS,
            )
        try:
            return self._read(request)
        finally:
            self.read_slots.release()

    def _read(self, request: Read):
        config = self.configuration()
        if config is None:
            raise refusal(
                "vault_not_connected", "Conecta una carpeta del hub o Fast Note Sync en Notas.", 409
            )
        remote = connector(config)
        try:
            return self._dispatch(remote, self.copy.reader(remote, config), request)
        finally:
            remote.close()

    def _dispatch(self, remote, reader, request):
        checked(
            request.path,
            folder=request.operation in {"list", "search", "index", "catalog", "tasks"},
        )
        if request.operation == "search" and isinstance(remote, FastNoteSync):
            entries, total = remote.page(
                keyword=request.query,
                searchContent=str(request.content).lower(),
                page=request.page,
                pageSize=request.page_size,
            )
            return {
                "entries": [{k: v for k, v in entry.items() if k != "stamp"} for entry in entries],
                "next_page": request.page + 1 if request.page * request.page_size < total else None,
            }
        if request.operation in {"backlinks", "outlinks"} and isinstance(remote, FastNoteSync):
            return page(remote.linked(request.path, request.operation), request)
        entries = reader.catalog()
        paths = {entry["path"] for entry in entries if entry["kind"] == "note"}
        reader.prune(paths)
        if request.operation in {"catalog", "tasks"}:
            built = index.build(reader, entries)
            return (
                index.catalog(built, request)
                if request.operation == "catalog"
                else index.task_page(built, request)
            )
        if request.operation == "read":
            return self._note(reader, paths, request)
        if request.operation == "list":
            if isinstance(remote, FastNoteSync):
                folders = [
                    item for item in self._folder(entries, request.path) if item["kind"] == "folder"
                ]
                return page(folders + remote.catalog(folder=request.path), request)
            return page(self._folder(entries, request.path), request)
        if request.operation == "index":
            if "index.md" in paths:
                return document("index.md", reader.read("index.md"), paths)
            listing = page([entry for entry in entries if entry["kind"] == "note"], request)
            body = "# Notas\n\n" + "\n".join(
                f"- [[{entry['path']}]]" for entry in listing["entries"]
            )
            return {
                **document("index.md", body, paths),
                "synthesized": True,
                "next_page": listing["next_page"],
            }
        if request.operation == "outlinks":
            note = document(request.path, reader.read(request.path), paths)
            return page(
                [{"path": link["path"], "kind": "note"} for link in note["outlinks"]], request
            )
        if request.operation == "backlinks" and request.path not in paths:
            raise refusal("vault_note_missing", "La nota no existe en el vault.", 404)
        return page(self._scan(reader, entries, paths, request), request)

    @staticmethod
    def _note(reader, paths, request):
        note = document(request.path, reader.read(request.path), paths)
        history = (
            activities(document("log.md", reader.read("log.md"), paths)["body"])
            if "log.md" in paths
            else []
        )
        history = [entry for entry in history if entry["path"] == request.path]
        start = (request.page - 1) * request.page_size
        return {
            **note,
            "history": history[start : start + request.page_size],
            "history_next_page": request.page + 1
            if start + request.page_size < len(history)
            else None,
        }

    @staticmethod
    def _folder(entries, path):
        prefix = path + "/" if path else ""
        found = {}
        for entry in entries:
            if not entry["path"].startswith(prefix):
                continue
            tail = entry["path"][len(prefix) :]
            if not tail:
                continue
            name, slash, _ = tail.partition("/")
            key = prefix + name
            found[key] = {"path": key, "kind": "folder" if slash else entry["kind"]}
        return sorted(found.values(), key=lambda item: (item["kind"] != "folder", item["path"]))

    @staticmethod
    def _scan(reader, entries, paths, request):
        found, scanned = [], 0
        for entry in entries:
            if entry["kind"] != "note":
                continue
            path = entry["path"]
            if request.operation == "search" and (
                not request.path or path.startswith(request.path + "/")
            ):
                if request.query.casefold() in path.casefold():
                    found.append(entry)
                    continue
                if not request.content:
                    continue
            elif request.operation == "search":
                continue
            text = reader.read(path)
            scanned += len(text.encode("utf-8"))
            if scanned > MAX_SCAN_BYTES:
                raise refusal(
                    "vault_scan_bytes",
                    f"La búsqueda supera {MAX_SCAN_BYTES} bytes leídos; acota la carpeta.",
                    bytes=scanned,
                    max_bytes=MAX_SCAN_BYTES,
                )
            if request.operation == "search":
                matches = request.query.casefold() in text.casefold()
            else:
                matches = any(
                    link["path"] == request.path for link in document(path, text, paths)["outlinks"]
                )
            if matches:
                if request.operation == "backlinks":
                    note = document(path, text, paths)
                    targets = [
                        link["target"] for link in note["outlinks"] if link["path"] == request.path
                    ]
                    snippet = next(
                        (
                            line
                            for line in text.splitlines()
                            if any(target in line for target in targets)
                        ),
                        "",
                    )
                    entry = {
                        **entry,
                        "title": str(note["frontmatter"].get("title", path)),
                        "type": str(note["frontmatter"].get("type", "")),
                        "snippet": snippet[:240],
                    }
                found.append(entry)
        return found
