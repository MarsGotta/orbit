#!/usr/bin/env python3
"""The generated TypeScript does not change when the tests run (REF-02 A1).

ts-rs writes `apps/desktop/src/bridge/generated/` as a side effect of
`cargo test`. A snapshot taken right before the tests and compared right after
says whether the files in the tree are the ones the Rust types produce; if
they are not, the interface was type-checked against stale types.

Usage:
  generated.py snapshot DIR FILE   records the digest of every file under DIR
  generated.py compare DIR FILE    names every file changed, added or removed since
"""

import hashlib
import json
import sys
from pathlib import Path


def digests(directory: Path) -> dict[str, str]:
    return {
        str(path.relative_to(directory)): hashlib.sha256(path.read_bytes()).hexdigest()
        for path in sorted(directory.rglob("*"))
        if path.is_file()
    }


def main(arguments: list[str]) -> int:
    if len(arguments) != 3 or arguments[0] not in ("snapshot", "compare"):
        print(__doc__, file=sys.stderr)
        return 2
    action, directory, snapshot = arguments[0], Path(arguments[1]), Path(arguments[2])
    if not directory.is_dir():
        print(f"no existe el directorio de tipos generados {directory}")
        return 1
    if action == "snapshot":
        snapshot.write_text(json.dumps(digests(directory), indent=1), encoding="utf-8")
        return 0
    if not snapshot.is_file():
        print(
            f"no hay instantánea previa {snapshot}; no se puede saber si los tests cambiaron {directory}"
        )
        return 1
    before = json.loads(snapshot.read_text(encoding="utf-8"))
    after = digests(directory)
    changed = sorted(name for name in before.keys() & after.keys() if before[name] != after[name])
    added = sorted(after.keys() - before.keys())
    removed = sorted(before.keys() - after.keys())
    if not (changed or added or removed):
        print(f"{directory}: {len(after)} ficheros, ninguno cambió al ejecutar los tests")
        return 0
    for label, names in (("cambiado", changed), ("nuevo", added), ("borrado", removed)):
        for name in names:
            print(f"  {label}: {directory / name}")
    print(
        f"cargo test cambió {len(changed)}, añadió {len(added)} y borró {len(removed)} ficheros "
        f"de {directory}: el TypeScript del árbol no es el que generan los tipos de Rust. "
        f"Revisa `git diff -- {directory}` e incluye los ficheros generados en el cambio."
    )
    return 1


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
