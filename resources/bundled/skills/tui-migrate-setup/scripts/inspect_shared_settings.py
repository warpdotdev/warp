#!/usr/bin/env python3
"""Inspect only GUI settings explicitly supported by both GUI and TUI."""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path
from typing import Any

try:
    import tomllib
except ModuleNotFoundError:  # pragma: no cover - exercised only on Python < 3.11.
    tomllib = None


class InspectionError(Exception):
    """A sanitized error safe to show without source file contents."""


def shared_setting_paths(schema: dict[str, Any]) -> list[tuple[str, ...]]:
    """Return settings whose property annotation includes both frontends."""

    paths: list[tuple[str, ...]] = []

    def visit(node: Any, prefix: tuple[str, ...]) -> None:
        if not isinstance(node, dict):
            return

        surfaces = node.get("x-warp-surfaces")
        if (
            prefix
            and isinstance(surfaces, list)
            and "gui" in surfaces
            and "tui" in surfaces
        ):
            # An annotated object is a setting value, not another hierarchy node.
            paths.append(prefix)
            return

        properties = node.get("properties")
        if not isinstance(properties, dict):
            return

        for key, child in properties.items():
            if isinstance(key, str):
                visit(child, (*prefix, key))

    visit(schema, ())
    return sorted(paths)


def nested_value(document: dict[str, Any], path: tuple[str, ...]) -> tuple[bool, Any]:
    current: Any = document
    for segment in path:
        if not isinstance(current, dict) or segment not in current:
            return False, None
        current = current[segment]
    return True, current


def _read_json_object(path: Path) -> dict[str, Any]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, json.JSONDecodeError) as error:
        raise InspectionError("schema_unavailable_or_invalid") from error
    if not isinstance(value, dict):
        raise InspectionError("schema_unavailable_or_invalid")
    return value


def _find_probe_path(
    value: Any, prefix: tuple[str, ...] = ()
) -> tuple[str, ...] | None:
    if isinstance(value, dict):
        if value.get("__warp_probe__") is True:
            return prefix
        for key, child in value.items():
            result = _find_probe_path(child, (*prefix, key))
            if result is not None:
                return result
    elif isinstance(value, list):
        for child in value:
            result = _find_probe_path(child, prefix)
            if result is not None:
                return result
    return None


def _table_header_path(line: str) -> tuple[bool, tuple[str, ...] | None]:
    stripped = line.strip()
    if not stripped.startswith("["):
        return False, None
    try:
        document = tomllib.loads(f"{stripped}\n__warp_probe__ = true\n")
    except tomllib.TOMLDecodeError:
        return False, None
    return True, _find_probe_path(document)


def _select_relevant_toml(contents: str, paths: list[tuple[str, ...]]) -> str:
    relevant_sections = {path[:-1] for path in paths}
    current_section: tuple[str, ...] | None = ()
    selected: list[str] = []

    for line in contents.splitlines(keepends=True):
        is_header, header_path = _table_header_path(line)
        if is_header:
            current_section = header_path
        if current_section in relevant_sections:
            selected.append(line)

    return "".join(selected)


def _read_toml_object(
    path: Path,
    paths: list[tuple[str, ...]],
    *,
    missing_ok: bool,
    role: str,
) -> dict[str, Any]:
    if path.is_symlink():
        raise InspectionError("settings_symlink_rejected")
    if missing_ok and not path.exists():
        return {}
    try:
        contents = path.read_text(encoding="utf-8")
        selected = _select_relevant_toml(contents, paths)
        value = tomllib.loads(selected) if selected else {}
    except (OSError, tomllib.TOMLDecodeError) as error:
        raise InspectionError(f"{role}_settings_unavailable_or_invalid") from error
    if not isinstance(value, dict):
        raise InspectionError(f"{role}_settings_unavailable_or_invalid")
    return value


def inspect_settings(
    schema: dict[str, Any],
    source: dict[str, Any],
    destination: dict[str, Any],
) -> dict[str, Any]:
    settings: list[dict[str, Any]] = []
    for path in shared_setting_paths(schema):
        source_present, source_value = nested_value(source, path)
        if not source_present:
            continue

        destination_present, destination_value = nested_value(destination, path)
        if not destination_present:
            state = "missing_in_tui"
        elif destination_value == source_value:
            state = "already_equal"
        else:
            state = "destination_conflict"

        item: dict[str, Any] = {
            "path": ".".join(path),
            "source_value": source_value,
            "destination_present": destination_present,
            "state": state,
        }
        if destination_present:
            item["destination_value"] = destination_value
        settings.append(item)

    return {
        "eligible_configured_count": len(settings),
        "settings": settings,
    }


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Inspect GUI settings declared for both Warp GUI and TUI."
    )
    parser.add_argument("--schema", required=True)
    parser.add_argument("--source", required=True)
    parser.add_argument("--destination", required=True)
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = parse_args(sys.argv[1:] if argv is None else argv)

    # Check raw strings before Path construction: Path("") means ".", which must
    # never become an accidental fallback for an unavailable GUI source profile.
    if not args.source:
        print("error: source_path_unavailable", file=sys.stderr)
        return 2
    if not args.schema or not args.destination:
        print("error: required_path_unavailable", file=sys.stderr)
        return 2
    if tomllib is None:
        print("error: python_3_11_or_newer_required", file=sys.stderr)
        return 2

    try:
        schema = _read_json_object(Path(args.schema))
        paths = shared_setting_paths(schema)
        source = _read_toml_object(
            Path(args.source),
            paths,
            missing_ok=True,
            role="source",
        )
        destination = _read_toml_object(
            Path(args.destination),
            paths,
            missing_ok=True,
            role="destination",
        )
        result = inspect_settings(schema, source, destination)
    except InspectionError as error:
        print(f"error: {error}", file=sys.stderr)
        return 1
    except Exception:
        print("error: inspection_failed", file=sys.stderr)
        return 1

    print(json.dumps(result, sort_keys=True, default=str))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
