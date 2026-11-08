#!/usr/bin/env python3
"""Each package uses only the packages its layer allows, the engine stays free
of what belongs to a client, and the coordinator `Runtime` only shrinks.

layers.toml, next to this guard, holds its rules. Like boundaries.toml and
size-ratchet.toml it is exact: what is new fails, and an entry the code no
longer needs fails too, so every list only shortens.

  packages  every workspace package and the workspace packages it uses as a
            normal dependency (dev and build dependencies are not part of
            what ships, so they do not count; optional and platform-only ones
            do). Read with `cargo metadata --no-deps --offline`, which builds
            nothing.
  optional  the dependencies that must stay optional: the engine only takes
            them when the Mac turns their feature on. Each is also in
            [packages]. The table may be missing.
  engine    the engine packages, and for each forbidden use (launching a
            process, branching on an Apple platform, the keychain crate,
            AppKit, Unix signals) the files that may still have it. Production
            code only, on the masked source (code_scan.py): comments and
            string contents do not count. Also the crates no engine package
            may depend on, under any name and of any kind of dependency.
  runtime   how many methods `impl Runtime` has in the coordinator's package,
            and how many of them live in a domain folder (`daemon::<d>`,
            where `<d>` owns a `…Handle` field of daemon/runtime.rs, the
            reading boundaries.py makes). Both numbers may only go down, and
            when they do the guard asks to lower them in layers.toml.

Exit status: 0 when the code matches layers.toml, 1 when it differs, 2 when
it cannot check (an extra argument, no layers.toml or one of the wrong shape,
cargo metadata failing).

Usage: layers.py [ROOT]    ROOT defaults to the repository
"""

import json
import re
import subprocess
import sys
import tomllib
from dataclasses import dataclass, field
from pathlib import Path

sys.dont_write_bytecode = True
sys.path.insert(0, str(Path(__file__).resolve().parent))

from code_scan import line_of, line_starts
from rust_modules import RustFile, crate_files

REPOSITORY = Path(__file__).resolve().parents[2]
CONFIG = Path("scripts") / "check" / "layers.toml"
ENGINE_KEYS = ("command_new", "cfg_macos", "keyring", "appkit", "unix_signals")
FORBIDDEN_CRATES_KEY = "forbidden_crates"
FORBIDDEN = {
    "command_new": re.compile(
        r"\bCommand\s*::\s*new\b"
        r"|\bprocess\s*::\s*Command\b"
        r"|\bprocess\s*::\s*\{[^}]*\bCommand\b"
        r"|\bprocess\s*::\s*\*"
    ),
    "cfg_macos": re.compile(r"\btarget_(?:os|vendor)\s*=\s*\""),
    "keyring": re.compile(r"\bkeyring\b"),
    "appkit": re.compile(r"\b(AppKit|objc2_app_kit|cocoa)\b"),
    "unix_signals": re.compile(r"\bsignal\s*::\s*(?:unix\b|\*|ctrl_c\b)|\bSignalKind\b"),
}
APPLE_VALUE = re.compile(r"target_(?:os\s*=\s*\"(?:macos|ios)\"|vendor\s*=\s*\"apple\")")
HANDLE_FIELD = re.compile(r"\b(\w+)\s*:\s*(\w+)\s*::\s*(\w+Handle)\b")
COORDINATOR = "Runtime"
DOMAIN_PARENT = "daemon"


class CannotCheck(Exception):
    """The guard cannot compare the code with layers.toml (exit 2)."""


@dataclass
class Workspace:
    """The workspace packages, their directories and their dependencies."""

    directories: dict[str, Path]
    # Normal workspace dependencies.
    uses: dict[str, set[str]]
    # The same, without the optional ones.
    required: dict[str, set[str]]
    # Every dependency of every kind, workspace or not: (crate, the name it is used under).
    crates: dict[str, set[tuple[str, str]]]


@dataclass
class Report:
    problems: list[str] = field(default_factory=list)
    numbers: dict[str, int] = field(default_factory=dict)

    def add(self, cause: str, text: str) -> None:
        self.problems.append(f"{cause}: {text}")


