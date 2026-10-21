"""Local administrator commands. Starting the service is always explicit."""

import argparse
import json
import sys
import time
from pathlib import Path

from .config import Settings
from .protocol import HubError
from .storage import Store, initialize


def main() -> None:
    parser = argparse.ArgumentParser(description="Servidor de dispositivos Orbit")
    parser.add_argument("--config", type=Path, required=True)
    commands = parser.add_subparsers(dest="command", required=True)
    commands.add_parser("init", help="Crear clave y almacenamiento cifrado (solo una vez)")
    pair = commands.add_parser("pair-code", help="Crear código de un solo uso")
    pair.add_argument("--ttl", type=int, default=300)
    revoke = commands.add_parser("revoke", help="Revocar un dispositivo")
    revoke.add_argument("device_id")
    commands.add_parser("devices", help="Mostrar identidades y revocación")
    serve = commands.add_parser("serve", help="Iniciar HTTP/WebSocket")
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
            print("Almacenamiento cifrado inicializado. Conserva una copia segura de la clave.")
            return
        if args.command == "serve":
            if not args.tls_cert and not (
                settings.allow_insecure_localhost and args.host in {"127.0.0.1", "::1", "localhost"}
            ):
                raise ValueError("Usa TLS o habilita desarrollo localhost explícitamente")
            if bool(args.tls_cert) != bool(args.tls_key):
                raise ValueError("TLS requiere certificado y clave")
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
                    "Credencial revocada. El hub solicitará interrupción y cerrará sus conexiones; "
                    "esto no revierte acciones ya ejecutadas."
                )
            else:
                print(json.dumps(store.devices(), indent=2))
        finally:
            store.close()
    except (OSError, ValueError, HubError) as exc:
        print(f"Orbit Hub: {exc}", file=sys.stderr)
        raise SystemExit(1) from None


if __name__ == "__main__":
    main()
