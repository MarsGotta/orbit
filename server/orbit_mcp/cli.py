"""Console entry point for the stdio-only Orbit MCP server."""

import sys

from .config import Config, ConfigError
from .hub import HubClient
from .server import build_server


def main() -> int:
    try:
        config = Config.from_env()
    except ConfigError as exc:
        print(f"Orbit MCP no puede arrancar: {exc}", file=sys.stderr)
        return 1
    build_server(config, HubClient(config)).run("stdio")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
