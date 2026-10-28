"""Vault-defined roles and field names; prose never configures the reader."""

import re

import yaml
from yaml.tokens import AliasToken, AnchorToken, TagToken

from .conformance import OKF_FIELDS
from .diagnostics import SchemaProblem

ROLES = ("person", "project", "meeting", "topic", "source", "journal", "note")
FIELD_NAMES = ("stage", "start")
DEFAULT_TYPES = {role: {"role": role} for role in ROLES if role != "note"}


def frontmatter(text):
    if not text.startswith(("---\n", "---\r\n")):
        return {}, text
    parts = re.split(r"^---\s*$", text, maxsplit=2, flags=re.MULTILINE)
    if len(parts) != 3:
        raise SchemaProblem(
            "schema_frontmatter_unclosed", "frontmatter closing delimiter missing", line=1
        )
    try:
        tokens = list(yaml.scan(parts[1]))
        if any(isinstance(t, (AliasToken, AnchorToken, TagToken)) for t in tokens):
            raise SchemaProblem(
                "schema_yaml_unsafe", "frontmatter contains YAML aliases, anchors or tags"
            )
        tree = yaml.compose(parts[1], Loader=yaml.BaseLoader)
        _unique_keys(tree)
        fields = yaml.load(parts[1], Loader=yaml.BaseLoader) or {}
    except yaml.YAMLError as exc:
        mark = getattr(exc, "problem_mark", None)
        line = mark.line + 1 if mark else 1
        raise SchemaProblem("schema_yaml_invalid", "invalid YAML", line=line) from None
    except RecursionError:
        raise SchemaProblem(
            "schema_yaml_depth", "frontmatter exceeds supported YAML depth"
        ) from None
    if not isinstance(fields, dict):
        raise SchemaProblem(
            "schema_frontmatter_mapping", "frontmatter must be a field mapping", line=2
        )
    return fields, parts[2].lstrip("\r\n")


def _unique_keys(node):
    if isinstance(node, yaml.MappingNode):
        seen = set()
        for key, value in node.value:
            if not isinstance(key, yaml.ScalarNode):
                raise SchemaProblem(
                    "schema_yaml_key_type", "non-scalar YAML key", line=key.start_mark.line + 1
                )
            if key.value in seen:
                raise SchemaProblem(
                    "schema_yaml_key_duplicate", "duplicate YAML key", line=key.start_mark.line + 1
                )
            seen.add(key.value)
            _unique_keys(value)
    elif isinstance(node, yaml.SequenceNode):
        for value in node.value:
            _unique_keys(value)


def load(text=""):
    schema = {
        "path": "SCHEMA.md" if text else "",
        "body": "",
        "folders": {},
        "types": DEFAULT_TYPES.copy(),
        "fields": {name: name for name in FIELD_NAMES},
        "warnings": [],
        "diagnostics": [],
    }
    try:
        fields, schema["body"] = frontmatter(text)
        types = fields.get("types", DEFAULT_TYPES)
        _types(types)
        aliases = fields.get("fields", {})
        _fields(aliases)
        schema["types"] = types
        schema["fields"].update(aliases)
        schema["folders"] = {
            name: spec["folder"] for name, spec in types.items() if "folder" in spec
        }
    except (ValueError, TypeError, RecursionError) as exc:
        problem = (
            exc
            if isinstance(exc, SchemaProblem)
            else SchemaProblem("schema_yaml_depth", "schema exceeds supported YAML depth")
        )
        line = problem.line or _location(text, problem.field)
        location = f" (line {line})" if line else ""
        schema["warnings"] = [f"SCHEMA.md: {problem}{location}. Defaults are used."]
        schema["diagnostics"] = [
            {
                "code": problem.code,
                "field": problem.field,
                "numbers": {"line": line} if line else {},
                **({"value": problem.value} if problem.value is not None else {}),
            }
        ]
    return schema


