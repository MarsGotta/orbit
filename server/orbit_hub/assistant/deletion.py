"""Whether what an agent asks to run deletes files (hub-v1 «Choices and
questions»).

A command that deletes is approved one at a time: the runtime offers only
`once` and `deny` for it, whatever the agent offered, and refuses a wider
answer, so a broad permission never deletes more than the one command the
person read. This reads the command's words, as a shell would split them:
`rm`, `unlink`, `rmdir`, `shred`, `trash`, `git rm`, `git clean` and `find
… -delete` (or `-exec rm`), also behind `sudo`, `xargs`, `env`, a variable
assignment or `sh -c`, in any part of a `&&`, `;` or `|` chain. A program
that deletes through its own code (a script, `python -c`) is not seen. It is
not a sandbox: it narrows the choices of a request the agent already made.
"""

import os
import re
import shlex
from dataclasses import dataclass

DELETERS = frozenset({"rm", "unlink", "rmdir", "shred", "srm", "trash"})
# What runs the program named after it, with its own flags first.
WRAPPERS = frozenset({"sudo", "doas", "command", "builtin", "exec", "nohup", "time", "env", "nice"})
SHELLS = frozenset({"sh", "bash", "zsh", "dash", "fish"})
OPERATORS = frozenset({"&&", "||", ";", "|", "&", "(", ")", ";;", "|&"})
# Words the shell expands into other names: what they delete is not known.
EXPANDS = frozenset("*?[]{}$`")
# `sh -c 'sh -c …'` past this depth is read as deleting: it protects the
# reader from a crafted command, not any real one.
MAX_DEPTH = 4


@dataclass(frozen=True)
class Deletion:
    # How many files the command names when it is one `rm` or `unlink` of
    # plain names, neither recursive nor by pattern; None when not known.
    paths: int | None = None

    def public(self) -> dict:
        return {} if self.paths is None else {"paths": self.paths}


def _words(command: str) -> tuple[list[str], bool]:
    """Its words, and whether they are the shell's own split."""
    # A newline ends a command as `;` does.
    text = command.replace("\n", " ; ")
    lexer = shlex.shlex(text, posix=True, punctuation_chars=True)
    lexer.whitespace_split = True
    try:
        return list(lexer), True
    except ValueError:
        # Unbalanced quotes: the shell would refuse it, but its words still say what it means.
        return text.split(), False


def _segments(words: list[str]) -> list[list[str]]:
    segments, current = [], []
    for word in words:
        if word in OPERATORS:
            if current:
                segments.append(current)
            current = []
        else:
            current.append(word)
    if current:
        segments.append(current)
    return segments


def _program(segment: list[str]) -> list[str]:
    """The segment from the program it runs: assignments and wrappers dropped."""
    index = 0
    while index < len(segment):
        word = segment[index]
        name = os.path.basename(word)
        if "=" in word and not word.startswith("-") and name == word:
            index += 1
        elif name in WRAPPERS or name == "xargs":
            index += 1
            while index < len(segment) and segment[index].startswith("-"):
                index += 1
        else:
            break
    return segment[index:]


def _deletes(segment: list[str], depth: int) -> bool:
    words = _program(segment)
    if not words:
        return False
    name, args = os.path.basename(words[0]), words[1:]
    if name in DELETERS:
        return True
    if name == "git":
        verbs = [arg for arg in args if not arg.startswith("-")]
        return bool(verbs) and verbs[0] in {"rm", "clean"}
    if name == "find":
        if "-delete" in args:
            return True
        return any(
            arg in {"-exec", "-execdir", "-ok", "-okdir"}
            and at + 1 < len(args)
            and os.path.basename(args[at + 1]) in DELETERS
            for at, arg in enumerate(args)
        )
    if name in SHELLS:
        # `-c` alone or among other short flags (`bash -lc`, as codex-acp runs).
        at = next(
            (at for at, arg in enumerate(args) if re.fullmatch(r"-[A-Za-z]*c[A-Za-z]*", arg)),
            None,
        )
        if at is not None and at + 1 < len(args):
            return depth >= MAX_DEPTH or _any_deletes(args[at + 1], depth + 1)
    return False


def _any_deletes(command: str, depth: int) -> bool:
    return any(_deletes(segment, depth) for segment in _segments(_words(command)[0]))


def _named_files(segments: list[list[str]]) -> int | None:
    """The files one plain `rm`/`unlink` names; None for anything else."""
    if len(segments) != 1:
        return None
    words = _program(segments[0])
    if not words or os.path.basename(words[0]) not in {"rm", "unlink"}:
        return None
    operands, flags_done = [], False
    for word in words[1:]:
        if not flags_done and word == "--":
            flags_done = True
        elif not flags_done and word.startswith("-") and word != "-":
            if word == "--recursive" or (not word.startswith("--") and set(word) & {"r", "R"}):
                return None
        else:
            operands.append(word)
    if not operands or any(set(word) & EXPANDS for word in operands):
        return None
    return len(operands)


def command_deletion(command: str) -> Deletion | None:
    """What a command line deletes, or None when it deletes nothing seen."""
    words, exact = _words(command)
    segments = _segments(words)
    if not any(_deletes(segment, 0) for segment in segments):
        return None
    return Deletion(paths=_named_files(segments) if exact else None)


def one_at_a_time(choices: tuple[str, ...] | None) -> tuple[str, ...]:
    """The choices left for a request that deletes: `once` and `deny` of
    those offered (both when the agent named none)."""
    offered = ("once", "deny") if choices is None else choices
    return tuple(choice for choice in offered if choice in {"once", "deny"})
