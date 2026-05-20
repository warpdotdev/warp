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
import sys

# Map from changelog-draft.json category names to release JSON keys.
CATEGORY_MAP = {
    "NEW-FEATURE": "newFeatures",
    "IMPROVEMENT": "improvements",
    "BUG-FIX": "bugFixes",
    "OZ": "oz_updates",
    "IMAGE": "images",
}

REPO_URL = "https://github.com/warpdotdev/warp"


def format_entry(entry: dict) -> str:
    """Format a single changelog entry as a text line with a PR link.

    Includes external contributor attribution when applicable.
    """
    text = entry["text"]
    pr_number = entry["pr_number"]
    link = f"([#{pr_number}]({REPO_URL}/pull/{pr_number}))"

    attribution = ""
    if entry.get("is_external") and entry.get("author"):
        attribution = f" — @{entry['author']} ✨"

    return f"{text} {link}{attribution}"


def convert(draft: dict) -> dict:
    """Convert a changelog-draft.json dict to changelog-release.json dict."""
    release: dict[str, list[str]] = {
        "newFeatures": [],
        "improvements": [],
        "bugFixes": [],
        "images": [],
        "oz_updates": [],
    }

    for entry in draft.get("entries", []):
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
    args = parser.parse_args()

    with open(args.input) as f:
        draft = json.load(f)

    release = convert(draft)

    with open(args.output, "w") as f:
        json.dump(release, f, indent=2)
        f.write("\n")

    # Summary to stdout for CI logs
    for key, items in release.items():
        print(f"  {key}: {len(items)} entries")


if __name__ == "__main__":
    main()
