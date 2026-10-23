"""What `pnpm licenses list --json` and `cargo metadata` print, for their doubles.

scripts/macos/bundle.sh writes the third-party notice from both listings
(scripts/release/third-party-notices.py). A double of pnpm or cargo answers
those two calls with the files written here: one interface package and one
crate of the app, each with an invented licence in a directory of the test.
"""

import json
from pathlib import Path


def listings(root: Path) -> tuple[Path, Path]:
    """Writes both listings under `root` and returns their paths."""
    interface = root / "packages" / "ejemplo-ui"
    crate = root / "packages" / "ejemplo-crate"
    for directory, who in ((interface, "Ana Ruiz"), (crate, "Carla Gil")):
        directory.mkdir(parents=True, exist_ok=True)
        (directory / "LICENSE").write_text(f"MIT License\n\nCopyright (c) 2024 {who}\n")
    pnpm = root / "pnpm-licenses.json"
    pnpm.write_text(
        json.dumps(
            {
                "MIT": [
                    {
                        "name": "ejemplo-ui",
                        "versions": ["1.0.0"],
                        "paths": [str(interface)],
                        "license": "MIT",
                    }
                ]
            }
        )
    )
    cargo = root / "cargo-metadata.json"
    app = {"id": "orbit-desktop 0.0.0", "name": "orbit-desktop", "version": "0.0.0"}
    cargo.write_text(
        json.dumps(
            {
                "packages": [
                    {**app, "license": None, "manifest_path": str(root / "Cargo.toml"), "source": None},
                    {
                        "id": "ejemplo-crate 1.0.0",
                        "name": "ejemplo-crate",
                        "version": "1.0.0",
                        "license": "MIT",
                        "manifest_path": str(crate / "Cargo.toml"),
                        "source": "registry+https://example.test",
                    },
                ],
                "workspace_members": [app["id"]],
                "resolve": {
                    "nodes": [
                        {
                            "id": app["id"],
                            "deps": [
                                {
                                    "pkg": "ejemplo-crate 1.0.0",
                                    "dep_kinds": [{"kind": None, "target": None}],
                                }
                            ],
                        },
                        {"id": "ejemplo-crate 1.0.0", "deps": []},
                    ]
                },
            }
        )
    )
    return pnpm, cargo


def answering(tool: str, pnpm: Path, cargo: Path) -> str:
    """Shell lines that make a double of `tool` print its listing when asked."""
    if tool == "pnpm":
        return f'case "$*" in *"licenses list"*) cat "{pnpm}";; esac\n'
    if tool == "cargo":
        return f'case "$1" in metadata) cat "{cargo}";; esac\n'
    raise ValueError(tool)
