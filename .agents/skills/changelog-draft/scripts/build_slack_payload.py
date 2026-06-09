#!/usr/bin/env python3
"""Build a Slack Block Kit payload from release-pipeline changelog JSON."""

from __future__ import annotations

import argparse
import html
import json
import re
import sys
from pathlib import Path
from typing import Iterable
from urllib.parse import quote

MAX_SECTION_TEXT_LENGTH = 3000
MAX_MESSAGE_BLOCKS = 50

SECTION_ORDER = (
    ("newFeatures", "New Features"),
    ("improvements", "Improvements"),
    ("bugFixes", "Bug Fixes"),
    ("images", "Image"),
    # Keep the existing label stable for compatibility with recent Slack posts.
    ("oz_updates", "oz_updates"),
)

MARKDOWN_LINK_RE = re.compile(r"\[([^\]]+)\]\((https?://[^)\s]+)\)")
SLACK_LINK_URL_SAFE_CHARS = "/:?&=#%+~@!$'()*;,[]"


def escape_slack_text(text: str) -> str:
    """Escape Slack control characters in ordinary mrkdwn text."""
    return html.escape(text, quote=False)


def escape_slack_link_url(url: str) -> str:
    """Percent-encode Slack link delimiters before embedding a URL in mrkdwn."""
    return quote(url, safe=SLACK_LINK_URL_SAFE_CHARS)


def slack_link(url: str, label: str) -> str:
    """Build a Slack mrkdwn link from an already-validated URL and label."""
    return f"<{escape_slack_link_url(url)}|{escape_slack_text(label)}>"


def markdown_links_to_slack(text: str) -> str:
    """Convert standard Markdown links to Slack mrkdwn links."""
    parts: list[str] = []
    last_end = 0
    for match in MARKDOWN_LINK_RE.finditer(text):
        parts.append(escape_slack_text(text[last_end : match.start()]))
        label, url = match.groups()
        parts.append(slack_link(url, label))
        last_end = match.end()
    parts.append(escape_slack_text(text[last_end:]))
    return "".join(parts)


def slack_lines(changelog: dict) -> list[str]:
    """Render non-empty changelog sections as Slack mrkdwn lines."""
    lines: list[str] = []
    for key, title in SECTION_ORDER:
        values = changelog.get(key, [])
        if not isinstance(values, list) or not values:
            continue
        lines.append(f"*{title}*")
        for value in values:
            lines.append(f"  • {markdown_links_to_slack(str(value))}")
    return lines


def split_overlong_line(line: str) -> Iterable[str]:
    """Split a pathological line so every Slack section remains valid."""
    while len(line) > MAX_SECTION_TEXT_LENGTH - 1:
        yield line[: MAX_SECTION_TEXT_LENGTH - 1]
        line = line[MAX_SECTION_TEXT_LENGTH - 1 :]
    yield line


def chunk_lines(lines: list[str]) -> list[str]:
    """Split text into section-sized chunks while retaining copy boundaries."""
    chunks: list[str] = []
    buffer = ""
    for raw_line in lines:
        for line in split_overlong_line(raw_line):
            candidate = line if not buffer else f"{buffer}\n{line}"
            # Reserve a character for a trailing newline. Keeping it in each
            # section preserves a separator when Slack copies adjacent blocks.
            if len(candidate) + 1 > MAX_SECTION_TEXT_LENGTH:
                chunks.append(f"{buffer}\n")
                buffer = line
            else:
                buffer = candidate
    if buffer:
        chunks.append(f"{buffer}\n")
    return chunks


def artifact_link_block(markdown_artifact_url: str) -> dict | None:
    """Build a Slack block linking to the downloadable Markdown artifact."""
    if not markdown_artifact_url:
        return None
    return {
        "type": "section",
        "text": {
            "type": "mrkdwn",
            "text": (
                "Raw Markdown changelog: "
                f"{slack_link(markdown_artifact_url, 'Download raw Markdown changelog artifact')}"
            ),
        },
    }


def build_payload(
    changelog: dict, release_tag: str, markdown_artifact_url: str = ""
) -> dict:
    """Build a Block Kit message and reject payloads Slack cannot accept."""
    chunks = chunk_lines(slack_lines(changelog))
    if not chunks:
        return {"blocks": []}

    blocks = [
        {
            "type": "header",
            "text": {
                "type": "plain_text",
                "text": f"Changelog for {release_tag}",
            },
        }
    ]
    artifact_block = artifact_link_block(markdown_artifact_url)
    if artifact_block is not None:
        blocks.append(artifact_block)
    blocks.extend(
        {
            "type": "section",
            "expand": True,
            "text": {"type": "mrkdwn", "text": chunk},
        }
        for chunk in chunks
    )

    if len(blocks) > MAX_MESSAGE_BLOCKS:
        raise ValueError(
            "Slack payload would require "
            f"{len(blocks)} blocks, exceeding Slack's {MAX_MESSAGE_BLOCKS}-block limit"
        )
    return {"blocks": blocks}


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Build Slack payload JSON from changelog release JSON"
    )
    parser.add_argument("--input", required=True, help="Path to changelog JSON")
    parser.add_argument("--release-tag", required=True, help="Release tag header text")
    parser.add_argument(
        "--markdown-artifact-url",
        default="",
        help="Download URL for the raw Markdown changelog artifact",
    )
    parser.add_argument("--output", required=True, help="Payload JSON output path")
    args = parser.parse_args()

    with open(args.input) as f:
        changelog = json.load(f)

    payload = build_payload(changelog, args.release_tag, args.markdown_artifact_url)
    Path(args.output).write_text(json.dumps(payload, separators=(",", ":")) + "\n")
    print(f"Built Slack payload with {len(payload['blocks'])} blocks", file=sys.stderr)


if __name__ == "__main__":
    main()
