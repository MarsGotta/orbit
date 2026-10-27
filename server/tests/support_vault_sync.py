"""Synthetic Fast Note Sync HTTP peer; no ports or services outside this process."""

import json
import threading
from contextlib import contextmanager
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import parse_qs, urlsplit

TOKEN = "invented-vault-token"
NOTES = {"index.md": "# Garden\n[[People/Ada]]", "People/Ada.md": "# Ada\nGardener."}


@contextmanager
def sync_server():
    calls = []

    class Handler(BaseHTTPRequestHandler):
        def log_message(self, *_args):
            pass

        def do_GET(self):
            url = urlsplit(self.path)
            query = {k: v[0] for k, v in parse_qs(url.query).items()}
            calls.append((url.path, query, self.headers.get("token")))
            data = None
            code, status = 1, True
            if self.headers.get("token") != TOKEN:
                code, status = 508, False
            elif query.get("vault") != "Invented Garden":
                code, status = 414, False
            elif url.path == "/api/note":
                path = query.get("path")
                if path in NOTES:
                    data = {"path": path, "content": NOTES[path], "size": len(NOTES[path])}
                else:
                    code, status = 430, False
            elif url.path in {"/api/notes", "/api/folder/notes"}:
                paths = sorted(NOTES)
                if url.path.endswith("folder/notes"):
                    paths = [p for p in paths if p.rpartition("/")[0] == query.get("path", "")]
                keyword = query.get("keyword", "").casefold()
                paths = [
                    p
                    for p in paths
                    if keyword in p.casefold()
                    or (query.get("searchContent") == "true" and keyword in NOTES[p].casefold())
                ]
                size, page = int(query.get("pageSize", 10)), int(query.get("page", 1))
                data = {
                    "list": [
                        {"path": p, "size": len(NOTES[p])}
                        for p in paths[(page - 1) * size : page * size]
                    ],
                    "pager": {"page": page, "pageSize": size, "totalRows": len(paths)},
                }
            elif url.path == "/api/note/backlinks":
                data = [
                    {
                        "path": "index.md",
                        "context": "private upstream context",
                        "isEmbed": False,
                        "linkText": "People/Ada",
                    }
                ]
            elif url.path == "/api/note/outlinks":
                data = [
                    {
                        "path": "People/Ada.md",
                        "context": "private upstream context",
                        "isEmbed": False,
                    }
                ]
            else:
                self.send_error(404)
                return
            body = json.dumps(
                {
                    "code": code,
                    "status": status,
                    "message": "never relay this private message",
                    "data": data,
                }
            ).encode()
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        yield f"http://127.0.0.1:{server.server_port}", calls
    finally:
        server.shutdown()
        server.server_close()
        thread.join()
