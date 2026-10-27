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
                "Indica la URL HTTP(S), el nombre del vault y un token válido, sin credenciales en la URL.",
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
                f"La lectura de Fast Note Sync superó {CONNECTOR_SECONDS} segundos.",
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
                        "Fast Note Sync rechazó el token; revisa su vigencia y acceso al vault.",
                        400,
                        upstream_status=response.status_code,
                    )
                if response.status_code != 200:
                    raise refusal(
                        "vault_sync_http",
                        f"Fast Note Sync respondió HTTP {response.status_code}.",
                        upstream_status=response.status_code,
                    )
                body = bytearray()
                for chunk in response.iter_bytes(chunk_size=64 * 1024):
                    if monotonic() > self.deadline:
                        raise refusal(
                            "vault_sync_timeout",
                            f"La lectura de Fast Note Sync superó {CONNECTOR_SECONDS} segundos.",
                            timeout_seconds=CONNECTOR_SECONDS,
                        )
                    body.extend(chunk)
                    if len(body) > MAX_UPSTREAM_BYTES:
                        raise refusal(
                            "vault_response_too_large",
                            f"La respuesta de Fast Note Sync supera {MAX_UPSTREAM_BYTES} bytes.",
                            413,
                            bytes=len(body),
                            max_bytes=MAX_UPSTREAM_BYTES,
                        )
            envelope = json.loads(body)
            code = envelope.get("code")
            if (
                envelope.get("status") is not True
                or type(code) is not int
                or code not in range(1, 7)
            ):
                self._business_failure(code)
            return envelope["data"]
        except httpx.TimeoutException:
            raise refusal(
                "vault_sync_timeout",
                f"Fast Note Sync no respondió dentro de {CONNECTOR_SECONDS} segundos.",
                timeout_seconds=CONNECTOR_SECONDS,
            ) from None
        except httpx.HTTPError:
            raise refusal(
                "vault_sync_connection",
                "No se pudo conectar con Fast Note Sync; revisa la URL y el servicio.",
            ) from None
        except (ValueError, TypeError, KeyError, AttributeError, RecursionError):
            raise refusal(
                "vault_sync_shape", "Fast Note Sync devolvió una respuesta con formato no válido."
            ) from None

    @staticmethod
    def _business_failure(code):
        numbers = {"upstream_code": code} if type(code) is int else {}
        causes = {
            414: ("vault_sync_missing", "Fast Note Sync no encuentra ese vault."),
            428: ("vault_note_missing", "La nota no existe en Fast Note Sync."),
            430: ("vault_note_missing", "La nota no existe en Fast Note Sync."),
            507: ("vault_token_refused", "Fast Note Sync requiere un token válido."),
            508: ("vault_token_expired", "El token de Fast Note Sync ha caducado."),
        }
        code_name, message = causes.get(
            code, ("vault_sync_refused", "Fast Note Sync rechazó la lectura del vault.")
        )
        raise refusal(
            code_name, message, 404 if code_name == "vault_note_missing" else 400, **numbers
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
                "vault_sync_shape", "La lista de Fast Note Sync no tiene el formato esperado."
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
                    f"El vault supera {MAX_SCAN_ENTRIES} entradas por recorrido.",
                    entries=total,
                    max_entries=MAX_SCAN_ENTRIES,
                )
            entries.extend(batch)
            if page * 100 >= total:
                return entries
        raise refusal(
            "vault_scan_limit",
            "El catálogo de Fast Note Sync no termina dentro del límite.",
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
                "vault_sync_shape", "Fast Note Sync devolvió una nota con formato o ruta distintos."
            )
        return data["content"]

    def linked(self, path, direction):
        data = self.get("note/" + direction, path=checked(path))
        if not isinstance(data, list):
            raise refusal(
                "vault_sync_shape", "Fast Note Sync devolvió enlaces con formato no válido."
            )
        if len(data) > MAX_SCAN_ENTRIES:
            raise refusal(
                "vault_scan_limit",
                f"La nota supera {MAX_SCAN_ENTRIES} enlaces.",
                entries=len(data),
                max_entries=MAX_SCAN_ENTRIES,
            )
        entries = []
        for item in data:
            if not isinstance(item, dict) or not isinstance(item.get("path"), str):
                raise refusal("vault_sync_shape", "Fast Note Sync devolvió un enlace sin ruta.")
            if visible(item["path"]) and item["path"].lower().endswith(".md"):
                entries.append({"path": checked(item["path"]), "kind": "note"})
        return entries
