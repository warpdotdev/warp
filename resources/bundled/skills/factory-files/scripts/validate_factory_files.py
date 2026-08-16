#!/usr/bin/env python3
"""Validate a Factory file tree against the bundled v1alpha1 JSON Schemas.

Usage:
    python3 validate_factory_files.py [FACTORY_ROOT] [--json] [--schemas DIR]

FACTORY_ROOT defaults to the current directory and must contain factory.yaml.

The script is intentionally dependency-free: it ships a restricted YAML reader
for the canonical forms this skill emits (no anchors, aliases, explicit tags,
or multiple documents) and a JSON Schema evaluator covering the keywords the
bundled schemas use. It is not a general-purpose YAML implementation. Anything
it cannot read confidently is reported rather than guessed at.

It does not replace server-side validation. Provider catalogues, model IDs,
environment IDs, secret names, and runner references are resolved by the
server; this checks structure, field names, enums, and cross-file references.

FORWARD COMPATIBILITY - DO NOT TIGHTEN
--------------------------------------
This validator and its schemas ship inside a Warp release, so they are
routinely older than the warp-server they are used against. They therefore
accept some input the current server rejects, on purpose. Unknown properties,
agent types, credential strategies, harness types and per-harness
capabilities, integration slugs, trigger providers and events, runner
platforms, Scorer output forms, and the Scorer label cap are all deferred to
the server.

If you are here because the validator accepted something the server rejects,
the fix is usually a clearer server diagnostic, not a stricter schema. A false
rejection is far more expensive than a false acceptance: it blocks correct
work and invites an agent to "repair" valid configuration by deleting it,
whereas the server revalidates every tree at apply time anyway.

Two checks are deliberately kept strict, and both are scoped so drift cannot
trip them: trigger filter keys apply only when the provider and event are both
recognized, and an unrecognized schemaVersion stops validation instead of
misapplying v1alpha1 rules. See specs/REMOTE-2727/TECH.md.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import sys
import unicodedata
from pathlib import Path
from typing import Any, Optional

SCHEMA_BY_KIND = {
    "factory": "factory.schema.json",
    "agent": "agent.schema.json",
    "automation": "automation.schema.json",
    "runner": "runner.schema.json",
    "scorer": "scorer.schema.json",
}

MAIN_AGENT_TYPES = {"MAIN", "FOREMAN"}


class Problem:
    """One validation failure, located as precisely as the input allows."""

    def __init__(self, path: str, message: str, line: Optional[int] = None, pointer: str = ""):
        self.path = path
        self.message = message
        self.line = line
        self.pointer = pointer

    def as_dict(self) -> dict[str, Any]:
        return {
            "path": self.path,
            "line": self.line,
            "pointer": self.pointer,
            "message": self.message,
        }

    def render(self) -> str:
        location = self.path
        if self.line is not None:
            location += f":{self.line}"
        if self.pointer:
            return f"{location}: {self.pointer}: {self.message}"
        return f"{location}: {self.message}"


# ---------------------------------------------------------------------------
# Restricted YAML reader
# ---------------------------------------------------------------------------


class YamlError(Exception):
    def __init__(self, message: str, line: int):
        super().__init__(message)
        self.message = message
        self.line = line


class _Line:
    __slots__ = ("number", "indent", "content")

    def __init__(self, number: int, indent: int, content: str):
        self.number = number
        self.indent = indent
        self.content = content


# Characters after which a quote opens a quoted scalar. Anywhere else a quote
# is an ordinary character, so "It's a thing" stays a plain scalar rather than
# an unterminated string.
_VALUE_START_CHARS = ":,[{-"


def _strip_comment(raw: str, line_number: int) -> str:
    """Remove a trailing comment, honoring quoted scalars."""
    out: list[str] = []
    quote: Optional[str] = None
    previous = ""
    index = 0
    while index < len(raw):
        char = raw[index]
        if quote:
            out.append(char)
            if char == "\\" and quote == '"' and index + 1 < len(raw):
                out.append(raw[index + 1])
                index += 2
                continue
            if char == quote:
                if quote == "'" and index + 1 < len(raw) and raw[index + 1] == "'":
                    out.append("'")
                    index += 2
                    continue
                quote = None
            index += 1
            continue
        if char in "\"'" and (previous == "" or previous in _VALUE_START_CHARS):
            quote = char
            out.append(char)
            previous = char
            index += 1
            continue
        if char == "#" and (index == 0 or raw[index - 1] in " \t"):
            break
        out.append(char)
        if char not in " \t":
            previous = char
        index += 1
    if quote:
        raise YamlError("unterminated quoted string", line_number)
    return "".join(out).rstrip()


def _reject_unsupported(content: str, line_number: int) -> None:
    """Reject line-level constructs the Factory file parser does not accept.

    Anchors, aliases, and tags are checked in [_parse_scalar] instead, because
    they are only meaningful where a node begins; scanning the whole line
    rejects ordinary prose such as "A & B" or "see *this*".
    """
    if content.strip() in ("---", "..."):
        raise YamlError("multiple YAML documents are not permitted", line_number)
    if re.match(r"^\s*<<\s*:", content):
        raise YamlError("yaml merge keys are not permitted", line_number)


def _opens_block_scalar(content: str, line_number: int) -> bool:
    while content.startswith("- "):
        content = content[2:].lstrip()
    entry = _split_key(content, line_number)
    value = entry[1] if entry is not None else content
    return value[:1] in ("|", ">")


def _skip_block_scalar_body(raw_lines: list[str], index: int, header_indent: int) -> int:
    """Return the index of the first line after a block scalar's body."""
    while index < len(raw_lines):
        raw = raw_lines[index]
        if raw.strip() == "":
            index += 1
            continue
        if len(raw) - len(raw.lstrip(" ")) <= header_indent:
            break
        index += 1
    return index