def is_text_list(value: object) -> bool:
    return isinstance(value, list) and all(isinstance(item, str) for item in value)


def is_count(value: object) -> bool:
    # TOML's true and false are Python's bool, which is an int.
    return isinstance(value, int) and not isinstance(value, bool)


def shape_problems(rules: dict) -> list[str]:
    """What in layers.toml has the wrong type. A string where a list goes would
    otherwise be read letter by letter, and pass."""
    problems = [
        f"[{table}] must be a table, and it is a {type(rules[table]).__name__}"
        for table in ("packages", "optional", "engine", "runtime")
        if table in rules and not isinstance(rules[table], dict)
    ]
    if problems:
        return problems
    wrong_list = [
        f"[{table}].{name}"
        for table in ("packages", "optional")
        for name, value in rules.get(table, {}).items()
        if not is_text_list(value)
    ]
    engine = rules.get("engine", {})
    wrong_list += [
        f"[engine].{key}"
        for key in ("packages", *ENGINE_KEYS, FORBIDDEN_CRATES_KEY)
        if key in engine and not is_text_list(engine[key])
    ]
    problems = [f"{name} must be a list of strings" for name in wrong_list]
    runtime = rules.get("runtime", {})
    if "package" in runtime and not isinstance(runtime["package"], str):
        problems.append("[runtime].package must be a string")
    problems += [
        f"[runtime].{key} must be a whole number"
        for key in ("methods", "in_domains")
        if key in runtime and not is_count(runtime[key])
    ]
    return problems


def read_rules(root: Path) -> dict:
    path = root / CONFIG
    if not path.is_file():
        raise CannotCheck(f"{CONFIG.as_posix()} is missing under {root}")
    try:
        rules = tomllib.loads(path.read_text(encoding="utf-8"))
    except tomllib.TOMLDecodeError as error:
        raise CannotCheck(f"{CONFIG.as_posix()} is not valid TOML: {error}") from error
    problems = shape_problems(rules)
    if problems:
        raise CannotCheck(f"{CONFIG.as_posix()}: {'; '.join(problems)}")
    engine = rules.get("engine", {})
    runtime = rules.get("runtime", {})
    missing = [
        f"[engine].{key}"
        for key in ("packages", *ENGINE_KEYS, FORBIDDEN_CRATES_KEY)
        if key not in engine
    ]
    missing += [f"[runtime].{key}" for key in ("package", "methods", "in_domains") if key not in runtime]
    if "packages" not in rules:
        missing.insert(0, "[packages]")
    if missing:
        raise CannotCheck(f"{CONFIG.as_posix()} lacks {', '.join(missing)}")
    return rules


def read_workspace(root: Path) -> Workspace:
    command = [
        "cargo", "metadata", "--format-version", "1", "--offline", "--no-deps",
        "--manifest-path", str(root / "Cargo.toml"),
    ]
    try:
        result = subprocess.run(command, capture_output=True, text=True, check=False)
    except OSError as error:
        raise CannotCheck(f"cargo metadata could not start: {error}") from error
    if result.returncode != 0:
        raise CannotCheck(
            f"cargo metadata failed with exit code {result.returncode}: {result.stderr.strip()}"
        )
    try:
        packages = json.loads(result.stdout)["packages"]
    except (ValueError, KeyError) as error:
        raise CannotCheck(f"cargo metadata printed no package list: {error}") from error
    names = {package["name"] for package in packages}
    directories = {package["name"]: Path(package["manifest_path"]).parent for package in packages}
    def normal(package: dict) -> list[dict]:
        return [
            dependency
            for dependency in package["dependencies"]
            if dependency.get("kind") is None and dependency["name"] in names
        ]

    uses = {package["name"]: {dep["name"] for dep in normal(package)} for package in packages}
    required = {
        package["name"]: {dep["name"] for dep in normal(package) if not dep.get("optional")}
        for package in packages
    }
    crates = {
        package["name"]: {(dep["name"], dep.get("rename") or dep["name"]) for dep in package["dependencies"]}
        for package in packages
    }
    return Workspace(directories, uses, required, crates)


def relative(path: Path, root: Path) -> str:
    try:
        return path.resolve().relative_to(root.resolve()).as_posix()
    except ValueError:
        return path.as_posix()


