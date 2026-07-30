#!/usr/bin/env python3
"""Convert changelog-draft.json to the release-pipeline-compatible changelog-release.json.

Reads the audit artifact produced by the changelog-draft skill and emits the
flat JSON structure consumed by the create_release workflow (Slack payload
builder + in-app changelog.json step).

Usage:
    python3 convert_to_release_json.py --input <changelog-draft.json> --output <changelog-release.json>

The output schema:
    {
      "newFeatures": ["..."],
      "improvements": ["..."],
      "bugFixes": ["..."],
      "images": ["..."],
      "oz_updates": ["..."]
    }
"""

import argparse
import json
import re
import subprocess
import sys

# Map from changelog-draft.json category names to release JSON keys.
CATEGORY_MAP = {
    "NEW-FEATURE": "newFeatures",
    "IMPROVEMENT": "improvements",
    "BUG-FIX": "bugFixes",
    "OZ": "oz_updates",
    "IMAGE": "images",
}

INTERNAL_AUTHOR_ASSOCIATIONS = frozenset({"COLLABORATOR", "MEMBER", "OWNER"})
PR_URL_RE = re.compile(r"^https://github\.com/([^/]+)/([^/]+)/pull/(\d+)$")


def github_profile_link(username: str) -> str:
    """Format a GitHub username as a markdown profile link."""
    return f"[@{username}](https://github.com/{username})"


def is_bot_author(author: str, entry: dict) -> bool:
    """Return whether an entry author is a bot or GitHub App account."""
    return (
        bool(entry.get("author_is_bot"))
        or author.endswith("[bot]")
        or author.startswith("app/")
    )


def fetch_author_metadata(entry: dict) -> dict:
    """Fetch author association metadata for an entry's PR URL via gh api.

    This is a best-effort safety net for older or incomplete draft artifacts.
    If the lookup fails, return the original entry unchanged.
    """
    if entry.get("author_association") or entry.get("author_is_bot"):
        return entry

    url = entry.get("url") or entry.get("pr_url") or ""
    match = PR_URL_RE.match(url)
    if match is None:
        return entry

    owner, repo, number = match.groups()
    result = subprocess.run(
        ["gh", "api", f"repos/{owner}/{repo}/pulls/{number}"],
        capture_output=True,
        text=True,
        check=False,
    )
    if result.returncode != 0 or not result.stdout:
        return entry

    try:
        data = json.loads(result.stdout)
    except json.JSONDecodeError:
        return entry

    enriched = dict(entry)
    enriched["author_association"] = data.get("author_association", "")
    user = data.get("user") if isinstance(data.get("user"), dict) else {}
    enriched["author_is_bot"] = user.get("type") == "Bot"
    return enriched


def should_attribute_author(entry: dict) -> bool:
    """Return whether a changelog entry should credit its author publicly."""
    author = entry.get("author")
    if not entry.get("is_external") or not author:
        return False
    if is_bot_author(author, entry):
        return False

    author_association = str(entry.get("author_association", "")).upper()
    if author_association in INTERNAL_AUTHOR_ASSOCIATIONS:
        return False

    return True


def format_entry(entry: dict) -> str:
    """Format a single changelog entry as a text line with a PR link.

    Includes external contributor attribution when applicable.
    """
    text = entry["text"]
    pr_number = entry.get("pr_number") or entry.get("number")
    url = entry.get("url") or entry.get("pr_url")

    link = ""
    if url and pr_number:
        link = f" ([#{pr_number}]({url}))"

    attribution = ""
    if should_attribute_author(entry):
        attribution = f" — {github_profile_link(entry['author'])} ✨"
    return f"{text}{link}{attribution}"


def convert(draft: dict, *, resolve_author_metadata: bool = False) -> dict:
    """Convert a changelog-draft.json dict to changelog-release.json dict."""
    release: dict[str, list[str]] = {
        "newFeatures": [],
        "improvements": [],
        "bugFixes": [],
        "images": [],
        "oz_updates": [],
    }

    for raw_entry in draft.get("entries", []):
        entry = fetch_author_metadata(raw_entry) if resolve_author_metadata else raw_entry
        category = entry.get("category", "")
        release_key = CATEGORY_MAP.get(category)
        if release_key is None:
            continue

        if category == "IMAGE":
            # IMAGE entries store a URL in "text" — pass through directly.
            release["images"].append(entry["text"])
        else:
            release[release_key].append(format_entry(entry))

    return release


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Convert changelog-draft.json to changelog-release.json"
    )
    parser.add_argument(
        "--input",
        required=True,
        help="Path to changelog-draft.json",
    )
    parser.add_argument(
        "--output",
        required=True,
        help="Path to write changelog-release.json",
    )
    parser.add_argument(
        "--resolve-author-metadata",
        action="store_true",
        help=(
            "Best-effort lookup of missing PR author association metadata via gh api "
            "before rendering contributor attribution"
        ),
    )
    args = parser.parse_args()

    with open(args.input) as f:
        draft = json.load(f)

    release = convert(draft, resolve_author_metadata=args.resolve_author_metadata)

    with open(args.output, "w") as f:
        json.dump(release, f, indent=2)
        f.write("\n")

    # Summary to stdout for CI logs
    for key, items in release.items():
        print(f"  {key}: {len(items)} entries")


if __name__ == "__main__":
    main()