def _read_lines(text: str) -> list[_Line]:
    raw_lines = text.replace("\r\n", "\n").replace("\r", "\n").split("\n")
    lines: list[_Line] = []
    index = 0
    while index < len(raw_lines):
        raw = raw_lines[index]
        number = index + 1
        index += 1
        if "\t" in raw[: len(raw) - len(raw.lstrip(" \t"))]:
            raise YamlError("tabs are not permitted for indentation", number)
        content = _strip_comment(raw, number)
        if not content.strip():
            continue
        _reject_unsupported(content, number)
        indent = len(content) - len(content.lstrip(" "))
        stripped = content.strip()
        lines.append(_Line(number, indent, stripped))
        # A block scalar's body is opaque text. Leaving it out of the
        # structural line list keeps its content from being read as YAML.
        if _opens_block_scalar(stripped, number):
            index = _skip_block_scalar_body(raw_lines, index, indent)
    return lines


_INT_RE = re.compile(r"^[-+]?[0-9]+$")
_HEX_RE = re.compile(r"^[-+]?0x[0-9a-fA-F]+$")
_OCT_RE = re.compile(r"^[-+]?0o[0-7]+$")
_FLOAT_RE = re.compile(r"^[-+]?(\.[0-9]+|[0-9]+(\.[0-9]*)?)([eE][-+]?[0-9]+)?$")
_YAML_DATE_RE = re.compile(r"^\d{4}-\d{2}-\d{2}(?:[Tt ]\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:[ \t]*(?:Z|[-+]\d{1,2}(?::\d{2})?))?)?$")


def _parse_scalar(token: str, line_number: int) -> Any:
    token = token.strip()
    if token == "" or token == "~" or token in ("null", "Null", "NULL"):
        return None
    if token.startswith("'"):
        if not token.endswith("'") or len(token) < 2:
            raise YamlError("invalid single-quoted string", line_number)
        return token[1:-1].replace("''", "'")
    if token.startswith('"'):
        try:
            return json.loads(token)
        except json.JSONDecodeError as error:
            raise YamlError(f"invalid double-quoted string: {error.msg}", line_number) from error
    if token[0] in "&*":
        raise YamlError("yaml anchors and aliases are not permitted", line_number)
    if token[0] == "!":
        raise YamlError("explicit yaml tags are not permitted", line_number)
    if token in ("true", "True", "TRUE"):
        return True
    if token in ("false", "False", "FALSE"):
        return False
    if _INT_RE.match(token):
        return int(token, 10)
    if _HEX_RE.match(token):
        return int(token, 16)
    if _OCT_RE.match(token):
        return int(token, 8)
    if _FLOAT_RE.match(token):
        return float(token)
    if _YAML_DATE_RE.match(token):
        raise YamlError("timestamps must be quoted so YAML keeps them as strings", line_number)
    if token.lower() in {".inf", "+.inf", "-.inf", ".nan"}:
        raise YamlError("non-finite YAML numbers are not permitted", line_number)
    return token


def _split_key(content: str, line_number: int) -> Optional[tuple[str, str]]:
    """Split `key: value`, honoring quoted keys. Returns None when absent."""
    quote: Optional[str] = None
    index = 0
    while index < len(content):
        char = content[index]
        if quote:
            if char == "\\" and quote == '"' and index + 1 < len(content):
                index += 2
                continue
            if char == quote:
                if quote == "'" and index + 1 < len(content) and content[index + 1] == "'":
                    index += 2
                    continue
                quote = None
            index += 1
            continue
        if char in "\"'":
            quote = char
            index += 1
            continue
        if char in "[{":
            return None
        if char == ":" and (index + 1 == len(content) or content[index + 1] in " \t"):
            key_token = content[:index].strip()
            key = _parse_scalar(key_token, line_number)
            if not isinstance(key, str):
                key = key_token
            return key, content[index + 1 :].strip()
        index += 1
    return None


def _parse_flow(text: str, line_number: int) -> Any:
    value, rest = _parse_flow_value(text.strip(), line_number)
    if rest.strip():
        raise YamlError("unexpected trailing content after flow collection", line_number)
    return value


def _parse_flow_value(text: str, line_number: int) -> tuple[Any, str]:
    text = text.lstrip()
    if not text:
        raise YamlError("unexpected end of flow collection", line_number)
    if text[0] == "[":
        items: list[Any] = []
        rest = text[1:].lstrip()
        if rest.startswith("]"):
            return items, rest[1:]
        while True:
            item, rest = _parse_flow_value(rest, line_number)
            items.append(item)
            rest = rest.lstrip()
            if rest.startswith(","):
                rest = rest[1:].lstrip()
                if rest.startswith("]"):
                    return items, rest[1:]
                continue
            if rest.startswith("]"):
                return items, rest[1:]
            raise YamlError("unterminated flow sequence", line_number)
    if text[0] == "{":
        mapping: dict[str, Any] = {}
        rest = text[1:].lstrip()
        if rest.startswith("}"):
            return mapping, rest[1:]
        while True:
            key_text, rest = _read_flow_scalar(rest, line_number)
            rest = rest.lstrip()
            if not rest.startswith(":"):
                raise YamlError("flow mapping entry is missing ':'", line_number)
            value, rest = _parse_flow_value(rest[1:], line_number)
            key = _parse_scalar(key_text, line_number)
            if not isinstance(key, str):
                key = key_text.strip()
            if key in mapping:
                raise YamlError(f'duplicate key "{key}"', line_number)
            mapping[key] = value
            rest = rest.lstrip()
            if rest.startswith(","):
                rest = rest[1:].lstrip()
                if rest.startswith("}"):
                    return mapping, rest[1:]
                continue
            if rest.startswith("}"):
                return mapping, rest[1:]
            raise YamlError("unterminated flow mapping", line_number)
    token, rest = _read_flow_scalar(text, line_number)
    return _parse_scalar(token, line_number), rest


