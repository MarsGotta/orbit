"""A real loopback REST peer implementing Fast Note Sync's versioned note API.

Like the upstream REST handler, baseHash is received but not enforced. Do not
make the fake promise a compare-and-swap or batch API the real service lacks.
"""

import hashlib
import json
import threading
from contextlib import contextmanager
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import parse_qs, urlsplit


@contextmanager
def sync_peer(files):
    notes = {
        p: {
            "path": p,
            "content": text,
            "version": 1,
            "contentHash": hashlib.md5(text.encode()).hexdigest(),
        }
        for p, text in files.items()
    }
    state = {
        "notes": notes,
        "writes": [],
        "fail_path": None,
        "lose_receipt": None,
        "histories": {},
        "calls": [],
    }

    class Handler(BaseHTTPRequestHandler):
        def log_message(self, *_args):
            pass

        def respond(self, data=None, code=1):
            body = json.dumps(
                {
                    "status": code == 1,
                    "code": code,
                    "data": data,
                    "message": "private upstream text",
                }
            ).encode()
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def admitted(self):
            assert self.headers.get("token") == "invented-token"
            return {k: v[0] for k, v in parse_qs(urlsplit(self.path).query).items()}

        def do_GET(self):
            query = self.admitted()
            assert query["vault"] == "Invented Garden"
            path = urlsplit(self.path).path
            state["calls"].append(("GET", path, query))
            if path == "/api/note":
                self.respond(notes.get(query["path"]), 1 if query["path"] in notes else 430)
            elif path == "/api/notes":
                page, size = int(query["page"]), int(query["pageSize"])
                listed = [notes[p] for p in sorted(notes)]
                self.respond(
                    {
                        "list": listed[(page - 1) * size : page * size],
                        "pager": {"page": page, "pageSize": size, "totalRows": len(listed)},
                    }
                )
            else:
                self.send_error(404)

        def do_POST(self):
            self.admitted()
            value = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
            assert value["vault"] == "Invented Garden"
            path = value["path"]
            state["writes"].append(value)
            if state["fail_path"] == path:
                self.respond(code=500)
                return
            old = notes.get(path)
            if value.get("createOnly") and old:
                self.respond(code=409)
                return
            if old:
                state["histories"].setdefault(path, []).append(dict(old))
            notes[path] = {
                "path": path,
                "content": value["content"],
                "version": old["version"] + 1 if old else 1,
                "contentHash": hashlib.md5(value["content"].encode()).hexdigest(),
            }
            if state["lose_receipt"] == path:
                self.close_connection = True
                return
            self.respond(notes[path])

        def do_DELETE(self):
            query = self.admitted()
            path = query["path"]
            if path in notes:
                state["histories"].setdefault(path, []).append(notes.pop(path))
            self.respond()

    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        yield f"http://127.0.0.1:{server.server_port}", state
    finally:
        server.shutdown()
        server.server_close()
        thread.join()
