#!/usr/bin/env python3
"""Validate a review.json artifact against an annotated PR diff.

This script is packaged with the review-pr skill and must work when the skill
is copied into a consuming repository without the full oz-for-oss source tree.
Keep it self-contained: do not import helpers from the repository package.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Any, TypedDict


class ReviewComment(TypedDict, total=False):
    """Normalized review comment accepted by GitHub's create-review API."""

    path: str
    line: int
    side: str
    body: str
    start_line: int
    start_side: str


@dataclass(frozen=True)
class ReviewValidationResult:
    """Validated review fields plus any comment-location errors."""

    body: str
    comments: list[ReviewComment]
    errors: list[str]


SUGGESTION_BLOCK_PATTERN = re.compile(
    r"```suggestion[^\n]*\r?\n(?P<content>.*?)\r?\n```",
    re.DOTALL,
)
ANNOTATED_OLD_PATTERN = re.compile(r"^\[OLD:(?P<old>\d+)\] ?(?P<text>.*)$")
ANNOTATED_NEW_PATTERN = re.compile(r"^\[NEW:(?P<new>\d+)\] ?(?P<text>.*)$")
ANNOTATED_CONTEXT_PATTERN = re.compile(
    r"^\[OLD:(?P<old>\d+),NEW:(?P<new>\d+)\] ?(?P<text>.*)$"
)


def normalize_review_path(value: Any) -> str:
    path = str(value or "").strip()
    path = re.sub(r"^(a/|b/|\./)", "", path)
    return path


def build_diff_maps_from_annotated_diff(
    diff_text: str,
) -> tuple[dict[str, dict[str, set[int]]], dict[str, dict[str, dict[int, str]]]]:
    """Build validation maps from the annotated diff shown to review agents."""
    diff_line_map: dict[str, dict[str, set[int]]] = {}
    diff_content_map: dict[str, dict[str, dict[int, str]]] = {}
    current_path = ""
    old_path = ""

    def ensure_path(path: str) -> None:
        diff_line_map.setdefault(path, {"LEFT": set(), "RIGHT": set()})
        diff_content_map.setdefault(path, {"LEFT": {}, "RIGHT": {}})

    for raw_line in diff_text.splitlines():
        if raw_line.startswith("diff --git "):
            current_path = ""
            old_path = ""
            continue
        if raw_line.startswith("--- "):
            candidate = raw_line[4:].strip()
            old_path = (
                "" if candidate == "/dev/null" else normalize_review_path(candidate)
            )
            continue
        if raw_line.startswith("+++ "):
            candidate = raw_line[4:].strip()
            if candidate == "/dev/null":
                current_path = old_path
            else:
                current_path = normalize_review_path(candidate)
            if current_path:
                ensure_path(current_path)
            continue
        if not current_path:
            continue
        old_match = ANNOTATED_OLD_PATTERN.match(raw_line)
        if old_match:
            line = int(old_match.group("old"))
            text = old_match.group("text")
            diff_line_map[current_path]["LEFT"].add(line)
            diff_content_map[current_path]["LEFT"][line] = text
            continue
        new_match = ANNOTATED_NEW_PATTERN.match(raw_line)
        if new_match:
            line = int(new_match.group("new"))
            text = new_match.group("text")
            diff_line_map[current_path]["RIGHT"].add(line)
            diff_content_map[current_path]["RIGHT"][line] = text
            continue
        context_match = ANNOTATED_CONTEXT_PATTERN.match(raw_line)
        if context_match:
            old_line = int(context_match.group("old"))
            new_line = int(context_match.group("new"))
            text = context_match.group("text")
            diff_line_map[current_path]["LEFT"].add(old_line)
            diff_line_map[current_path]["RIGHT"].add(new_line)
            diff_content_map[current_path]["LEFT"][old_line] = text
            diff_content_map[current_path]["RIGHT"][new_line] = text

    return diff_line_map, diff_content_map


def _extract_suggestion_blocks(body: str | None) -> list[list[str]]:
    blocks: list[list[str]] = []
    for match in SUGGESTION_BLOCK_PATTERN.finditer(body or ""):
        content = match.group("content")
        lines = [line.rstrip("\r") for line in content.split("\n")]
        blocks.append(lines)
    return blocks


def _validate_suggestion_blocks(
    comment: dict[str, Any],
    diff_content_map: dict[str, dict[str, dict[int, str]]],
) -> list[str]:
    errors: list[str] = []
    body = comment.get("body") or ""
    blocks = _extract_suggestion_blocks(body)
    if not blocks:
        return errors

    path = comment.get("path") or ""
    side = comment.get("side") or "RIGHT"
    start_side = comment.get("start_side") or side
    line_no = comment.get("line")
    if not isinstance(line_no, int):
        return errors
    start_line = comment.get("start_line") or line_no
    content_for_start_side = diff_content_map.get(path, {}).get(start_side, {})
    content_for_end_side = diff_content_map.get(path, {}).get(side, {})

    for block_index, block_lines in enumerate(blocks):
        if not block_lines or block_lines == [""]:
            continue
        prev_context = content_for_start_side.get(start_line - 1)
        next_context = content_for_end_side.get(line_no + 1)
        first_line = block_lines[0]
        last_line = block_lines[-1]
        if prev_context is not None and first_line == prev_context:
            errors.append(
                f"suggestion block {block_index} duplicates the context line immediately above "
                f"`start_line` ({start_line - 1}); that line is not replaced and will appear twice after the suggestion is applied"
            )
        if next_context is not None and last_line == next_context:
            errors.append(
                f"suggestion block {block_index} duplicates the context line immediately below "
                f"`line` ({line_no + 1}); that line is not replaced and will appear twice after the suggestion is applied"
            )
    return errors