def _read_flow_scalar(text: str, line_number: int) -> tuple[str, str]:
    text = text.lstrip()
    if text[:1] in ("'", '"'):
        quote = text[0]
        index = 1
        while index < len(text):
            if text[index] == "\\" and quote == '"':
                index += 2
                continue
            if text[index] == quote:
                if quote == "'" and text[index + 1 : index + 2] == "'":
                    index += 2
                    continue
                return text[: index + 1], text[index + 1 :]
            index += 1
        raise YamlError("unterminated quoted string in flow collection", line_number)
    index = 0
    while index < len(text) and text[index] not in ",]}:":
        index += 1
    return text[:index].strip(), text[index:]


class _Reader:
    def __init__(self, lines: list[_Line], raw_lines: list[str]):
        self.lines = lines
        self.raw_lines = raw_lines
        self.index = 0

    def peek(self) -> Optional[_Line]:
        return self.lines[self.index] if self.index < len(self.lines) else None

    def parse_block(self, indent: int) -> Any:
        line = self.peek()
        if line is None or line.indent < indent:
            return None
        if line.content.startswith("- ") or line.content == "-":
            return self._parse_sequence(line.indent)
        return self._parse_mapping(line.indent)

    def _parse_sequence(self, indent: int) -> list[Any]:
        items: list[Any] = []
        while True:
            line = self.peek()
            if line is None or line.indent != indent:
                break
            if not (line.content.startswith("- ") or line.content == "-"):
                break
            self.index += 1
            remainder = line.content[1:].strip()
            if remainder == "":
                items.append(self.parse_block(indent + 1))
                continue
            entry = _split_key(remainder, line.number)
            if entry is not None:
                # An inline mapping entry opens a mapping whose remaining keys
                # are indented to the column the first key started at.
                after_dash = line.content[1:]
                lead = len(after_dash) - len(after_dash.lstrip(" "))
                inline_indent = indent + 1 + lead
                items.append(self._parse_inline_mapping(entry, line.number, inline_indent))
                continue
            items.append(self._parse_value(remainder, line.number, indent))
        return items

    def _parse_inline_mapping(
        self, entry: tuple[str, str], line_number: int, indent: int
    ) -> dict[str, Any]:
        key, value_token = entry
        mapping: dict[str, Any] = {key: self._parse_value(value_token, line_number, indent)}
        return self._parse_mapping(indent, existing=mapping)

    def _parse_mapping(
        self, indent: int, existing: Optional[dict[str, Any]] = None
    ) -> dict[str, Any]:
        mapping: dict[str, Any] = existing if existing is not None else {}
        while True:
            line = self.peek()
            if line is None or line.indent != indent:
                break
            if line.content.startswith("- "):
                break
            entry = _split_key(line.content, line.number)
            if entry is None:
                raise YamlError(f"expected 'key: value', found {line.content!r}", line.number)
            self.index += 1
            key, value_token = entry
            if key in mapping:
                raise YamlError(f'duplicate key "{key}"', line.number)
            mapping[key] = self._parse_value(value_token, line.number, indent)
        return mapping

    def _parse_value(self, token: str, line_number: int, indent: int) -> Any:
        if token.startswith("|") or token.startswith(">"):
            return self._parse_block_scalar(token, line_number, indent)
        if token.startswith("[") or token.startswith("{"):
            return _parse_flow(token, line_number)
        if token != "":
            return _parse_scalar(token, line_number)
        nested = self.peek()
        if nested is None or nested.indent <= indent:
            return None
        return self.parse_block(nested.indent)

    def _parse_block_scalar(self, header: str, line_number: int, indent: int) -> str:
        style = header[0]
        chomp = "clip"
        if "-" in header[1:]:
            chomp = "strip"
        elif "+" in header[1:]:
            chomp = "keep"
        collected: list[str] = []
        # line_number is 1-based, so it indexes the line after the header.
        cursor = line_number
        block_indent: Optional[int] = None
        while cursor < len(self.raw_lines):
            raw = self.raw_lines[cursor]
            if raw.strip() == "":
                collected.append("")
                cursor += 1
                continue
            current_indent = len(raw) - len(raw.lstrip(" "))
            if current_indent <= indent:
                break
            if block_indent is None:
                block_indent = current_indent
            collected.append(raw[block_indent:])
            cursor += 1
        if chomp != "keep":
            while collected and collected[-1] == "":
                collected.pop()
        if style == "|":
            text = "\n".join(collected)
        else:
            text = " ".join(part.strip() for part in collected if part.strip())
        if chomp == "strip" or not text:
            return text
        return text + "\n"


def load_yaml(text: str) -> Any:
    """Parse the restricted YAML subset the Factory file format accepts."""
    raw_lines = text.replace("\r\n", "\n").replace("\r", "\n").split("\n")
    lines = _read_lines(text)
    if not lines:
        return None
    reader = _Reader(lines, raw_lines)
    value = reader.parse_block(lines[0].indent)
    remaining = reader.peek()
    if remaining is not None:
        raise YamlError(f"unexpected content {remaining.content!r}", remaining.number)
    return value


def split_frontmatter(text: str) -> tuple[str, str, int]:
    """Return a Markdown resource's frontmatter, body, and line offset."""
    normalized = text.replace("\r\n", "\n").replace("\r", "\n")
    lines = normalized.split("\n")
    if not lines or lines[0].rstrip() != "---":
        raise YamlError("resource file must start with a frontmatter fence (---)", 1)
    for index in range(1, len(lines)):
        if lines[index].rstrip() == "---":
            return "\n".join(lines[1:index]), "\n".join(lines[index + 1 :]), 1
    raise YamlError("frontmatter is missing a closing fence (---)", 1)


