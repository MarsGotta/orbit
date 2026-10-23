"""Reads Rust and Swift source as text, without a compiler.

The gate's size limits and domain boundaries only need three things from a
source file: which lines hold code, where each function starts and ends, and
which items are tests. Both languages put comments and string literals around
code that can contain anything, braces included, so everything here works on a
*masked* copy of the source: the same length and the same newlines, with every
comment turned into spaces and every character inside a string or character
literal turned into `S`. On that copy a brace is a brace.

A line holds code when its masked text is not blank: comments and blank lines
do not count, the lines of a multi-line string do.
"""

import re
from dataclasses import dataclass, field

IDENTIFIER = re.compile(r"[A-Za-z_][A-Za-z0-9_]*")
RUST_RAW_STRING = re.compile(r'(?:br|cr|r)(#*)"')


def _blank(out: list[str], start: int, end: int, fill: str) -> None:
    for index in range(start, end):
        if out[index] != "\n":
            out[index] = fill


def _block_comment_end(src: str, start: int) -> int:
    """The end of the block comment that opens at `start`; both languages nest them."""
    depth, index = 1, start + 2
    while index < len(src) and depth:
        if src.startswith("/*", index):
            depth, index = depth + 1, index + 2
        elif src.startswith("*/", index):
            depth, index = depth - 1, index + 2
        else:
            index += 1
    return index


def _starts_word(src: str, index: int) -> bool:
    return index == 0 or not (src[index - 1].isalnum() or src[index - 1] == "_")


def _rust_quote_end(src: str, index: int) -> int | None:
    """The end of a char literal at `index`, or None for a lifetime or a label."""
    if src.startswith("\\", index + 1):
        close = src.find("'", index + 3)
        return None if close < 0 else close + 1
    if index + 2 < len(src) and src[index + 2] == "'" and src[index + 1] != "\n":
        return index + 3
    return None


def mask_rust(src: str) -> str:
    out = list(src)
    index, size = 0, len(src)
    while index < size:
        char = src[index]
        if src.startswith("//", index):
            end = src.find("\n", index)
            end = size if end < 0 else end
            _blank(out, index, end, " ")
            index = end
            continue
        if src.startswith("/*", index):
            end = _block_comment_end(src, index)
            _blank(out, index, end, " ")
            index = end
            continue
        raw = RUST_RAW_STRING.match(src, index) if _starts_word(src, index) else None
        if raw:
            close = '"' + raw.group(1)
            end = src.find(close, raw.end())
            end = size if end < 0 else end + len(close)
            _blank(out, raw.end(), end - len(close), "S")
            index = end
            continue
        prefixed = char in "bc" and src.startswith('"', index + 1) and _starts_word(src, index)
        if char == '"' or prefixed:
            start = index + (2 if prefixed else 1)
            end = start
            while end < size and src[end] != '"':
                end += 2 if src[end] == "\\" else 1
            _blank(out, start, min(end, size), "S")
            index = end + 1
            continue
        if char == "'":
            end = _rust_quote_end(src, index)
            if end is not None:
                _blank(out, index + 1, end - 1, "S")
                index = end
                continue
        index += 1
    return "".join(out)


def _swift_string_end(src: str, index: int, hashes: str, triple: bool) -> int:
    """The offset right after the string that opens at `index` (its quotes included)."""
    quote = '"""' if triple else '"'
    position = index + len(hashes) + len(quote)
    escape = "\\" + hashes
    while position < len(src):
        if not triple and src[position] == "\n":
            return position
        if src.startswith(escape + "(", position):
            position = _swift_interpolation_end(src, position + len(escape) + 1)
            continue
        if src.startswith(escape, position):
            position += len(escape) + 1
            continue
        if src.startswith(quote + hashes, position):
            return position + len(quote) + len(hashes)
        position += 1
    return position


def _swift_interpolation_end(src: str, position: int) -> int:
    depth = 1
    while position < len(src) and depth:
        if src[position] == '"':
            position = _swift_string_end(src, position, "", src.startswith('"""', position))
            continue
        depth += {"(": 1, ")": -1}.get(src[position], 0)
        position += 1
    return position


