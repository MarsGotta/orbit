"""Which Rust files are production code, and which of their items are tests.

A file is production when the compiler builds it outside tests: it is a
crate's root (`src/lib.rs`, `src/main.rs`, `src/bin/*.rs`, `build.rs`) or a
module that a production file declares with `mod name;` outside `#[cfg(test)]`.
The module tree is followed the way rustc does, `#[path = "…"]` included, so a
file is a test because of how it is declared, not because of its name.
Integration tests (`tests/`), examples and benches are no crate's root here and
never production.

Inside a production file, an item under `#[cfg(test)]`, `#[test]` or
`#[tokio::test]` is test code: its lines and its functions do not count.
"""

import re
from dataclasses import dataclass
from pathlib import Path

from code_scan import Scope, classify_rust, header_start, mask_rust, scopes

TEST_ATTRIBUTE = re.compile(r"#\[\s*(?:cfg\s*\(\s*test\s*\)|test|tokio\s*::\s*test\b[^\]]*)\s*\]")
MOD_DECLARATION = re.compile(r"\bmod\s+([A-Za-z_][A-Za-z0-9_]*)\s*;")
PATH_ATTRIBUTE = re.compile(r'#\[\s*path\s*=\s*"(S+)"\s*\]')


@dataclass
class RustFile:
    path: Path
    crate: Path
    module: tuple[str, ...]
    test: bool
    source: str
    masked: str
    test_ranges: list[tuple[int, int]]
    scopes: list[Scope]

    def in_test(self, offset: int) -> bool:
        return any(start <= offset <= end for start, end in self.test_ranges)


def item_end(masked: str, start: int, pairs: dict[int, int]) -> int:
    """The offset where the item that starts at `start` ends: its `;` or its `}`."""
    depth, index = 0, start
    while index < len(masked):
        char = masked[index]
        if char in "([":
            depth += 1
        elif char in ")]":
            depth -= 1
        elif char == ";" and depth == 0:
            return index
        elif char == "{" and depth == 0:
            return pairs.get(index, len(masked) - 1)
        elif char == "}" and depth == 0:
            return index - 1
        index += 1
    return len(masked) - 1


def test_ranges(masked: str, found: list[Scope]) -> list[tuple[int, int]]:
    """The spans, attribute to end, of every item marked as a test."""
    pairs = {scope.open: scope.close for scope in found}
    ranges: list[tuple[int, int]] = []
    for attribute in TEST_ATTRIBUTE.finditer(masked):
        if ranges and attribute.start() <= ranges[-1][1]:
            continue
        ranges.append((attribute.start(), item_end(masked, attribute.end(), pairs)))
    return ranges


def read_rust(path: Path, crate: Path, module: tuple[str, ...], test: bool) -> RustFile:
    source = path.read_text(encoding="utf-8")
    masked = mask_rust(source)
    found = scopes(masked, classify_rust)
    return RustFile(path, crate, module, test, source, masked, test_ranges(masked, found), found)


def _inline_modules(file: RustFile, offset: int) -> list[str]:
    names = []
    for scope in file.scopes:
        if scope.kind == "module" and scope.open < offset < scope.close:
            names.append(scope.name)
    return names


def _path_attribute(file: RustFile, offset: int) -> str | None:
    """The `#[path = "…"]` among the attributes of the declaration at `offset`."""
    start = header_start(file.masked, offset)
    found = list(PATH_ATTRIBUTE.finditer(file.masked, start, offset))
    if not found:
        return None
    value = found[-1].span(1)
    return file.source[value[0] : value[1]]


def _child_path(file: RustFile, owns_directory: bool, name: str, offset: int) -> Path:
    inline = _inline_modules(file, offset)
    directory = file.path.parent if owns_directory else file.path.with_suffix("")
    attribute = _path_attribute(file, offset)
    if attribute is not None:
        base = file.path.parent if not inline else directory.joinpath(*inline)
        return (base / attribute).resolve()
    base = directory.joinpath(*inline)
    flat = base / f"{name}.rs"
    return flat if flat.exists() else base / name / "mod.rs"


def crate_roots(crate: Path) -> list[Path]:
    roots = [crate / "src" / "lib.rs", crate / "src" / "main.rs", crate / "build.rs"]
    roots.extend(sorted((crate / "src" / "bin").glob("*.rs")))
    return [root for root in roots if root.is_file()]


def crate_files(crate: Path) -> dict[Path, RustFile]:
    """Every file the crate's roots reach, each marked test or production.

    A file reached both ways is production. A declared module whose file is
    missing raises, because the scan would otherwise be silently incomplete.
    """
    files: dict[Path, RustFile] = {}
    pending = [(root.resolve(), (), False, True) for root in crate_roots(crate)]
    while pending:
        path, module, test, owns_directory = pending.pop()
        known = files.get(path)
        if known and (test or not known.test):
            continue
        file = read_rust(path, crate, module, test)
        files[path] = file
        for declaration in MOD_DECLARATION.finditer(file.masked):
            name = declaration.group(1)
            child = _child_path(file, owns_directory, name, declaration.start())
            if not child.is_file():
                raise FileNotFoundError(f"{path}: `mod {name};` names no file ({child})")
            child_owns = (
                child.name == "mod.rs" or _path_attribute(file, declaration.start()) is not None
            )
            inline = tuple(_inline_modules(file, declaration.start()))
            child_test = test or file.in_test(declaration.start())
            pending.append((child.resolve(), module + inline + (name,), child_test, child_owns))
    return files


def rust_crates(root: Path, folders: list[str]) -> list[Path]:
    """The crate directories (with a Cargo.toml) under each folder, or the folder itself."""
    crates = []
    for folder in folders:
        base = root / folder
        if (base / "Cargo.toml").is_file():
            crates.append(base)
        else:
            crates.extend(sorted(path.parent for path in base.glob("*/Cargo.toml")))
    return crates