# ---------------------------------------------------------------------------
# JSON Schema evaluator
# ---------------------------------------------------------------------------

_TYPE_CHECKS = {
    "object": lambda value: isinstance(value, dict),
    "array": lambda value: isinstance(value, list),
    "string": lambda value: isinstance(value, str),
    "integer": lambda value: isinstance(value, int) and not isinstance(value, bool),
    "number": lambda value: isinstance(value, (int, float)) and not isinstance(value, bool),
    "boolean": lambda value: isinstance(value, bool),
    "null": lambda value: value is None,
}


class SchemaStore:
    """Loads sibling schema files and resolves local and relative $refs."""

    def __init__(self, directory: Path):
        self.directory = directory
        self._cache: dict[str, Any] = {}

    def document(self, filename: str) -> Any:
        if filename not in self._cache:
            path = self.directory / filename
            self._cache[filename] = json.loads(path.read_text(encoding="utf-8"))
        return self._cache[filename]

    def resolve(self, ref: str, current: str) -> tuple[Any, str]:
        filename, _, pointer = ref.partition("#")
        target = filename or current
        document = self.document(target)
        node = document
        for token in [segment for segment in pointer.split("/") if segment]:
            token = token.replace("~1", "/").replace("~0", "~")
            node = node[token]
        return node, target


# Keywords the evaluator implements, and the annotations it may ignore. A
# keyword in neither set is reported rather than skipped: silently ignoring an
# unimplemented keyword would under-validate without any signal.
_SUPPORTED_KEYWORDS = frozenset(
    {
        "$ref",
        "type",
        "const",
        "enum",
        "minLength",
        "maxLength",
        "pattern",
        "minimum",
        "maximum",
        "minItems",
        "maxItems",
        "uniqueItems",
        "items",
        "required",
        "minProperties",
        "properties",
        "additionalProperties",
        "propertyNames",
        "allOf",
        "anyOf",
        "oneOf",
        "not",
        "if",
        "then",
        "else",
    }
)
_ANNOTATION_KEYWORDS = frozenset(
    {
        "$schema",
        "$id",
        "$defs",
        "$comment",
        "title",
        "description",
        "x-warp-character-class",
        "x-warp-known-max-items",
        "x-warp-known-values",
        "x-warp-max-trimmed-runes",
    }
)



def _matches_pattern(pattern: str, value: str) -> bool:
    return re.search(pattern, value) is not None


def _describe(value: Any) -> str:
    for name, check in _TYPE_CHECKS.items():
        if name != "number" and check(value):
            return name
    return type(value).__name__


def validate_instance(
    instance: Any,
    schema: Any,
    store: SchemaStore,
    document: str,
    pointer: str = "",
) -> list[str]:
    """Evaluate the JSON Schema keywords used by the bundled schemas."""
    if schema is True or schema == {}:
        return []
    if schema is False:
        return [f"{pointer or '/'}: no value is allowed here"]

    errors: list[str] = []

    unsupported = set(schema) - _SUPPORTED_KEYWORDS - _ANNOTATION_KEYWORDS
    if unsupported:
        listed = ", ".join(sorted(unsupported))
        errors.append(
            f"{pointer or '/'}: this validator does not implement schema keyword(s) {listed}; "
            "its result is incomplete until they are added"
        )

    if "$ref" in schema:
        target, target_document = store.resolve(schema["$ref"], document)
        errors.extend(validate_instance(instance, target, store, target_document, pointer))

    if "type" in schema:
        declared = schema["type"]
        names = declared if isinstance(declared, list) else [declared]
        if not any(_TYPE_CHECKS[name](instance) for name in names):
            errors.append(
                f"{pointer or '/'}: expected {' or '.join(names)}, found {_describe(instance)}"
            )
            return errors

    if "const" in schema and instance != schema["const"]:
        errors.append(f"{pointer or '/'}: must be {json.dumps(schema['const'])}")
    if "enum" in schema and instance not in schema["enum"]:
        allowed = ", ".join(json.dumps(option) for option in schema["enum"])
        errors.append(f"{pointer or '/'}: {json.dumps(instance)} must be one of {allowed}")

    if isinstance(instance, str):
        if "minLength" in schema and len(instance) < schema["minLength"]:
            required = schema["minLength"]
            detail = "must not be empty" if required == 1 else f"must be at least {required} characters"
            errors.append(f"{pointer or '/'}: {detail}")
        if "maxLength" in schema and len(instance) > schema["maxLength"]:
            errors.append(f"{pointer or '/'}: must be at most {schema['maxLength']} characters")
        if "pattern" in schema and not _matches_pattern(schema["pattern"], instance):
            errors.append(f"{pointer or '/'}: {json.dumps(instance)} does not match the required format")

    if isinstance(instance, (int, float)) and not isinstance(instance, bool):
        if "minimum" in schema and instance < schema["minimum"]:
            errors.append(f"{pointer or '/'}: must be at least {schema['minimum']}")
        if "maximum" in schema and instance > schema["maximum"]:
            errors.append(f"{pointer or '/'}: must be at most {schema['maximum']}")

    if isinstance(instance, list):
        if "minItems" in schema and len(instance) < schema["minItems"]:
            errors.append(f"{pointer or '/'}: must contain at least {schema['minItems']} entries")
        if "maxItems" in schema and len(instance) > schema["maxItems"]:
            errors.append(f"{pointer or '/'}: must contain at most {schema['maxItems']} entries")
        if schema.get("uniqueItems") and _has_duplicates(instance):
            errors.append(f"{pointer or '/'}: entries must be unique")
        if "items" in schema:
            for index, item in enumerate(instance):
                errors.extend(
                    validate_instance(item, schema["items"], store, document, f"{pointer}/{index}")
                )

    if isinstance(instance, dict):
        for name in schema.get("required", []):
            if name not in instance:
                errors.append(f"{pointer or '/'}: {name} is required")
        if "minProperties" in schema and len(instance) < schema["minProperties"]:
            errors.append(f"{pointer or '/'}: must declare at least {schema['minProperties']} field")
        properties = schema.get("properties", {})
        for name, value in instance.items():
            if name in properties:
                errors.extend(
                    validate_instance(value, properties[name], store, document, f"{pointer}/{name}")
                )
            elif "additionalProperties" in schema:
                additional = schema["additionalProperties"]
                if additional is False:
                    known = ", ".join(sorted(properties)) or "none"
                    errors.append(
                        f"{pointer or '/'}: unknown field {json.dumps(name)} (accepted: {known})"
                    )
                else:
                    errors.extend(
                        validate_instance(value, additional, store, document, f"{pointer}/{name}")
                    )
        if "propertyNames" in schema:
            for name in instance:
                errors.extend(
                    validate_instance(
                        name, schema["propertyNames"], store, document, f"{pointer}/{name}"
                    )
                )

    for subschema in schema.get("allOf", []):
        errors.extend(validate_instance(instance, subschema, store, document, pointer))

    if "anyOf" in schema:
        branches = [
            validate_instance(instance, subschema, store, document, pointer)
            for subschema in schema["anyOf"]
        ]
        if all(branch for branch in branches):
            errors.append(_combine(pointer, schema, branches, "does not match any accepted form"))

    if "oneOf" in schema:
        branches = [
            validate_instance(instance, subschema, store, document, pointer)
            for subschema in schema["oneOf"]
        ]
        matched = [index for index, branch in enumerate(branches) if not branch]
        if len(matched) == 0:
            errors.append(_combine(pointer, schema, branches, "does not match any accepted form"))
        elif len(matched) > 1:
            errors.append(
                f"{pointer or '/'}: matches more than one mutually exclusive form"
                + (f" ({schema['description']})" if "description" in schema else "")
            )

    if "not" in schema and not validate_instance(instance, schema["not"], store, document, pointer):
        errors.append(
            f"{pointer or '/'}: "
            + (schema.get("description") or "this form is not allowed here")
        )

    if "if" in schema:
        matched = not validate_instance(instance, schema["if"], store, document, pointer)
        branch = schema.get("then") if matched else schema.get("else")
        if branch is not None:
            errors.extend(validate_instance(instance, branch, store, document, pointer))

    return errors


