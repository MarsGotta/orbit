"""Portable vault paths: never absolute, hidden, system or outside the root."""

import posixpath
import re
from urllib.parse import unquote, urlsplit

from .method import file_name
from .models import refusal

SYSTEM_NAMES = frozenset({"__MACOSX", "Thumbs.db", "desktop.ini"})


def visible(path):
    return all(
        part and not part.startswith(".") and part not in SYSTEM_NAMES for part in path.split("/")
    )


def checked(path, *, folder=False):
    if folder and not path:
        return ""
    if (
        not path
        or path.startswith("/")
        or "\\" in path
        or ":" in path
        or any(ord(c) < 32 for c in path)
        or not visible(path)
    ):
        raise refusal(
            "vault_path_refused",
            "The path must stay inside the vault, without hidden files or symbolic links.",
        )
    if not folder and not path.lower().endswith(".md"):
        raise refusal("vault_not_markdown", "Only Markdown notes (.md) can be read.")
    return path


def resolve(source, target, paths, *, wiki=False, basenames=None):
    """Only unique existing Markdown targets; external/ambiguous links stay unresolved."""
    target = unquote(target.split("|", 1)[0].strip())
    try:
        if urlsplit(target).scheme or target.startswith(("//", "\\")):
            return None
    except ValueError:
        return None
    target = re.split(r"[#^?]", target, maxsplit=1)[0]
    if not target:
        return source
    absolute = target.startswith("/")
    target = target.lstrip("/")
    if not target.lower().endswith(".md"):
        target += ".md"
    candidates = [
        posixpath.normpath(
            target if absolute else posixpath.join(posixpath.dirname(source), target)
        )
    ]
    if wiki:
        candidates.insert(0, posixpath.normpath(target))
    for candidate in candidates:
        if visible(candidate) and candidate in paths:
            return candidate
    if wiki and "/" not in target:
        matches = (
            (basenames or {}).get(file_name(target[:-3]), [])
            if basenames is not None
            else [
                p for p in paths if file_name(p.rsplit("/", 1)[-1][:-3]) == file_name(target[:-3])
            ]
        )
        if len(matches) == 1:
            return matches[0]
    return None