def mask_swift(src: str) -> str:
    out = list(src)
    index, size = 0, len(src)
    while index < size:
        if src.startswith("//", index):
            end = src.find("\n", index)
            end = size if end < 0 else end
            _blank(out, index, end, " ")
            index = end
            continue
        if src.startswith("/*", index):
            end = _block_comment_end(src, index)
            _blank(out, index, end, " ")
            index = end
            continue
        opening = re.compile(r'(#*)("""|")').match(src, index)
        if opening and (opening.group(1) or src[index] == '"'):
            hashes, triple = opening.group(1), opening.group(2) == '"""'
            end = _swift_string_end(src, index, hashes, triple)
            _blank(out, opening.end(), max(opening.end(), end - len(opening.group(0))), "S")
            index = end
            continue
        index += 1
    return "".join(out)


def line_starts(text: str) -> list[int]:
    starts = [0]
    starts.extend(index + 1 for index, char in enumerate(text) if char == "\n")
    return starts


def line_of(starts: list[int], offset: int) -> int:
    """The 1-based line that holds `offset`."""
    low, high = 0, len(starts) - 1
    while low < high:
        middle = (low + high + 1) // 2
        if starts[middle] <= offset:
            low = middle
        else:
            high = middle - 1
    return low + 1


def code_lines(masked: str) -> list[bool]:
    """`result[n]` says whether line n (1-based) holds code; `result[0]` is unused."""
    return [False] + [bool(line.strip()) for line in masked.split("\n")]


@dataclass
class Scope:
    """A pair of braces and what opened it: a function, a type, a module or a block."""

    kind: str
    name: str
    header_start: int
    open: int
    close: int
    parent: "Scope | None" = None
    children: list["Scope"] = field(default_factory=list)


def brace_pairs(masked: str) -> dict[int, int]:
    """Each `{` offset and the offset of the `}` that closes it (the end of text if none)."""
    pairs, stack = {}, []
    for index, char in enumerate(masked):
        if char == "{":
            stack.append(index)
        elif char == "}" and stack:
            pairs[stack.pop()] = index
    for opening in stack:
        pairs[opening] = len(masked) - 1
    return pairs


def header_start(masked: str, brace: int) -> int:
    """Where the text that introduces the `{` at `brace` starts.

    Walking back, the header ends at the item before it: a `;`, `{` or `}` at
    the same depth, or a `(` or `[` that encloses the brace (a closure passed
    as an argument). Brackets inside the header, like `[u8; 4]`, are skipped.
    """
    depth = 0
    index = brace - 1
    while index >= 0:
        char = masked[index]
        if char in ")]":
            depth += 1
        elif char in "([":
            if depth == 0:
                return index + 1
            depth -= 1
        elif char in ";{}" and depth == 0:
            return index + 1
        index -= 1
    return 0


def scopes(masked: str, classify) -> list[Scope]:
    """Every brace pair, classified by `classify(header) -> (kind, name)`, nested."""
    pairs = brace_pairs(masked)
    found: list[Scope] = []
    stack: list[Scope] = []
    for opening in sorted(pairs):
        while stack and stack[-1].close < opening:
            stack.pop()
        start = header_start(masked, opening)
        kind, name = classify(masked[start:opening])
        parent = stack[-1] if stack else None
        scope = Scope(kind, name, start, opening, pairs[opening], parent)
        if parent:
            parent.children.append(scope)
        found.append(scope)
        stack.append(scope)
    return found