def _combine(pointer: str, schema: Any, branches: list[list[str]], summary: str) -> str:
    detail = schema.get("description")
    head = f"{pointer or '/'}: {detail or summary}"
    nested = sorted({message for branch in branches for message in branch})
    if not nested:
        return head
    return head + " — " + "; ".join(nested[:4])


def _has_duplicates(items: list[Any]) -> bool:
    seen: list[str] = []
    for item in items:
        key = json.dumps(item, sort_keys=True)
        if key in seen:
            return True
        seen.append(key)
    return False


# ---------------------------------------------------------------------------
# Factory tree traversal
# ---------------------------------------------------------------------------


def classify(relative: str) -> tuple[str, str]:
    """Mirror the server's path classification. Returns (kind, name)."""
    if relative == "factory.yaml":
        return "factory", ""
    segments = relative.split("/")
    if len(segments) >= 2 and segments[0] == "skills":
        return "skill", ""
    if len(segments) >= 4 and segments[0] == "agents" and segments[2] == "skills":
        return "skill", ""
    if len(segments) == 3 and segments[0] == "agents" and segments[2] == "agent.md":
        return ("agent", segments[1]) if _valid_name(segments[1]) else ("invalid", "")
    if len(segments) == 3 and segments[0] == "automations" and segments[2] == "automation.md":
        return ("automation", segments[1]) if _valid_name(segments[1]) else ("invalid", "")
    if len(segments) == 2 and segments[0] == "automations" and segments[1].endswith(".md"):
        name = segments[1][: -len(".md")]
        return ("automation", name) if _valid_name(name) else ("invalid", "")
    if len(segments) == 2 and segments[0] == "runners" and segments[1].endswith(".yaml"):
        name = segments[1][: -len(".yaml")]
        return ("runner", name) if _valid_name(name) else ("invalid", "")
    if len(segments) == 3 and segments[0] == "scorers" and segments[2] == "scorer.md":
        return ("scorer", segments[1]) if _valid_name(segments[1]) else ("invalid", "")
    if len(segments) == 2 and segments[0] == "scorers" and segments[1].endswith(".md"):
        return "invalid", ""
    base = segments[-1]
    if segments[0] == "agents" and base == "agent.md":
        return "invalid", ""
    if segments[0] == "automations" and base == "automation.md":
        return "invalid", ""
    if segments[0] == "runners" and base.endswith(".yaml"):
        return "invalid", ""
    if segments[0] == "scorers" and base == "scorer.md":
        return "invalid", ""
    return "unrelated", ""


def _valid_name(name: str) -> bool:
    return name not in ("", ".", "..") and "/" not in name

def _resource_files(root: Path) -> list[Path]:
    files = [root / "factory.yaml"]
    for directory_name in ("agents", "automations", "runners", "scorers"):
        resource_root = root / directory_name
        if not resource_root.is_dir():
            continue
        for directory, child_directories, filenames in os.walk(resource_root):
            relative_directory = Path(directory).relative_to(root)
            parts = relative_directory.parts
            if directory_name == "agents" and len(parts) == 2:
                child_directories[:] = [name for name in child_directories if name != "skills"]
            files.extend(Path(directory) / filename for filename in filenames)
    return sorted(files)


