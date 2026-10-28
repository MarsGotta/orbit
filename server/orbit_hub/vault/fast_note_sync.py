"""Fast Note Sync's REST reader (master docs, token header, no mutation endpoints)."""

import json
from time import monotonic
from urllib.parse import urlsplit

import httpx

from .models import CONNECTOR_SECONDS, MAX_SCAN_ENTRIES, MAX_UPSTREAM_BYTES, refusal
from .paths import checked, visible
from .privacy import private_read


class FastNoteSync:
    def __init__(self, configuration):
        self.config = configuration
        self.deadline = monotonic() + CONNECTOR_SECONDS
        self.client = None
        try:
            url = urlsplit(configuration.url)
            _port = url.port
            valid = (
                url.scheme in {"https", "http"}
                and url.hostname
                and not url.username
                and not url.password
                and not url.query
                and not url.fragment
            )
            # Explicit local/self-hosted HTTP is supported; production transport
            # is the operator's configuration, never inferred from a default host.
            if not valid or not configuration.vault or not configuration.token:
                raise ValueError()
            configuration.token.get_secret_value().encode("ascii")
            if any(ord(c) < 33 or ord(c) == 127 for c in configuration.token.get_secret_value()):
                raise ValueError()
        except (ValueError, UnicodeError):
            raise refusal(
                "vault_connection_fields",
                "Enter the HTTP(S) URL, vault name and a valid token, without credentials in the URL.",
            ) from None

    def close(self):
        if self.client is not None:
            with private_read():
                self.client.close()

    def get(self, endpoint, **parameters):
        """Stream a bounded envelope. Never return upstream messages/details."""
        token = self.config.token.get_secret_value()
        if self.client is None:
            self.client = httpx.Client(follow_redirects=False, trust_env=False)
        remaining = self.deadline - monotonic()
        if remaining <= 0:
            raise refusal(
                "vault_sync_timeout",
                f"The Fast Note Sync read exceeded {CONNECTOR_SECONDS} seconds.",
                message_code="vault_sync_timeout",
                timeout_seconds=CONNECTOR_SECONDS,
            )
        try:
            with (
                private_read(),
                self.client.stream(
                    "GET",
                    self.config.url.rstrip("/") + "/api/" + endpoint,
                    params={"vault": self.config.vault, **parameters},
                    headers={"token": token},
                    timeout=remaining,
                ) as response,
            ):
                if response.status_code in {401, 403}:
                    raise refusal(
                        "vault_token_refused",
                        "Fast Note Sync rejected the token; check its validity and vault access.",
                        400,
                        upstream_status=response.status_code,
                    )
                if response.status_code != 200:
                    raise refusal(
                        "vault_sync_http",
                        f"Fast Note Sync returned HTTP {response.status_code}.",
                        upstream_status=response.status_code,
                    )
                body = bytearray()
                for chunk in response.iter_bytes(chunk_size=64 * 1024):
                    if monotonic() > self.deadline:
                        raise refusal(
                            "vault_sync_timeout",
                            f"The Fast Note Sync read exceeded {CONNECTOR_SECONDS} seconds.",
                            message_code="vault_sync_timeout",
                            timeout_seconds=CONNECTOR_SECONDS,
                        )
                    body.extend(chunk)
                    if len(body) > MAX_UPSTREAM_BYTES:
                        raise refusal(
                            "vault_response_too_large",
                            f"The Fast Note Sync response exceeds {MAX_UPSTREAM_BYTES} bytes.",
                            413,
                            message_code="vault_sync_response_too_large",
                            bytes=len(body),
                            max_bytes=MAX_UPSTREAM_BYTES,
                        )
            return self._envelope(body)
        except httpx.TimeoutException:
            raise refusal(
                "vault_sync_timeout",
                f"Fast Note Sync did not respond within {CONNECTOR_SECONDS} seconds.",
                message_code="vault_sync_no_response",
                timeout_seconds=CONNECTOR_SECONDS,
            ) from None
        except httpx.HTTPError:
            raise refusal(
                "vault_sync_connection",
                "Could not connect to Fast Note Sync; check the URL and service.",
            ) from None
        except (ValueError, TypeError, KeyError, AttributeError, RecursionError):
            raise refusal(
                "vault_sync_shape",
                "Fast Note Sync returned an invalid response format.",
                message_code="vault_sync_shape",
            ) from None

    def _envelope(self, body):
        envelope = json.loads(body)
        code = envelope.get("code")
        if envelope.get("status") is not True or type(code) is not int or code not in range(1, 7):
            self._business_failure(code)
        return envelope["data"]

    @staticmethod
    def _business_failure(code):
        numbers = {"upstream_code": code} if type(code) is int else {}
        causes = {
            414: ("vault_sync_missing", "Fast Note Sync cannot find that vault."),
            428: ("vault_note_missing", "The note does not exist in Fast Note Sync."),
            430: ("vault_note_missing", "The note does not exist in Fast Note Sync."),
            507: ("vault_token_refused", "Fast Note Sync requires a valid token."),
            508: ("vault_token_expired", "The Fast Note Sync token has expired."),
        }
        code_name, message = causes.get(
            code, ("vault_sync_refused", "Fast Note Sync refused the vault read.")
        )
        raise refusal(
            code_name,
            message,
            404 if code_name == "vault_note_missing" else 400,
            message_code={
                428: "vault_sync_note_missing",
                430: "vault_sync_note_missing",
                507: "vault_sync_token_required",
            }.get(code),
            **numbers,
        )

    def page(self, endpoint="notes", **parameters):
        data = self.get(endpoint, **parameters)
        try:
            listing, pager = data["list"], data["pager"]
            if not isinstance(listing, list) or len(listing) > parameters["pageSize"]:
                raise ValueError()
            if type(pager["totalRows"]) is not int or pager["totalRows"] < 0:
                raise ValueError()
            if (
                pager.get("page") != parameters["page"]
                or pager.get("pageSize") != parameters["pageSize"]
            ):
                raise ValueError()
            entries = []
            for entry in listing:
                path = entry["path"]
                if isinstance(path, str) and visible(path) and path.lower().endswith(".md"):
                    stamp = [entry.get(key) for key in ("contentHash", "version", "mtime")]
                    reliable = any(isinstance(value, (str, int)) and value for value in stamp)
                    entries.append(
                        {
                            "path": checked(path),
                            "kind": "note",
                            "stamp": stamp if reliable else None,
                        }
                    )
            return entries, pager["totalRows"]
        except (KeyError, TypeError, ValueError):
            raise refusal(
                "vault_sync_shape",
                "The Fast Note Sync list has an unexpected format.",
                message_code="vault_sync_list_shape",
            ) from None

    def catalog(self, folder=None):
        entries = []
        for page in range(1, MAX_SCAN_ENTRIES // 100 + 1):
            parameters = {"path": folder} if folder is not None else {}
            batch, total = self.page(
                "folder/notes" if folder is not None else "notes",
                page=page,
                pageSize=100,
                **parameters,
            )
            if total > MAX_SCAN_ENTRIES:
                raise refusal(
                    "vault_scan_limit",
                    f"The vault exceeds {MAX_SCAN_ENTRIES} entries per scan.",
                    message_code="vault_sync_scan_limit",
                    entries=total,
                    max_entries=MAX_SCAN_ENTRIES,
                )
            entries.extend(batch)
            if page * 100 >= total:
                return entries
        raise refusal(
            "vault_scan_limit",
            "The Fast Note Sync catalogue does not finish within the limit.",
            message_code="vault_sync_catalog_incomplete",
            max_entries=MAX_SCAN_ENTRIES,
        )

    def read(self, path):
        data = self.get("note", path=checked(path))
        if (
            not isinstance(data, dict)
            or not isinstance(data.get("content"), str)
            or data.get("path") != path
        ):
            raise refusal(
                "vault_sync_shape",
                "Fast Note Sync returned a note with a different format or path.",
                message_code="vault_sync_note_shape",
            )
        return data["content"]

    def linked(self, path, direction):
        data = self.get("note/" + direction, path=checked(path))
        if not isinstance(data, list):
            raise refusal(
                "vault_sync_shape",
                "Fast Note Sync returned invalid links.",
                message_code="vault_sync_links_shape",
            )
        if len(data) > MAX_SCAN_ENTRIES:
            raise refusal(
                "vault_scan_limit",
                f"The note exceeds {MAX_SCAN_ENTRIES} links.",
                message_code="vault_sync_links_limit",
                entries=len(data),
                max_entries=MAX_SCAN_ENTRIES,
            )
        entries = []
        for item in data:
            if not isinstance(item, dict) or not isinstance(item.get("path"), str):
                raise refusal(
                    "vault_sync_shape",
                    "Fast Note Sync returned a link without a path.",
                    message_code="vault_sync_link_path",
                )
            if visible(item["path"]) and item["path"].lower().endswith(".md"):
                entries.append({"path": checked(item["path"]), "kind": "note"})
        return entries