def production_files(directory: Path) -> list[RustFile]:
    return sorted(
        (file for file in crate_files(directory).values() if not file.test),
        key=lambda file: file.path,
    )


def check_packages(rules: dict, workspace: Workspace, report: Report) -> None:
    allowed = {name: set(uses) for name, uses in rules["packages"].items()}
    for name in sorted(workspace.uses.keys() - allowed.keys()):
        report.add(
            "package_without_rule",
            f"package {name} has no line in [packages] of layers.toml; add it with the "
            "workspace packages its layer lets it use",
        )
    named = (
        set(allowed)
        | set(rules.get("optional", {}))
        | set(rules["engine"]["packages"])
        | {rules["runtime"]["package"]}
    )
    for name in sorted(named - workspace.uses.keys()):
        report.add("rule_without_package", f"layers.toml names {name}, which is not a workspace package")
    for name in sorted(workspace.uses.keys() & allowed.keys()):
        listed = ", ".join(sorted(allowed[name])) or "none"
        for used in sorted(workspace.uses[name] - allowed[name]):
            report.add(
                "dependency_not_allowed",
                f"{name} uses {used}, which its rule does not allow (allowed: {listed})",
            )
        for unused in sorted(allowed[name] - workspace.uses[name]):
            report.add(
                "dependency_no_longer_used",
                f"{name} no longer uses {unused}: remove it from its rule in layers.toml",
            )


def check_optional(rules: dict, workspace: Workspace, report: Report) -> None:
    """Each dependency in [optional] is in its package's rule and really is optional."""
    for name, dependencies in sorted(rules.get("optional", {}).items()):
        if name not in workspace.uses:
            continue
        allowed = set(rules["packages"].get(name, []))
        for dependency in sorted(dependencies):
            if dependency not in allowed:
                report.add(
                    "dependency_must_be_optional",
                    f"{name} lists {dependency} in [optional], but its line in [packages] of "
                    "layers.toml does not allow it: add it there or remove it from [optional]",
                )
            elif dependency not in workspace.uses[name]:
                report.add(
                    "dependency_no_longer_used",
                    f"{name} no longer uses {dependency}: remove it from its line in [optional] "
                    "of layers.toml",
                )
            elif dependency in workspace.required[name]:
                report.add(
                    "dependency_must_be_optional",
                    f"{name} takes {dependency} as a required dependency; layers.toml lists it in "
                    "[optional], so declare it with optional = true",
                )


def check_forbidden_crates(rules: dict, workspace: Workspace, report: Report) -> None:
    """No engine package depends on a forbidden crate, whatever it is called there."""
    engine = rules["engine"]
    forbidden = set(engine[FORBIDDEN_CRATES_KEY])
    for package in sorted(set(engine["packages"]) & workspace.crates.keys()):
        for crate, alias in sorted(workspace.crates[package]):
            if crate in forbidden:
                report.add(
                    "engine_forbidden_crate",
                    f"engine package {package} depends on {crate}, under the name {alias}: "
                    "that belongs to a client package",
                )


def forbidden_offsets(file: RustFile, key: str) -> list[int]:
    """Where `key` appears in the file's code, outside tests."""
    offsets = []
    for match in FORBIDDEN[key].finditer(file.masked):
        if file.in_test(match.start()):
            continue
        if key == "cfg_macos" and not APPLE_VALUE.match(file.source, match.start()):
            continue
        offsets.append(match.start())
    return offsets


def check_engine(rules: dict, workspace: Workspace, root: Path, report: Report) -> None:
    engine = rules["engine"]
    found: dict[str, set[str]] = {key: set() for key in ENGINE_KEYS}
    for package in sorted(set(engine["packages"]) & workspace.directories.keys()):
        for file in production_files(workspace.directories[package]):
            path = relative(file.path, root)
            for key in ENGINE_KEYS:
                offsets = forbidden_offsets(file, key)
                if not offsets:
                    continue
                found[key].add(path)
                if path in engine[key]:
                    continue
                line = line_of(line_starts(file.masked), offsets[0])
                report.add(
                    "engine_forbidden_use",
                    f"{path}:{line}: {key} in engine package {package} ({len(offsets)} uses), "
                    "outside the files layers.toml allows",
                )
    for key in ENGINE_KEYS:
        for path in sorted(set(engine[key]) - found[key]):
            report.add(
                "engine_allowance_unused",
                f"{path} no longer has {key}: remove it from [engine].{key} in layers.toml",
            )