def _leaves_factory_root(path: Path, root: Path) -> bool:
    """Report whether reading path would follow a link out of the tree.

    The server never resolves links: it parses an in-memory git tree, where a
    symlink is a blob whose content is the target path, so it sees the link
    itself. Following one here would both diverge from that and read a file the
    Factory does not contain - an untrusted repository could otherwise point a
    resource at any readable path and have its content echoed back in a parse
    error.
    """
    if path.is_symlink():
        return True
    try:
        path.resolve().relative_to(root.resolve())
    except (OSError, ValueError, RuntimeError):
        return True
    return False


SUPPORTED_SCHEMA_VERSION = "v1alpha1"


def _unsupported_schema_version(root: Path) -> Optional[Problem]:
    """Report a tree whose schemaVersion these schemas do not describe.

    Validating a newer tree against v1alpha1 rules would bury the one useful
    fact under a cascade of bogus unknown-field reports, so stop instead and
    say the server is the authority.
    """
    factory_file = root / "factory.yaml"
    if _leaves_factory_root(factory_file, root):
        # Leave the report to validate_tree, which names it as a link.
        return None
    try:
        parsed = load_yaml(factory_file.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, YamlError):
        return None
    if not isinstance(parsed, dict):
        return None
    declared = parsed.get("schemaVersion")
    if not isinstance(declared, str) or declared.strip() in ("", SUPPORTED_SCHEMA_VERSION):
        return None
    return Problem(
        "factory.yaml",
        f"these bundled schemas describe {SUPPORTED_SCHEMA_VERSION}, not "
        f"{declared.strip()!r}, so this tree was not validated locally; check it "
        "with the server instead of downgrading schemaVersion",
        pointer="schemaVersion",
    )


def validate_tree(root: Path, store: SchemaStore) -> list[Problem]:
    problems: list[Problem] = []
    documents: dict[str, tuple[str, str, Any]] = {}
    seen_names: dict[tuple[str, str], str] = {}

    if not (root / "factory.yaml").is_file():
        return [Problem("factory.yaml", "factory.yaml is required at the Factory root")]

    unsupported = _unsupported_schema_version(root)
    if unsupported is not None:
        return [unsupported]

    for absolute in _resource_files(root):
        relative = absolute.relative_to(root).as_posix()
        kind, name = classify(relative)
        if kind in ("unrelated", "skill"):
            continue
        if kind == "invalid":
            problems.append(
                Problem(
                    relative,
                    "resource files must use factory.yaml, agents/<name>/agent.md, "
                    "automations/<name>/automation.md, runners/<name>.yaml, "
                    "or scorers/<name>/scorer.md",
                )
            )
            continue
        if kind in ("automation", "runner", "scorer"):
            previous = seen_names.get((kind, name))
            if previous is not None:
                problems.append(
                    Problem(relative, f'{kind} "{name}" is also declared by {previous}')
                )
                continue
            seen_names[(kind, name)] = relative

        if _leaves_factory_root(absolute, root):
            problems.append(
                Problem(
                    relative,
                    "resource file is a symlink, or resolves outside the Factory root, "
                    "and was not read. The server parses the repository tree, so it sees "
                    "the link itself rather than its target and cannot accept this "
                    "either. Replace it with a real file.",
                )
            )
            continue

        try:
            text = absolute.read_text(encoding="utf-8")
        except (OSError, UnicodeError) as error:
            problems.append(Problem(relative, f"could not read UTF-8 resource: {error}"))
            continue
        offset = 0
        body = ""
        try:
            if kind in ("agent", "automation", "scorer"):
                frontmatter, body, offset = split_frontmatter(text)
                parsed = load_yaml(frontmatter) if frontmatter.strip() else {}
            else:
                parsed = load_yaml(text)
        except YamlError as error:
            problems.append(Problem(relative, error.message, error.line + offset))
            continue

        if parsed is None:
            parsed = {}
        if not isinstance(parsed, dict):
            problems.append(Problem(relative, "document root must be a YAML mapping"))
            continue

        documents[relative] = (kind, name, parsed)
        schema = store.document(SCHEMA_BY_KIND[kind])
        for message in validate_instance(parsed, schema, store, SCHEMA_BY_KIND[kind]):
            pointer, _, detail = message.partition(": ")
            problems.append(Problem(relative, detail, pointer=pointer.lstrip("/").replace("/", ".")))
        if kind == "automation":
            problems.extend(_automation_semantics(relative, parsed))
        elif kind == "factory":
            problems.extend(_factory_semantics(relative, parsed))
        elif kind == "runner":
            problems.extend(_runner_semantics(relative, parsed))
        elif kind == "scorer":
            problems.extend(_scorer_semantics(relative, parsed, body))

    problems.extend(_validate_cross_file(documents))
    return problems


def _scorer_semantics(relative: str, parsed: dict[str, Any], body: str) -> list[Problem]:
    problems: list[Problem] = []
    if not body.strip():
        problems.append(Problem(relative, "the Markdown body is the rubric and must not be empty"))

    agents = parsed.get("agents")
    if isinstance(agents, list):
        normalized_agents = [value.strip() for value in agents if isinstance(value, str)]
        if len(set(normalized_agents)) != len(normalized_agents):
            problems.append(Problem(relative, "agent names must be unique after trimming", pointer="agents"))

    labels = parsed.get("labels")
    threshold = parsed.get("passingScore")
    numeric_scores: list[float] = []
    if isinstance(labels, list):
        seen_labels: set[str] = set()
        for index, label in enumerate(labels):
            if not isinstance(label, dict):
                continue
            value = label.get("value")
            if isinstance(value, str):
                normalized_value = value.strip()
                if normalized_value in seen_labels:
                    problems.append(
                        Problem(
                            relative,
                            f'duplicate label "{normalized_value}"',
                            pointer=f"labels.{index}.value",
                        )
                    )
                seen_labels.add(normalized_value)
            score = label.get("score")
            if isinstance(score, (int, float)) and not isinstance(score, bool):
                numeric_scores.append(float(score))
    if (
        isinstance(threshold, (int, float))
        and not isinstance(threshold, bool)
        and numeric_scores
    ):
        threshold_value = float(threshold)
        if not any(score >= threshold_value for score in numeric_scores):
            problems.append(
                Problem(
                    relative,
                    "at least one label score must be at or above passingScore",
                    pointer="passingScore",
                )
            )
        if not any(score < threshold_value for score in numeric_scores):
            problems.append(
                Problem(
                    relative,
                    "at least one label score must be below passingScore",
                    pointer="passingScore",
                )
            )

    sampling_rate = parsed.get("samplingRate")
    if isinstance(sampling_rate, (int, float)) and not isinstance(sampling_rate, bool):
        if float(sampling_rate) == 0:
            problems.append(
                Problem(
                    relative,
                    "samplingRate must not be 0; use enabled: false to stop scoring",
                    pointer="samplingRate",
                )
            )
    return problems


