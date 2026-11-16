#!/usr/bin/env python3
"""Safely merge global file-based MCP configs without revealing their contents."""

from __future__ import annotations

import argparse
import copy
import hashlib
import json
import os
import re
import stat
import sys
import tempfile
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path
from typing import Any
from urllib.parse import parse_qsl, urlsplit

MAX_CONFIG_BYTES = 8 * 1024 * 1024
FINGERPRINT_VERSION = b"warp-tui-mcp-merge-v1\0"
PLACEHOLDER = re.compile(r"\$\{[A-Za-z_][A-Za-z0-9_]*\}|\{\{[^{}\r\n]+\}\}")
MANAGED_MARKERS = frozenset(
    {
        "warp_id",
        "installation_id",
        "installation_uuid",
        "gallery_data",
        "template",
        "variables",
        "variable_values",
    }
)
WRAPPER_PATHS: tuple[tuple[str, ...], ...] = (
    ("mcpServers",),
    ("mcp_servers",),
    ("servers",),
    ("mcp", "servers"),
)


class MergeError(Exception):
    """A sanitized error safe to emit without config data."""


@dataclass(frozen=True)
class ParsedConfig:
    document: dict[str, Any]
    servers: dict[str, Any]
    wrapper_path: tuple[str, ...]
    exists: bool
    raw: bytes
    mode: int | None


@dataclass(frozen=True)
class MergePlan:
    document: dict[str, Any]
    fingerprint: str
    source_server_count: int
    eligible_source_count: int
    destination_server_count: int
    add_count: int
    conflict_count: int
    skipped_sensitive_count: int
    skipped_reinstall_count: int
    result_server_count: int
    content_change_required: bool
    permission_update_required: bool

    @property
    def would_change(self) -> bool:
        return self.content_change_required or self.permission_update_required


def _path_value(document: dict[str, Any], path: tuple[str, ...]) -> Any:
    current: Any = document
    for segment in path:
        if not isinstance(current, dict) or segment not in current:
            return None
        current = current[segment]
    return current


def _set_path(
    document: dict[str, Any], path: tuple[str, ...], value: dict[str, Any]
) -> None:
    current = document
    for segment in path[:-1]:
        child = current.get(segment)
        if not isinstance(child, dict):
            child = {}
            current[segment] = child
        current = child
    if path:
        current[path[-1]] = value
    else:
        document.clear()
        document.update(value)


def _read_regular_file(path: Path, *, missing_ok: bool) -> tuple[bytes, int | None]:
    try:
        metadata = path.lstat()
    except FileNotFoundError:
        if missing_ok:
            return b"", None
        raise MergeError("source_config_unavailable")
    except OSError as error:
        raise MergeError("config_unavailable") from error

    if stat.S_ISLNK(metadata.st_mode):
        raise MergeError("config_symlink_rejected")
    if not stat.S_ISREG(metadata.st_mode):
        raise MergeError("config_not_regular_file")
    if metadata.st_size > MAX_CONFIG_BYTES:
        raise MergeError("config_too_large")

    flags = os.O_RDONLY
    if hasattr(os, "O_NOFOLLOW"):
        flags |= os.O_NOFOLLOW
    try:
        descriptor = os.open(path, flags)
        with os.fdopen(descriptor, "rb") as file:
            raw = file.read(MAX_CONFIG_BYTES + 1)
    except OSError as error:
        raise MergeError("config_unavailable") from error
    if len(raw) > MAX_CONFIG_BYTES:
        raise MergeError("config_too_large")
    return raw, stat.S_IMODE(metadata.st_mode)


def _decode_document(raw: bytes) -> dict[str, Any]:
    try:
        document = json.loads(raw.decode("utf-8"))
    except (UnicodeError, json.JSONDecodeError) as error:
        raise MergeError("invalid_json") from error
    if not isinstance(document, dict):
        raise MergeError("invalid_config_shape")
    return document


