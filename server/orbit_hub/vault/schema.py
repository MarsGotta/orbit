"""Vault-defined roles and field names; prose never configures the reader."""

import re

import yaml
from yaml.tokens import AliasToken, AnchorToken, TagToken

from .conformance import OKF_FIELDS

ROLES = ("person", "project", "meeting", "topic", "source", "journal", "note")
FIELD_NAMES = ("stage", "start")
DEFAULT_TYPES = {role: {"role": role} for role in ROLES if role != "note"}


def frontmatter(text):
    if not text.startswith(("---\n", "---\r\n")):
        return {}, text
    parts = re.split(r"^---\s*$", text, maxsplit=2, flags=re.MULTILINE)
    if len(parts) != 3:
        raise ValueError("falta el cierre del frontmatter (línea 1)")
    try:
        tokens = list(yaml.scan(parts[1]))
        if any(isinstance(t, (AliasToken, AnchorToken, TagToken)) for t in tokens):
            raise ValueError("el frontmatter contiene alias, anclas o etiquetas YAML")
        tree = yaml.compose(parts[1], Loader=yaml.BaseLoader)
        _unique_keys(tree)
        fields = yaml.load(parts[1], Loader=yaml.BaseLoader) or {}
    except yaml.YAMLError as exc:
        mark = getattr(exc, "problem_mark", None)
        line = mark.line + 1 if mark else 1
        raise ValueError(f"YAML inválido en la línea {line}") from None
    except RecursionError:
        raise ValueError("el frontmatter supera la profundidad YAML admitida") from None
    if not isinstance(fields, dict):
        raise TypeError("el frontmatter debe ser un mapa de campos (línea 2)")
    return fields, parts[2].lstrip("\r\n")


def _unique_keys(node):
    if isinstance(node, yaml.MappingNode):
        seen = set()
        for key, value in node.value:
            if not isinstance(key, yaml.ScalarNode):
                raise TypeError(f"clave YAML no escalar en la línea {key.start_mark.line + 1}")
            if key.value in seen:
                raise ValueError(f"clave YAML duplicada en la línea {key.start_mark.line + 1}")
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
        message = str(exc)
        location = _location(text, message)
        schema["warnings"] = [f"SCHEMA.md: {message}{location}. Se usan los valores por defecto."]
    return schema


def _location(text, message):
    if "línea" in message:
        return ""
    parts = re.split(r"^---\s*$", text, maxsplit=2, flags=re.MULTILINE)
    if len(parts) != 3:
        return ""
    try:
        root = yaml.compose(parts[1], Loader=yaml.BaseLoader)
        locations = []
        _locations(root, "", locations)
        matching = [(name, line) for name, line in locations if message.startswith(name + " ")]
        if matching:
            _, line = max(matching, key=lambda item: len(item[0]))
            return f" (línea {line})"
    except (yaml.YAMLError, RecursionError):
        pass
    return ""


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
        raise TypeError("types debe ser un mapa")
    for name, spec in types.items():
        if not isinstance(name, str) or not name.strip():
            raise ValueError("types contiene un nombre vacío")
        if not isinstance(spec, dict):
            raise TypeError(f"types.{name} debe ser un mapa con role")
        if spec.get("role") not in ROLES:
            raise ValueError(f"types.{name}.role debe ser uno de {', '.join(ROLES)}")
        if set(spec) - {"role", "folder"}:
            raise ValueError(f"types.{name} contiene campos distintos de role y folder")
        if "folder" in spec:
            folder = spec["folder"]
            if (
                not isinstance(folder, str)
                or not folder.endswith("/")
                or any(part in {"", ".", ".."} for part in folder[:-1].split("/"))
                or "\\" in folder
                or re.search(r"[\x00-\x1f]", folder)
            ):
                raise ValueError(
                    f"types.{name}.folder debe ser una carpeta relativa terminada en /"
                )
    folders = [spec["folder"] for spec in types.values() if "folder" in spec]
    if len(folders) != len(set(folders)):
        raise ValueError("types contiene carpetas duplicadas")


def _fields(fields):
    if not isinstance(fields, dict):
        raise TypeError("fields debe ser un mapa")
    for name, alias in fields.items():
        if name not in FIELD_NAMES:
            raise ValueError(f"fields.{name} no es un campo de Orbit")
        if isinstance(alias, str) and alias in (*OKF_FIELDS, "okf_version"):
            raise ValueError(f"fields.{name} no puede usar el campo de OKF {alias}")
        if not isinstance(alias, str) or not alias.strip():
            raise ValueError(f"fields.{name} debe ser un nombre de campo")


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