def _factory_semantics(relative: str, parsed: dict[str, Any]) -> list[Problem]:
    problems: list[Problem] = []
    alias = parsed.get("alias")
    if isinstance(alias, str):
        normalized_alias = alias.strip()
        if len(normalized_alias) > 60:
            problems.append(Problem(relative, "alias must not exceed 60 characters", pointer="alias"))
        if any(
            unicodedata.category(character)[:1] not in {"L", "N"} and character not in " _.-"
            for character in normalized_alias
        ):
            problems.append(
                Problem(
                    relative,
                    "alias may only contain letters, digits, spaces, '-', '_', and '.'",
                    pointer="alias",
                )
            )

    secrets = parsed.get("secrets")
    if isinstance(secrets, list):
        normalized = [value.strip() for value in secrets if isinstance(value, str)]
        if len(set(normalized)) != len(normalized):
            problems.append(
                Problem(relative, "secret names must be unique after trimming", pointer="secrets")
            )

    repositories = parsed.get("repositories")
    if isinstance(repositories, list):
        seen: set[tuple[str, str]] = set()
        for index, repository in enumerate(repositories):
            if not isinstance(repository, dict):
                continue
            owner, name = repository.get("owner"), repository.get("name")
            if not isinstance(owner, str) or not isinstance(name, str):
                continue
            key = (owner.strip(), name.strip())
            if key in seen:
                problems.append(
                    Problem(
                        relative,
                        f"duplicate repository {key[0]}/{key[1]} after trimming",
                        pointer=f"repositories.{index}",
                    )
                )
            seen.add(key)
    return problems


def _runner_semantics(relative: str, parsed: dict[str, Any]) -> list[Problem]:
    shape = parsed.get("instanceShape")
    platform = parsed.get("platform")
    os_name = platform.get("os", "linux") if isinstance(platform, dict) else "linux"
    if os_name != "linux" or not isinstance(shape, dict):
        return []
    problems: list[Problem] = []
    for field in ("vcpus", "memoryGb"):
        value = shape.get(field)
        if isinstance(value, int) and not isinstance(value, bool) and value > 0:
            if value & (value - 1):
                problems.append(
                    Problem(
                        relative,
                        f"{field} must be a power of two for Linux runners",
                        pointer=f"instanceShape.{field}",
                    )
                )
    return problems


_CRON_DESCRIPTORS = {
    "@yearly",
    "@annually",
    "@monthly",
    "@weekly",
    "@daily",
    "@midnight",
    "@hourly",
}
_DURATION_RE = re.compile(
    r"^[+-]?(?:0|(?:(?:\d+(?:\.\d*)?|\.\d+)(?:ns|us|µs|μs|ms|s|m|h))+)$"
)
_MONTH_NAMES = {
    "jan": 1,
    "feb": 2,
    "mar": 3,
    "apr": 4,
    "may": 5,
    "jun": 6,
    "jul": 7,
    "aug": 8,
    "sep": 9,
    "oct": 10,
    "nov": 11,
    "dec": 12,
}
_DAY_NAMES = {"sun": 0, "mon": 1, "tue": 2, "wed": 3, "thu": 4, "fri": 5, "sat": 6}


def _cron_number(value: str, names: Optional[dict[str, int]]) -> Optional[int]:
    if names is not None and value.lower() in names:
        return names[value.lower()]
    if not re.fullmatch(r"\d+", value):
        return None
    return int(value)


def _valid_cron_field(
    field: str, minimum: int, maximum: int, names: Optional[dict[str, int]] = None
) -> bool:
    for expression in filter(None, field.split(",")):
        parts = expression.split("/")
        if len(parts) > 2:
            return False
        base = parts[0]
        if len(parts) == 2 and (not parts[1].isdigit() or int(parts[1]) == 0):
            return False
        if base in {"*", "?"}:
            continue
        bounds = base.split("-")
        if len(bounds) > 2:
            return False
        start = _cron_number(bounds[0], names)
        end = _cron_number(bounds[-1], names)
        if start is None or end is None:
            return False
        if start < minimum or end > maximum or start > end:
            return False
    return bool(field)


def _valid_cron(expression: str) -> bool:
    expression = expression.strip()
    if expression in _CRON_DESCRIPTORS:
        return True
    if expression.startswith("@every "):
        return _DURATION_RE.fullmatch(expression[len("@every ") :]) is not None
    fields = expression.split()
    if len(fields) != 5:
        return False
    return all(
        validator
        for validator in (
            _valid_cron_field(fields[0], 0, 59),
            _valid_cron_field(fields[1], 0, 23),
            _valid_cron_field(fields[2], 1, 31),
            _valid_cron_field(fields[3], 1, 12, _MONTH_NAMES),
            _valid_cron_field(fields[4], 0, 6, _DAY_NAMES),
        )
    )