def _location(text, field):
    if not field:
        return 0
    parts = re.split(r"^---\s*$", text, maxsplit=2, flags=re.MULTILINE)
    if len(parts) != 3:
        return 0
    try:
        root = yaml.compose(parts[1], Loader=yaml.BaseLoader)
        locations = []
        _locations(root, "", locations)
        matching = [
            (name, line)
            for name, line in locations
            if field == name or field.startswith(name + ".")
        ]
        if matching:
            return max(matching, key=lambda item: len(item[0]))[1]
    except (yaml.YAMLError, RecursionError):
        pass
    return 0


def _locations(node, prefix, result):
    if not isinstance(node, yaml.MappingNode):
        return
    for key, value in node.value:
        if isinstance(key, yaml.ScalarNode):
            name = prefix + key.value
            result.append((name, key.start_mark.line + 1))
            _locations(value, name + ".", result)


def _types(types):
    if not isinstance(types, dict):
        raise SchemaProblem("schema_types_mapping", "types must be a mapping", "types")
    for name, spec in types.items():
        if not isinstance(name, str) or not name.strip():
            raise SchemaProblem("schema_type_name", "types contains an empty name", "types")
        if not isinstance(spec, dict):
            raise SchemaProblem(
                "schema_type_mapping", f"types.{name} must be a mapping with role", f"types.{name}"
            )
        if spec.get("role") not in ROLES:
            raise SchemaProblem(
                "schema_type_role",
                f"types.{name}.role must be one of {', '.join(ROLES)}",
                f"types.{name}.role",
            )
        if set(spec) - {"role", "folder"}:
            raise SchemaProblem(
                "schema_type_fields",
                f"types.{name} contains fields other than role and folder",
                f"types.{name}",
            )
        if "folder" in spec:
            folder = spec["folder"]
            if (
                not isinstance(folder, str)
                or not folder.endswith("/")
                or any(part in {"", ".", ".."} for part in folder[:-1].split("/"))
                or "\\" in folder
                or re.search(r"[\x00-\x1f]", folder)
            ):
                raise SchemaProblem(
                    "schema_type_folder",
                    f"types.{name}.folder must be a relative folder ending in /",
                    f"types.{name}.folder",
                )
    folders = [spec["folder"] for spec in types.values() if "folder" in spec]
    if len(folders) != len(set(folders)):
        raise SchemaProblem("schema_folders_duplicate", "types contains duplicate folders", "types")


def _fields(fields):
    if not isinstance(fields, dict):
        raise SchemaProblem("schema_fields_mapping", "fields must be a mapping", "fields")
    for name, alias in fields.items():
        if name not in FIELD_NAMES:
            raise SchemaProblem(
                "schema_field_unknown", f"fields.{name} is not an Orbit field", f"fields.{name}"
            )
        if isinstance(alias, str) and alias in (*OKF_FIELDS, "okf_version"):
            raise SchemaProblem(
                "schema_field_reserved",
                f"fields.{name} cannot use the OKF field {alias}",
                f"fields.{name}",
                value=alias,
            )
        if not isinstance(alias, str) or not alias.strip():
            raise SchemaProblem(
                "schema_field_name", f"fields.{name} must be a field name", f"fields.{name}"
            )


def properties(path, fields, schema):
    kind = fields.get("type")
    if not isinstance(kind, str) or not kind.strip():
        folders = sorted(schema["folders"].items(), key=lambda item: -len(item[1]))
        kind = next((name for name, folder in folders if path.startswith(folder)), "note")
    role = schema["types"].get(kind, {}).get("role", "note")
    values = {name: fields[name] for name in OKF_FIELDS if name in fields}
    values.update(
        {name: fields[alias] for name, alias in schema["fields"].items() if alias in fields}
    )
    values["status"] = fields.get("status", "stable")
    if isinstance(values.get("verified"), dict):
        values["verified"] = [values["verified"]]
    return kind, role, values