def validate_review_payload(
    review: Any,
    diff_line_map: dict[str, dict[str, set[int]]],
    diff_content_map: dict[str, dict[str, dict[int, str]]] | None = None,
) -> ReviewValidationResult:
    """Validate a review.json payload against the annotated PR diff."""
    if not isinstance(review, dict):
        raise ValueError("Review payload must be a JSON object.")

    raw_body = review.get("body")
    if raw_body is None:
        raw_body = review.get("summary") or ""
    if not isinstance(raw_body, str):
        raise ValueError("Review payload `body` must be a string.")

    raw_comments = review.get("comments") or []
    if not isinstance(raw_comments, list):
        raise ValueError("Review payload `comments` must be a list.")

    normalized_comments: list[ReviewComment] = []
    errors: list[str] = []

    for index, raw_comment in enumerate(raw_comments):
        if not isinstance(raw_comment, dict):
            errors.append(f"`comments[{index}]` must be an object.")
            continue

        path = normalize_review_path(raw_comment.get("path"))
        line = raw_comment.get("line")
        body_value = raw_comment.get("body")
        body = body_value.strip() if isinstance(body_value, str) else ""
        side = raw_comment.get("side")

        if not path:
            errors.append(f"`comments[{index}]` is missing `path`.")
            continue
        if path not in diff_line_map:
            errors.append(
                f"`comments[{index}]` references `{path}`, which is not part of the PR diff. Move that feedback to top-level `body` instead."
            )
            continue
        if not isinstance(line, int) or line <= 0:
            errors.append(
                f"`comments[{index}]` for `{path}` must include a positive integer `line`."
            )
            continue
        if side not in {"LEFT", "RIGHT"}:
            errors.append(
                f"`comments[{index}]` for `{path}:{line}` must include `side` set to `LEFT` or `RIGHT`."
            )
            continue
        if not body:
            errors.append(f"`comments[{index}]` for `{path}` is missing `body`.")
            continue

        allowed_lines = diff_line_map[path][side]
        if line not in allowed_lines:
            errors.append(
                f"`comments[{index}]` references `{path}:{line}` on `{side}`, which is not commentable in the PR diff."
            )
            continue

        normalized_comment: ReviewComment = {
            "path": path,
            "line": line,
            "side": side,
            "body": body,
        }

        if "start_line" in raw_comment and raw_comment.get("start_line") is not None:
            start_line = raw_comment.get("start_line")
            if not isinstance(start_line, int) or start_line <= 0:
                errors.append(
                    f"`comments[{index}]` for `{path}` has invalid `start_line`; it must be a positive integer."
                )
                continue
            start_side = raw_comment.get("start_side")
            if start_side not in {"LEFT", "RIGHT"}:
                errors.append(
                    f"`comments[{index}]` for `{path}` has `start_line` but is missing `start_side`; set `start_side` to `LEFT` or `RIGHT`."
                )
                continue
            if start_side == side and start_line >= line:
                errors.append(
                    f"`comments[{index}]` for `{path}` has invalid `start_line`; when `start_side` matches `side`, it must be smaller than `line`."
                )
                continue
            if start_line not in diff_line_map[path][start_side]:
                errors.append(
                    f"`comments[{index}]` references `{path}:{start_line}` on `{start_side}` as `start_line`, which is not commentable in the PR diff."
                )
                continue
            normalized_comment["start_line"] = start_line
            normalized_comment["start_side"] = start_side
        elif raw_comment.get("start_side") is not None:
            errors.append(
                f"`comments[{index}]` for `{path}:{line}` has `start_side` without `start_line`."
            )
            continue

        if diff_content_map is not None:
            suggestion_errors = _validate_suggestion_blocks(
                normalized_comment, diff_content_map
            )
            if suggestion_errors:
                for err in suggestion_errors:
                    errors.append(
                        f"`comments[{index}]` for `{path}:{line}` on `{side}` has an invalid suggestion block: {err}."
                    )
                continue

        normalized_comments.append(normalized_comment)

    return ReviewValidationResult(
        body=raw_body.strip(),
        comments=normalized_comments,
        errors=errors,
    )


def _load_json(path: Path) -> Any:
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except FileNotFoundError:
        raise SystemExit(f"review validation failed: {path} does not exist")
    except json.JSONDecodeError as exc:
        raise SystemExit(f"review validation failed: {path} is invalid JSON: {exc}")


def _validate_verdict(payload: Any) -> list[str]:
    if not isinstance(payload, dict):
        return ["review.json must decode to a JSON object."]
    verdict = payload.get("verdict")
    if verdict not in {"APPROVE", "REJECT"}:
        return ['`verdict` must be exactly "APPROVE" or "REJECT".']
    return []


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Validate review.json comments against annotated pr_diff.txt."
    )
    parser.add_argument(
        "--review-json",
        default="review.json",
        type=Path,
        help="Path to the review.json artifact to validate.",
    )
    parser.add_argument(
        "--diff",
        default="pr_diff.txt",
        type=Path,
        help="Path to the annotated PR diff consumed during review.",
    )
    args = parser.parse_args()

    payload = _load_json(args.review_json)
    try:
        diff_text = args.diff.read_text(encoding="utf-8")
    except FileNotFoundError:
        print(f"review validation failed: {args.diff} does not exist", file=sys.stderr)
        return 1

    diff_line_map, diff_content_map = build_diff_maps_from_annotated_diff(diff_text)
    result = validate_review_payload(payload, diff_line_map, diff_content_map)
    errors = _validate_verdict(payload) + result.errors
    if errors:
        print("review validation failed:", file=sys.stderr)
        for error in errors:
            print(f"- {error}", file=sys.stderr)
        return 1

    print(
        "review validation passed: "
        f"{len(result.comments)} inline comment(s), {len(diff_line_map)} diff file(s)"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