def _automation_semantics(relative: str, parsed: dict[str, Any]) -> list[Problem]:
    """Report filter values listed in both in and not_in.

    Such a filter can never match, so the server rejects it rather than
    persisting a silently dead subscription. JSON Schema cannot compare two
    sibling arrays, so the check lives here.
    """
    problems: list[Problem] = []
    triggers = parsed.get("triggers")
    if not isinstance(triggers, list):
        return problems
    schedule_keys: set[str] = set()
    for index, trigger in enumerate(triggers):
        if not isinstance(trigger, dict):
            continue
        schedule = trigger.get("schedule")
        if isinstance(schedule, dict):
            name = schedule.get("name")
            normalized_name = name.strip() if isinstance(name, str) else ""
            key = f"name:{normalized_name}" if normalized_name else "unnamed"
            if key in schedule_keys:
                detail = (
                    f'duplicate inline schedule name "{normalized_name}"'
                    if normalized_name
                    else "at most one inline schedule may omit name"
                )
                problems.append(
                    Problem(relative, detail, pointer=f"triggers.{index}.schedule")
                )
            schedule_keys.add(key)
            cron = schedule.get("cron")
            if isinstance(cron, str) and not _valid_cron(cron):
                problems.append(
                    Problem(
                        relative,
                        f"invalid cron expression {json.dumps(cron)}",
                        pointer=f"triggers.{index}.schedule.cron",
                    )
                )

        declared = trigger.get("filter")
        if not isinstance(declared, dict):
            continue
        provider, event = trigger.get("provider"), trigger.get("event")
        for field, matcher in declared.items():
            if not isinstance(matcher, dict):
                continue
            included = matcher.get("in")
            excluded = matcher.get("not_in")
            if not isinstance(included, list) or not isinstance(excluded, list):
                continue
            normalize = _matcher_normalizer(provider, event, field)
            excluded_keys = {normalize(value) for value in excluded}
            for value in included:
                if normalize(value) in excluded_keys:
                    problems.append(
                        Problem(
                            relative,
                            f"{json.dumps(value)} is present in, or equivalent to a value in, "
                            "both in and not_in, "
                            "so this filter can never match",
                            pointer=f"triggers.{index}.filter.{field}",
                        )
                    )
    return problems


def _matcher_normalizer(provider: Any, event: Any, field: str):
    lowercase_fields: set[tuple[str, str]] = {
        ("github", "assignees"),
        ("github", "authors"),
        ("github", "mentioned"),
        ("github", "reviewers"),
        ("github", "reviewer_teams"),
        ("github", "review_states"),
        ("github", "conclusions"),
        ("github", "workflows"),
        ("gitlab", "repos"),
        ("gitlab", "actions"),
        ("gitlab", "mentioned"),
        ("linear", "mentioned_user_ids"),
        ("linear", "labels"),
    }

    def normalize(value: Any) -> Any:
        if not isinstance(value, str):
            return value
        if provider == "github" and event == "push" and field == "branches":
            return value[len("refs/heads/") :] if value.startswith("refs/heads/") else value
        if provider == "slack" and field == "emojis":
            emoji = value.strip().strip(":")
            skin_tone = emoji.find("::skin-tone-")
            if skin_tone >= 0:
                emoji = emoji[:skin_tone]
            return emoji.lower()
        if field == "keywords" and provider in {"linear", "slack", "jira"}:
            return value.strip().lower()
        if (provider, field) in lowercase_fields:
            return value.lower()
        return value

    return normalize


def _validate_cross_file(documents: dict[str, tuple[str, str, Any]]) -> list[Problem]:
    """Check the tree-level rules that no single-document schema can express.

    Runner references are deliberately not checked: a name the tree does not
    declare legitimately resolves to an existing team runner on the server.
    """
    problems: list[Problem] = []
    agent_names: set[str] = set()
    main_agents: list[str] = []

    for relative, (kind, name, parsed) in documents.items():
        if kind == "agent":
            agent_names.add(name)
            if str(parsed.get("agentType", "")) in MAIN_AGENT_TYPES:
                main_agents.append(relative)

    if not main_agents:
        problems.append(
            Problem("factory.yaml", "exactly one Agent must declare agentType MAIN or FOREMAN")
        )
    elif len(main_agents) > 1:
        for relative in sorted(main_agents):
            problems.append(
                Problem(relative, "only one Agent may declare agentType MAIN or FOREMAN")
            )

    for relative, (kind, _, parsed) in documents.items():
        if kind == "automation":
            agent = parsed.get("agent")
            if isinstance(agent, str) and agent not in agent_names:
                problems.append(
                    Problem(relative, f'agent "{agent}" must name a declared Agent', pointer="agent")
                )
        elif kind == "scorer":
            agents = parsed.get("agents")
            if not isinstance(agents, list):
                continue
            for index, agent in enumerate(agents):
                if isinstance(agent, str) and agent.strip() not in agent_names:
                    problems.append(
                        Problem(
                            relative,
                            f'agent "{agent.strip()}" must name a declared Agent',
                            pointer=f"agents.{index}",
                        )
                    )
    return problems


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("root", nargs="?", default=".", help="Factory root containing factory.yaml")
    parser.add_argument("--json", action="store_true", help="emit machine-readable output")
    parser.add_argument(
        "--schemas",
        default=str(Path(__file__).resolve().parent.parent / "schemas"),
        help="directory holding the bundled JSON Schemas",
    )
    args = parser.parse_args()

    root = Path(args.root).resolve()
    store = SchemaStore(Path(args.schemas).resolve())
    problems = validate_tree(root, store)

    if args.json:
        print(json.dumps({"valid": not problems, "problems": [p.as_dict() for p in problems]}, indent=2))
    elif problems:
        print(f"{len(problems)} problem(s) in {root}:", file=sys.stderr)
        for problem in problems:
            print(f"  {problem.render()}", file=sys.stderr)
    else:
        print(f"{root}: factory files are valid against the v1alpha1 schemas")
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