def _detect_wrapper(document: dict[str, Any]) -> tuple[tuple[str, ...], dict[str, Any]]:
    matches: list[tuple[tuple[str, ...], dict[str, Any]]] = []
    for wrapper_path in WRAPPER_PATHS:
        value = _path_value(document, wrapper_path)
        if value is not None:
            if not isinstance(value, dict):
                raise MergeError("invalid_wrapper_shape")
            matches.append((wrapper_path, value))

    if len(matches) > 1:
        raise MergeError("ambiguous_wrapper")
    if matches:
        return matches[0]

    # Warp accepts an unwrapped map. Require object-valued entries so unrelated
    # application JSON does not silently become an MCP config.
    if all(isinstance(value, dict) for value in document.values()):
        return (), document
    raise MergeError("unrecognized_wrapper")


def load_config(path: Path) -> ParsedConfig:
    raw, mode = _read_regular_file(path, missing_ok=True)
    exists = mode is not None
    if not exists:
        return ParsedConfig(
            document={},
            servers={},
            wrapper_path=("mcpServers",),
            exists=False,
            raw=b"",
            mode=None,
        )

    document = _decode_document(raw)
    wrapper_path, servers = _detect_wrapper(document)
    for value in servers.values():
        if not isinstance(value, dict):
            raise MergeError("invalid_server_definition")
    return ParsedConfig(
        document=document,
        servers=servers,
        wrapper_path=wrapper_path,
        exists=True,
        raw=raw,
        mode=mode,
    )


def _has_managed_marker(value: Any) -> bool:
    if isinstance(value, dict):
        for key, child in value.items():
            if key.casefold() in MANAGED_MARKERS or _has_managed_marker(child):
                return True
    elif isinstance(value, list):
        return any(_has_managed_marker(child) for child in value)
    return False


def _placeholder_backed(value: Any) -> bool:
    if not isinstance(value, str) or PLACEHOLDER.search(value) is None:
        return False
    residue = PLACEHOLDER.sub("", value)
    normalized_residue = "".join(
        character for character in residue.casefold() if character.isalnum()
    )
    return normalized_residue in {"", "basic", "bearer", "token"}


def _looks_sensitive_key(key: str) -> bool:
    normalized = "".join(
        character for character in key.casefold() if character.isalnum()
    )
    return any(
        marker in normalized
        for marker in (
            "password",
            "passwd",
            "secret",
            "token",
            "apikey",
            "authorization",
            "credential",
        )
    )


def _url_has_literal_credentials(value: str) -> bool:
    try:
        parsed = urlsplit(value)
        if parsed.username is not None or parsed.password is not None:
            return True
        return any(
            _looks_sensitive_key(key) and not _placeholder_backed(item)
            for key, item in parse_qsl(parsed.query, keep_blank_values=True)
        )
    except ValueError:
        return True


def _args_have_literal_credentials(arguments: Any) -> bool:
    if not isinstance(arguments, list):
        return False
    for index, argument in enumerate(arguments):
        if not isinstance(argument, str) or not argument.startswith("-"):
            continue
        flag, separator, inline_value = argument.partition("=")
        if not _looks_sensitive_key(flag):
            continue
        if separator and not _placeholder_backed(inline_value):
            return True
        if (
            not separator
            and index + 1 < len(arguments)
            and not _placeholder_backed(arguments[index + 1])
        ):
            return True
    return False


def _has_literal_sensitive_map(value: Any) -> bool:
    if isinstance(value, dict):
        for key, child in value.items():
            folded = key.casefold()
            if folded in {"env", "headers", "http_headers", "environment"}:
                if not isinstance(child, dict):
                    return True
                if any(not _placeholder_backed(item) for item in child.values()):
                    return True
            if folded == "args" and _args_have_literal_credentials(child):
                return True
            if folded in {"url", "serverurl"} and isinstance(child, str):
                if _url_has_literal_credentials(child):
                    return True
            if _looks_sensitive_key(folded):
                if isinstance(child, str) and not _placeholder_backed(child):
                    return True
            if _has_literal_sensitive_map(child):
                return True
    elif isinstance(value, list):
        return any(_has_literal_sensitive_map(child) for child in value)
    return False


