#!/usr/bin/env python3
"""Resolve the release comparison range for the changelog-draft skill.

Determines the previous release cut (base tag) given a release tag and channel.
Release tags follow: v0.YYYY.MM.DD.HH.MM.<channel>_NN where _NN is the RC/hotfix
number within a release cut. Multiple tags can share the same date-time prefix
(e.g. _00, _01, _02 are all part of one release cut).

The base tag is the _00 tag of the *previous* release cut — meaning a different
date-time prefix — not another tag from the same cut.

Usage:
    python3 resolve_release_range.py --release-tag <tag> --channel <stable|preview|dev> \\
        [--repo-dir <path>]

Outputs JSON to stdout on success (no logs to stdout):
    {"base": "<prev_cut_00>", "head": "<release_tag>", "range": "<prev_cut_00>..<release_tag>"}

Exits non-zero with a concise stderr message on:
    - Invalid tag format
    - Channel mismatch between tag and --channel
    - Head tag not present in repo
    - No previous cut found
"""

import argparse
import json
import re
import subprocess
import sys

# Tag format: v0.YYYY.MM.DD.HH.MM.<channel>_NN
TAG_RE = re.compile(
    r"^(v0)\.(\d{4})\.(\d{2})\.(\d{2})\.(\d{2})\.(\d{2})\.(stable|preview|dev)_(\d+)$"
)
VALID_CHANNELS = frozenset({"stable", "preview", "dev"})


def parse_tag(tag: str) -> dict | None:
    """Parse a release tag into its components.

    Returns a dict with keys: datetime_prefix, channel, nn, cut_prefix.
    Returns None if the tag does not match the expected format.
    """
    m = TAG_RE.match(tag)
    if not m:
        return None
    # datetime_prefix: YYYY.MM.DD.HH.MM — lexicographically sortable
    datetime_prefix = f"{m.group(2)}.{m.group(3)}.{m.group(4)}.{m.group(5)}.{m.group(6)}"
    channel = m.group(7)
    return {
        "datetime_prefix": datetime_prefix,
        "channel": channel,
        "nn": int(m.group(8)),
        # Full prefix without _NN: v0.YYYY.MM.DD.HH.MM.channel
        "cut_prefix": f"v0.{datetime_prefix}.{channel}",
    }


def tag_exists(tag: str, repo_dir: str) -> bool:
    """Return True if the tag exists in the given git repository."""
    result = subprocess.run(
        ["git", "tag", "--list", tag],
        capture_output=True,
        text=True,
        cwd=repo_dir,
    )
    if result.returncode != 0:
        return False
    return any(t.strip() == tag for t in result.stdout.splitlines())


def list_cut_tags(channel: str, repo_dir: str) -> list[str]:
    """List all _00 cut tags for the channel, sorted descending by version.

    Only _00 tags are cut points; _01, _02, etc. are hotfixes of the same cut.
    """
    result = subprocess.run(
        [
            "git",
            "tag",
            "--list",
            f"v0.*.{channel}_00",
            "--sort=-version:refname",
        ],
        capture_output=True,
        text=True,
        cwd=repo_dir,
    )
    if result.returncode != 0:
        return []
    return [t.strip() for t in result.stdout.splitlines() if t.strip()]


def find_base_tag(release_tag: str, channel: str, repo_dir: str) -> str:
    """Find the _00 base tag of the previous release cut.

    The base is the greatest _00 cut tag whose date-time prefix is strictly
    earlier than the release tag's date-time prefix.

    Raises SystemExit on validation errors (invalid format, channel mismatch,
    missing head tag, no previous cut).
    """
    parsed = parse_tag(release_tag)
    if parsed is None:
        print(
            f"error: invalid tag format '{release_tag}'; "
            "expected v0.YYYY.MM.DD.HH.MM.<channel>_NN "
            "(e.g. v0.2026.05.06.09.12.stable_00)",
            file=sys.stderr,
        )
        sys.exit(1)

    if parsed["channel"] != channel:
        print(
            f"error: channel mismatch: tag '{release_tag}' specifies channel "
            f"'{parsed['channel']}', but --channel='{channel}' was given",
            file=sys.stderr,
        )
        sys.exit(1)

    if not tag_exists(release_tag, repo_dir):
        print(
            f"error: head tag '{release_tag}' does not exist in the repository "
            f"at '{repo_dir}'",
            file=sys.stderr,
        )
        sys.exit(1)

    cut_tags = list_cut_tags(channel, repo_dir)
    if not cut_tags:
        print(
            f"error: no _00 cut tags found for channel '{channel}' in '{repo_dir}'",
            file=sys.stderr,
        )
        sys.exit(1)

    release_datetime = parsed["datetime_prefix"]

    # Iterate in descending version order; return the first tag that is
    # strictly earlier than the release cut.
    for tag in cut_tags:
        p = parse_tag(tag)
        if p is None:
            continue
        # Skip if same release cut (date-time prefix matches)
        if p["datetime_prefix"] == release_datetime:
            continue
        # Skip cuts that are later than the release tag (e.g. when looking up
        # a _01 hotfix, the next release's _00 must not become the base)
        if p["datetime_prefix"] > release_datetime:
            continue
        # This is the greatest prior _00 cut with a different prefix
        return tag

    print(
        f"error: no previous release cut found before '{release_tag}' "
        f"for channel '{channel}'",
        file=sys.stderr,
    )
    sys.exit(1)


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Resolve the release comparison range for changelog generation"
    )
    parser.add_argument(
        "--release-tag",
        required=True,
        help=(
            "The release tag to generate the changelog for "
            "(e.g. v0.2026.05.06.09.12.stable_00)"
        ),
    )
    parser.add_argument(
        "--channel",
        required=True,
        choices=sorted(VALID_CHANNELS),
        help="Release channel: dev, preview, or stable",
    )
    parser.add_argument(
        "--repo-dir",
        default=".",
        help="Path to the git repository (default: current directory)",
    )
    args = parser.parse_args()

    base_tag = find_base_tag(args.release_tag, args.channel, args.repo_dir)

    output = {
        "base": base_tag,
        "head": args.release_tag,
        "range": f"{base_tag}..{args.release_tag}",
    }
    json.dump(output, sys.stdout, indent=2)
    print()  # trailing newline


if __name__ == "__main__":
    main()
