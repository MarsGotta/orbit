#!/usr/bin/env python3
"""Write the notice of the software by others that Orbit.app ships.

Read at build time from what the build itself installed, offline: the
interface's packages as `pnpm licenses list --prod --json` lists them (what
pnpm installed for production, which includes everything Vite bundles), and
the crates compiled into the app as `cargo metadata --filter-platform` resolves
them, following only normal dependencies from the app's crate. Orbit's own
crates are under Orbit's LICENSE, which ships beside this notice; a crate that
only tests or build scripts use is not inside the app.

Each package is named with its version and the licence it declares, then every
licence text those packages carry is written once, with the packages that carry
it. No path of the build machine is written. The cua-driver engine keeps its own
licence (third-party/cua-driver/LICENSE.md).

Usage: third-party-notices.py --pnpm-licenses PNPM.json --cargo-metadata CARGO.json \
    --root orbit-desktop --output NOTICES.txt

Runs on the build Mac's /usr/bin/python3 (3.9): standard library only.
"""

import argparse
import json
import re
import sys
from pathlib import Path

# The first line, which the archive verifier checks
# (scripts/release/verify-macos-archive.py, NOTICES_HEADER).
HEADER = "Third-party software in Orbit\n"

# The files a package ships its licence in: LICENSE, LICENSE-MIT, LICENCE.md,
# COPYING, NOTICE, UNLICENSE, COPYRIGHT, in any case.
LICENCE_FILE = re.compile(r"^(licen[cs]e|copying|notice|unlicense|copyright)", re.IGNORECASE)


class NoticeError(ValueError):
    """The listings cannot give a complete notice."""


def read_listing(path, tool):
    try:
        data = json.loads(Path(path).read_text(encoding="utf-8"))
    except (OSError, ValueError) as error:
        raise NoticeError(f"{tool} did not leave a readable JSON listing in {path}: {error}")
    if not isinstance(data, dict):
        raise NoticeError(f"{tool} left a listing that is not a JSON object")
    return data


def licence_texts(directory, extra=None):
    """The licence files directly in a package's directory, by name."""
    names = sorted(
        entry.name
        for entry in directory.iterdir()
        if entry.is_file() and LICENCE_FILE.match(entry.name)
    )
    if extra and extra not in names and (directory / extra).is_file():
        names.append(extra)
    texts = []
    for name in names:
        text = (directory / name).read_bytes().decode("utf-8", errors="replace").strip()
        if text:
            texts.append(text)
    return texts


def interface_packages(listing):
    """(name, version, licence, directory) of every package pnpm listed."""
    packages = []
    for licence, entries in listing.items():
        for entry in entries:
            name = entry.get("name")
            versions = entry.get("versions") or []
            paths = entry.get("paths") or []
            if not name or len(versions) != len(paths):
                raise NoticeError(f"pnpm listed {name or 'a package'} without one path per version")
            for version, path in zip(versions, paths):
                packages.append((name, version, entry.get("license") or licence, Path(path)))
    if not packages:
        raise NoticeError("pnpm licenses listed no package: the interface's notice would be empty")
    return packages


def compiled_crates(metadata, root):
    """(name, version, licence, directory, licence file) of every crate the app links."""
    try:
        packages = {package["id"]: package for package in metadata["packages"]}
        nodes = {node["id"]: node for node in metadata["resolve"]["nodes"]}
        ours = set(metadata["workspace_members"])
    except (KeyError, TypeError):
        raise NoticeError("cargo metadata left no packages, workspace or resolved graph")
    roots = [identifier for identifier, package in packages.items() if package["name"] == root]
    if len(roots) != 1:
        raise NoticeError(f"cargo metadata has {len(roots)} packages named {root}, not one")
    linked, pending = set(), [roots[0]]
    while pending:
        identifier = pending.pop()
        if identifier in linked:
            continue
        linked.add(identifier)
        for dependency in nodes.get(identifier, {}).get("deps", []):
            # Only normal dependencies end up in the binary; `dev` and `build`
            # ones run on the build machine.
            if any(kind.get("kind") is None for kind in dependency.get("dep_kinds", [])):
                pending.append(dependency["pkg"])
    crates = []
    for identifier in sorted(linked - ours):
        package = packages[identifier]
        crates.append(
            (
                package["name"],
                package["version"],
                package.get("license") or "",
                Path(package["manifest_path"]).parent,
                package.get("license_file"),
            )
        )
    if not crates:
        raise NoticeError(f"cargo metadata links no crate of others into {root}")
    return crates


def render(interface, crates):
    """The notice: the packages, then each licence text once."""
    texts = {}
    order = []

    def cite(name, version, directory, extra=None):
        if not directory.is_dir():
            raise NoticeError(f"{name} {version} is not where its listing says it is")
        numbers = []
        for text in licence_texts(directory, extra):
            if text not in texts:
                texts[text] = (len(order) + 1, [])
                order.append(text)
            number, carriers = texts[text]
            if f"{name} {version}" not in carriers:
                carriers.append(f"{name} {version}")
            numbers.append(number)
        return numbers

    def line(name, version, licence, numbers):
        declared = licence or "no licence declared"
        if not numbers:
            return f"{name} {version} · {declared} · its package carries no licence file"
        return f"{name} {version} · {declared} · text {', '.join(str(n) for n in numbers)}"

    sections = []
    rows = []
    for name, version, licence, directory in sorted(interface, key=lambda p: (p[0], p[1])):
        rows.append(line(name, version, licence, cite(name, version, directory)))
    sections.append(("The interface: packages pnpm installed for it", rows))
    rows = []
    for name, version, licence, directory, extra in sorted(crates, key=lambda p: (p[0], p[1])):
        rows.append(line(name, version, licence, cite(name, version, directory, extra)))
    sections.append(("The app: Rust crates compiled into it", rows))
    out = [
        HEADER,
        "=" * (len(HEADER) - 1) + "\n\n",
        "Orbit.app contains software written by others. Each package is listed\n"
        "with the version built into this copy and the licence it declares; the\n"
        "licence texts they carry follow, each once, numbered.\n\n",
    ]
    for title, rows in sections:
        out.append(f"{title}\n{'-' * len(title)}\n")
        out.extend(f"{row}\n" for row in rows)
        out.append("\n")
    title = "Licence texts"
    out.append(f"{title}\n{'-' * len(title)}\n")
    for text in order:
        number, carriers = texts[text]
        out.append(f"\n[{number}] Carried by {', '.join(carriers)}\n\n{text}\n")
    return "".join(out)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--pnpm-licenses", required=True)
    parser.add_argument("--cargo-metadata", required=True)
    parser.add_argument("--root", required=True)
    parser.add_argument("--output", required=True)
    arguments = parser.parse_args(argv)
    try:
        interface = interface_packages(read_listing(arguments.pnpm_licenses, "pnpm licenses"))
        crates = compiled_crates(
            read_listing(arguments.cargo_metadata, "cargo metadata"), arguments.root
        )
        notice = render(interface, crates)
    except NoticeError as error:
        print(f"The third-party notice was not written: {error}", file=sys.stderr)
        return 1
    Path(arguments.output).write_text(notice, encoding="utf-8")
    print(
        f"Third-party notice: {len(interface)} interface packages and {len(crates)} crates, "
        f"{len(notice.encode('utf-8'))} B"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