def _validate_source_definition(definition: dict[str, Any]) -> None:
    command = definition.get("command")
    url = definition.get("url", definition.get("serverUrl"))
    if (command is None) == (url is None):
        raise MergeError("invalid_server_definition")
    if command is not None:
        if not isinstance(command, str) or not command:
            raise MergeError("invalid_server_definition")
        arguments = definition.get("args", [])
        if not isinstance(arguments, list) or not all(
            isinstance(argument, str) for argument in arguments
        ):
            raise MergeError("invalid_server_definition")
    if url is not None and (not isinstance(url, str) or not url):
        raise MergeError("invalid_server_definition")


def _classify_source(
    servers: dict[str, Any],
) -> tuple[dict[str, Any], int, int]:
    eligible: dict[str, Any] = {}
    skipped_sensitive = 0
    skipped_reinstall = 0
    for name, definition in servers.items():
        if _has_managed_marker(definition):
            skipped_reinstall += 1
        elif _has_literal_sensitive_map(definition):
            skipped_sensitive += 1
        else:
            _validate_source_definition(definition)
            eligible[name] = definition
    return eligible, skipped_sensitive, skipped_reinstall


def _fingerprint(source: bytes, destination: bytes, destination_exists: bool) -> str:
    digest = hashlib.sha256()
    digest.update(FINGERPRINT_VERSION)
    for label, raw in (
        (b"source\0", source),
        (
            b"destination-present\0" if destination_exists else b"destination-absent\0",
            destination,
        ),
    ):
        digest.update(label)
        digest.update(len(raw).to_bytes(8, "big"))
        digest.update(raw)
    return digest.hexdigest()


def build_plan(source: ParsedConfig, destination: ParsedConfig) -> MergePlan:
    eligible, skipped_sensitive, skipped_reinstall = _classify_source(source.servers)
    additions = {
        name: value
        for name, value in eligible.items()
        if name not in destination.servers
    }
    conflict_count = len(set(eligible).intersection(destination.servers))

    merged_servers = copy.deepcopy(destination.servers)
    for name, value in additions.items():
        merged_servers[name] = copy.deepcopy(value)

    if destination.exists:
        result_document = copy.deepcopy(destination.document)
        wrapper_path = destination.wrapper_path
    else:
        result_document = {}
        wrapper_path = source.wrapper_path
    _set_path(result_document, wrapper_path, merged_servers)

    permission_update_required = (
        destination.exists
        and destination.mode is not None
        and destination.mode & 0o077 != 0
    )
    return MergePlan(
        document=result_document,
        fingerprint=_fingerprint(source.raw, destination.raw, destination.exists),
        source_server_count=len(source.servers),
        eligible_source_count=len(eligible),
        destination_server_count=len(destination.servers),
        add_count=len(additions),
        conflict_count=conflict_count,
        skipped_sensitive_count=skipped_sensitive,
        skipped_reinstall_count=skipped_reinstall,
        result_server_count=len(merged_servers),
        content_change_required=bool(additions),
        permission_update_required=permission_update_required,
    )


def _serialized(document: dict[str, Any]) -> bytes:
    return (json.dumps(document, indent=2, ensure_ascii=False) + "\n").encode("utf-8")


def _write_restricted_file(path: Path, contents: bytes, *, exclusive: bool) -> None:
    flags = os.O_WRONLY | os.O_CREAT
    flags |= os.O_EXCL if exclusive else os.O_TRUNC
    if hasattr(os, "O_NOFOLLOW"):
        flags |= os.O_NOFOLLOW
    descriptor = os.open(path, flags, 0o600)
    try:
        os.fchmod(descriptor, 0o600)
        with os.fdopen(descriptor, "wb", closefd=False) as file:
            file.write(contents)
            file.flush()
            os.fsync(file.fileno())
    finally:
        os.close(descriptor)


def _backup_path(destination: Path) -> Path:
    stamp = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    for counter in range(1000):
        candidate = destination.with_name(
            f"{destination.name}.backup-{stamp}-{counter:03d}"
        )
        if not candidate.exists() and not candidate.is_symlink():
            return candidate
    raise MergeError("backup_name_unavailable")