def module_of(file: RustFile, offset: int) -> tuple[str, ...]:
    """The module path at `offset`: the file's module and the inline modules around it."""
    inline = [
        scope.name
        for scope in file.scopes
        if scope.kind == "module" and scope.open < offset < scope.close
    ]
    return file.module + tuple(inline)


def domain_names(directory: Path) -> set[str]:
    runtime = directory / "src" / DOMAIN_PARENT / "runtime.rs"
    return {match.group(2) for match in HANDLE_FIELD.finditer(runtime.read_text(encoding="utf-8"))}


def count_runtime(directory: Path) -> tuple[int, int]:
    """Methods of `impl Runtime` outside tests, and how many sit in a domain folder."""
    domains = domain_names(directory)
    methods = in_domains = 0
    for file in production_files(directory):
        for scope in file.scopes:
            if scope.kind != "impl" or scope.name != COORDINATOR or file.in_test(scope.open):
                continue
            for child in scope.children:
                if child.kind != "function" or file.in_test(child.open):
                    continue
                methods += 1
                module = module_of(file, child.open)
                if len(module) > 1 and module[0] == DOMAIN_PARENT and module[1] in domains:
                    in_domains += 1
    return methods, in_domains


def check_runtime(rules: dict, workspace: Workspace, report: Report) -> None:
    package = rules["runtime"]["package"]
    if package not in workspace.directories:
        return
    methods, in_domains = count_runtime(workspace.directories[package])
    report.numbers.update(methods=methods, in_domains=in_domains)
    for measured, key, cause, what in (
        (methods, "methods", "runtime_methods", "methods of impl Runtime"),
        (in_domains, "in_domains", "runtime_domain_methods", "methods of impl Runtime in domain folders"),
    ):
        recorded = rules["runtime"][key]
        if measured > recorded:
            report.add(
                f"{cause}_grew",
                f"{package} has {measured} {what}, more than the {recorded} layers.toml records; "
                "move the new code to its domain's handle instead",
            )
        elif measured < recorded:
            report.add(
                f"{cause}_shrank",
                f"{package} has {measured} {what}, fewer than the {recorded} layers.toml records: "
                f"lower [runtime].{key} to {measured}",
            )


def summary(rules: dict, workspace: Workspace, report: Report) -> str:
    dependencies = sum(len(uses) for uses in workspace.uses.values())
    allowances = sum(len(rules["engine"][key]) for key in ENGINE_KEYS)
    methods = report.numbers.get("methods", "?")
    in_domains = report.numbers.get("in_domains", "?")
    return (
        f"{len(workspace.uses)} packages, {dependencies} workspace dependencies, "
        f"{allowances} engine file allowances, {methods} methods of impl Runtime "
        f"({in_domains} in domain folders)"
    )


def check(root: Path) -> int:
    rules = read_rules(root)
    workspace = read_workspace(root)
    report = Report()
    check_packages(rules, workspace, report)
    check_optional(rules, workspace, report)
    check_engine(rules, workspace, root, report)
    check_forbidden_crates(rules, workspace, report)
    check_runtime(rules, workspace, report)
    print(summary(rules, workspace, report))
    for problem in report.problems:
        print(f"  {problem}")
    if report.problems:
        print(f"{len(report.problems)} differences from {CONFIG.as_posix()}")
        return 1
    print(f"every package, engine file and Runtime count is the one {CONFIG.as_posix()} records")
    return 0


def main(arguments: list[str]) -> int:
    if len(arguments) > 1:
        print(f"layers.py: too many arguments\n{__doc__}", file=sys.stderr)
        return 2
    root = Path(arguments[0]).resolve() if arguments else REPOSITORY
    try:
        return check(root)
    except (CannotCheck, OSError) as error:
        print(f"layers.py cannot check: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
