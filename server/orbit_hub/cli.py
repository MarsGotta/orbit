"""Local administrator commands. Starting the service is always explicit."""

import argparse
import json
import sqlite3
import sys
import time
from datetime import UTC, datetime
from pathlib import Path

from .backup import BackupError, nightly, restore_copy
from .config import Settings
from .protocol import HubError
from .storage import Store, initialize
from .urls import LOOPBACK_HOSTS


def main() -> None:
    parser = argparse.ArgumentParser(description="Orbit device hub")
    parser.add_argument("--config", type=Path, required=True)
    commands = parser.add_subparsers(dest="command", required=True)
    commands.add_parser("init", help="Create the key and the encrypted store (once)")
    pair = commands.add_parser("pair-code", help="Create a one-time pairing code")
    pair.add_argument("--ttl", type=int, default=300)
    revoke = commands.add_parser("revoke", help="Revoke a device")
    revoke.add_argument("device_id")
    commands.add_parser("devices", help="List device identities and revocation")
    backup = commands.add_parser(
        "backup", help="Copy the database into a directory, keeping 7 daily and 4 weekly copies"
    )
    backup.add_argument("destination", type=Path)
    restore = commands.add_parser(
        "restore", help="Put a copy into an empty data directory and check it with the key"
    )
    restore.add_argument("copy", type=Path)
    serve = commands.add_parser("serve", help="Serve HTTP and WebSocket")
    serve.add_argument("--host", default="127.0.0.1")
    serve.add_argument("--port", type=int, default=8788)
    serve.add_argument("--tls-cert", type=Path)
    serve.add_argument("--tls-key", type=Path)
    args = parser.parse_args()
    try:
        settings = Settings.load(args.config)
        if args.command == "init":
            initialize(settings.data_dir, settings.key_file)
            Store(settings.data_dir, settings.key_file).close()
            print("Encrypted store initialized. Keep a safe copy of the key.")
            return
        if args.command == "backup":
            # The key stays out of the copy (docs/self-hosting.md, "Backups").
            today = datetime.now(UTC).astimezone().date()  # the server's own day
            for line in nightly(settings.data_dir, args.destination, today):
                print(line)
            return
        if args.command == "restore":
            size = restore_copy(args.copy, settings.data_dir)
            try:
                Store(settings.data_dir, settings.key_file).close()
            except BaseException:
                # The data directory was empty: leave it so.
                for suffix in ("", "-wal", "-shm"):
                    (settings.data_dir / f"hub.sqlite3{suffix}").unlink(missing_ok=True)
                raise
            print(
                f"Restored {args.copy} ({size} bytes) into {settings.data_dir}; "
                "it opens with the configured key."
            )
            return
        if args.command == "serve":
            if not args.tls_cert and not (
                settings.allow_insecure_localhost and args.host in LOOPBACK_HOSTS
            ):
                raise ValueError(
                    "serve requires TLS unless allow_insecure_localhost is set and the host is loopback"
                )
            if bool(args.tls_cert) != bool(args.tls_key):
                raise ValueError("TLS requires both --tls-cert and --tls-key")
            import uvicorn

            from .app import create_app

            uvicorn.run(
                create_app(settings),
                host=args.host,
                port=args.port,
                ssl_certfile=str(args.tls_cert) if args.tls_cert else None,
                ssl_keyfile=str(args.tls_key) if args.tls_key else None,
                ws_max_size=settings.max_frame_bytes,
                ws_max_queue=16,
                limit_concurrency=64,
                access_log=False,
                proxy_headers=False,
            )
            return
        store = Store(settings.data_dir, settings.key_file)
        try:
            if args.command == "pair-code":
                print(store.create_code(time.time(), args.ttl))
            elif args.command == "revoke":
                store.revoke(args.device_id)
                print(
                    "Credential revoked. The hub will request interruption and close its "
                    "connections; this does not undo actions already performed."
                )
            else:
                print(json.dumps(store.devices(), indent=2))
        finally:
            store.close()
    except (OSError, ValueError, HubError, BackupError, sqlite3.Error) as exc:
        print(f"Orbit Hub: {exc}", file=sys.stderr)
        raise SystemExit(1) from None


if __name__ == "__main__":
    main()