RUST_ATTRIBUTE = re.compile(r"#!?\[[^\]]*(?:\[[^\]]*\][^\]]*)*\]")
RUST_FN = re.compile(r"\bfn\s+([A-Za-z_][A-Za-z0-9_]*)")
RUST_IMPL = re.compile(r"^\s*(?:unsafe\s+)?impl\b")
RUST_MOD = re.compile(r"^\s*(?:pub(?:\([^)]*\))?\s+)?mod\s+([A-Za-z_][A-Za-z0-9_]*)\s*$")
RUST_TRAIT = re.compile(r"^\s*(?:pub(?:\([^)]*\))?\s+)?(?:unsafe\s+)?trait\s+([A-Za-z_]\w*)")
RUST_ITEM_PREFIX = re.compile(
    r"^\s*(?:pub(?:\([^)]*\))?\s+)?(?:(?:const|async|unsafe|extern(?:\s+\"S*\")?)\s+)*fn\b"
)


def _last_type_name(text: str) -> str:
    """`orbit_core::State<T>` -> `State`: the last path segment, without generics."""
    depth, kept = 0, []
    for char in text:
        if char == "<":
            depth += 1
        elif char == ">":
            depth -= 1
        elif depth == 0:
            kept.append(char)
    names = IDENTIFIER.findall("".join(kept))
    return names[-1] if names else "?"


def _rust_impl_name(header: str) -> str:
    body = re.sub(r"^\s*(?:unsafe\s+)?impl\s*", "", header)
    if body.startswith("<"):
        depth = 0
        for index, char in enumerate(body):
            depth += {"<": 1, ">": -1}.get(char, 0)
            if depth == 0:
                body = body[index + 1 :]
                break
    body = re.split(r"\bwhere\b", body)[0]
    if re.search(r"\bfor\b", body):
        trait, target = re.split(r"\bfor\b", body, maxsplit=1)
        return f"<{_last_type_name(target)} as {_last_type_name(trait)}>"
    return _last_type_name(body)


def classify_rust(header: str) -> tuple[str, str]:
    """What a Rust `{` opens, from the text before it (attributes removed)."""
    text = RUST_ATTRIBUTE.sub(" ", header)
    function = RUST_FN.search(text)
    if function and RUST_ITEM_PREFIX.match(text[: function.end()]):
        return "function", function.group(1)
    if RUST_IMPL.match(text):
        return "impl", _rust_impl_name(text)
    module = RUST_MOD.match(text)
    if module:
        return "module", module.group(1)
    trait = RUST_TRAIT.match(text)
    if trait:
        return "trait", trait.group(1)
    return "block", ""


SWIFT_TYPE = re.compile(
    r"\b(?:class|struct|enum|extension|actor|protocol)\s+([A-Za-z_][A-Za-z0-9_.]*)"
)
SWIFT_FUNCTION = re.compile(
    r"(?<![.\w])(?:func\s+([A-Za-z_][A-Za-z0-9_]*|[^\s(<]+)|(init)[?!]?|(deinit))(?=\s*[(<{]|\s*$)"
)


def classify_swift(header: str) -> tuple[str, str]:
    """What a Swift `{` opens. The last declaration keyword in the header wins:
    a protocol requirement without a body can precede a method in one header."""
    functions = list(SWIFT_FUNCTION.finditer(header))
    types = list(SWIFT_TYPE.finditer(header))
    last_function = functions[-1] if functions else None
    last_type = types[-1] if types else None
    if last_function and (not last_type or last_function.start() > last_type.start()):
        return "function", next(group for group in last_function.groups() if group)
    if last_type:
        return "type", last_type.group(1).split(".")[-1]
    return "block", ""


def qualified_name(scope: Scope, separator: str) -> str:
    """The function's name after every enclosing function, type, impl or module."""
    parts = [scope.name]
    parent = scope.parent
    while parent:
        if parent.kind != "block":
            parts.append(parent.name)
        parent = parent.parent
    return separator.join(reversed(parts))


def number_duplicates(names: list[str]) -> list[str]:
    """The second `name` of a file becomes `name#2`, the third `name#3`."""
    seen: dict[str, int] = {}
    numbered = []
    for name in names:
        seen[name] = seen.get(name, 0) + 1
        numbered.append(name if seen[name] == 1 else f"{name}#{seen[name]}")
    return numbered