def apply_plan(
    plan: MergePlan,
    destination: Path,
    destination_config: ParsedConfig,
) -> bool:
    """Apply a verified plan. Return whether a content backup was created."""

    if not plan.content_change_required:
        if plan.permission_update_required:
            try:
                os.chmod(destination, 0o600, follow_symlinks=False)
            except (NotImplementedError, OSError) as error:
                raise MergeError("permission_update_failed") from error
        return False

    try:
        destination.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
    except OSError as error:
        raise MergeError("destination_directory_unavailable") from error
    if destination.is_symlink():
        raise MergeError("config_symlink_rejected")

    stage_descriptor: int | None = None
    stage_path: Path | None = None
    try:
        stage_descriptor, stage_name = tempfile.mkstemp(
            prefix=f".{destination.name}.stage-", dir=destination.parent
        )
        stage_path = Path(stage_name)
        os.fchmod(stage_descriptor, 0o600)
        with os.fdopen(stage_descriptor, "wb", closefd=False) as file:
            file.write(_serialized(plan.document))
            file.flush()
            os.fsync(file.fileno())
        os.close(stage_descriptor)
        stage_descriptor = None

        backup_created = False
        if destination_config.exists:
            backup = _backup_path(destination)
            _write_restricted_file(backup, destination_config.raw, exclusive=True)
            backup_created = True

        os.replace(stage_path, destination)
        stage_path = None
        os.chmod(destination, 0o600, follow_symlinks=False)
        if hasattr(os, "O_DIRECTORY"):
            directory_fd = os.open(destination.parent, os.O_RDONLY | os.O_DIRECTORY)
            try:
                os.fsync(directory_fd)
            finally:
                os.close(directory_fd)
        return backup_created
    except MergeError:
        raise
    except OSError as error:
        raise MergeError("atomic_write_failed") from error
    finally:
        if stage_descriptor is not None:
            os.close(stage_descriptor)
        if stage_path is not None:
            try:
                stage_path.unlink()
            except FileNotFoundError:
                pass
            except OSError:
                pass


def _summary(
    plan: MergePlan, *, status: str, backup_created: bool = False
) -> dict[str, Any]:
    return {
        "status": status,
        "fingerprint": plan.fingerprint,
        "source_server_count": plan.source_server_count,
        "eligible_source_count": plan.eligible_source_count,
        "destination_server_count": plan.destination_server_count,
        "add_count": plan.add_count,
        "conflict_count": plan.conflict_count,
        "skipped_sensitive_count": plan.skipped_sensitive_count,
        "skipped_reinstall_count": plan.skipped_reinstall_count,
        "result_server_count": plan.result_server_count,
        "content_change_required": plan.content_change_required,
        "permission_update_required": plan.permission_update_required,
        "would_change": plan.would_change,
        "backup_created": backup_created,
    }


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Redacted destination-wins merge for Warp global MCP configs."
    )
    parser.add_argument("--source", required=True)
    parser.add_argument("--destination", required=True)
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--dry-run", action="store_true")
    mode.add_argument("--apply", action="store_true")
    parser.add_argument("--fingerprint")
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = parse_args(sys.argv[1:] if argv is None else argv)

    # Path("") aliases ".", so reject unavailable host context before any
    # filesystem lookup or path synthesis.
    if not args.source:
        print("error: source_path_unavailable", file=sys.stderr)
        return 2
    if not args.destination:
        print("error: destination_path_unavailable", file=sys.stderr)
        return 2
    if args.apply and not args.fingerprint:
        print("error: fingerprint_required", file=sys.stderr)
        return 2
    if args.dry_run and args.fingerprint:
        print("error: fingerprint_not_valid_for_dry_run", file=sys.stderr)
        return 2

    try:
        source = load_config(Path(args.source))
        destination = load_config(Path(args.destination))
        plan = build_plan(source, destination)
        if args.dry_run:
            print(json.dumps(_summary(plan, status="ready"), sort_keys=True))
            return 0
        if args.fingerprint != plan.fingerprint:
            raise MergeError("fingerprint_mismatch")
        backup_created = apply_plan(plan, Path(args.destination), destination)
        print(
            json.dumps(
                _summary(plan, status="applied", backup_created=backup_created),
                sort_keys=True,
            )
        )
        return 0
    except MergeError as error:
        print(f"error: {error}", file=sys.stderr)
        return 1
    except Exception:
        print("error: merge_failed", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
