#!/usr/bin/env python3
"""Add updates that impact TUI users to legacy changelog JSON.

Preview and dev releases use the legacy changelog action, which only parses
explicit PR-body markers. This post-processor reuses the normalized PR metadata
collector used by stable releases, copies explicit TUI-impact entries, and moves
commit-labeled TUI-only entries out of the regular changelog buckets.
"""

import argparse
import json
import re
import subprocess

from fetch_prs import collect_prs

REGULAR_CATEGORY_KEYS = {
    "NEW-FEATURE": "newFeatures",
    "IMPROVEMENT": "improvements",
    "BUG-FIX": "bugFixes",
}
TUI_TOKEN_RE = re.compile(r"(?<![A-Za-z0-9])TUI(?![A-Za-z0-9])", re.IGNORECASE)
TRAILING_PR_RE = re.compile(r"\s*\(#\d+\)\s*$")
LEADING_TICKET_RE = re.compile(r"^\s*\[[A-Z]+-\d+\]\s*")
TUI_PREFIX_RE = re.compile(
    r"""(?ix)
    ^\s*
    (?:
        (?:\[TUI\]|TUI)\s*(?::|-)\s*
        |
        (?:feat|fix|chore|refactor|perf|test|docs)\s*\(\s*TUI\s*\)\s*:\s*
    )
    """
)


def run(cmd: list[str]) -> str:
    result = subprocess.run(cmd, capture_output=True, text=True, check=True)
    return result.stdout.strip()


def previous_release_cut(release_tag: str, channel: str) -> str:
    """Find the _00 tag from the previous release date."""
    release_date_prefix = release_tag.rsplit("_", 1)[0]
    tags = run(
        [
            "git",
            "tag",
            "--list",
            f"v0.*.{channel}_00",
            "--sort=-version:refname",
        ]
    )
    for tag in tags.splitlines():
        if tag.rsplit("_", 1)[0] != release_date_prefix:
            return tag
    raise ValueError(f"could not find a previous {channel} release cut")


def is_tui_subject(subject: str) -> bool:
    return bool(TUI_TOKEN_RE.search(subject))


def normalize_subject(subject: str) -> str:
    """Turn a TUI-labeled commit subject into compact changelog text."""
    text = TRAILING_PR_RE.sub("", subject).strip()
    text = LEADING_TICKET_RE.sub("", text)
    text = TUI_PREFIX_RE.sub("", text).strip()
    if text:
        text = text[0].upper() + text[1:]
    return text


def remove_first(items: list, value: str) -> None:
    try:
        items.remove(value)
    except ValueError:
        pass


def append_unique(items: list[str], value: str) -> None:
    if value and value not in items:
        items.append(value)


def add_tui_updates(changelog: dict, prs: list[dict]) -> dict:
    """Return a changelog with entries that impact TUI in tui_updates."""
    tui_updates = list(changelog.get("tui_updates") or [])

    for pr in prs:
        explicit_entries = pr.get("explicit_entries") or []
        explicit_categories = {
            entry.get("category", "") for entry in explicit_entries
        }
        if "NONE" in explicit_categories:
            continue

        regular_entries = [
            entry
            for entry in explicit_entries
            if entry.get("category") in REGULAR_CATEGORY_KEYS
        ]
        explicit_tui_entries = [
            entry for entry in explicit_entries if entry.get("category") == "TUI"
        ]

        if explicit_tui_entries:
            selected_text = [
                entry.get("text", "").strip() for entry in explicit_tui_entries
            ]
            move_regular_entries = False
        else:
            subject = pr.get("commit_subject") or pr.get("title") or ""
            title = pr.get("title") or ""
            tui_headline = subject if is_tui_subject(subject) else title
            if "OZ" in explicit_categories or not is_tui_subject(tui_headline):
                continue
            selected_text = [
                entry.get("text", "").strip() for entry in regular_entries
            ]
            if not selected_text:
                selected_text = [normalize_subject(tui_headline)]
            move_regular_entries = True

        if move_regular_entries:
            for entry in regular_entries:
                release_key = REGULAR_CATEGORY_KEYS[entry["category"]]
                values = changelog.get(release_key)
                if isinstance(values, list):
                    remove_first(values, entry.get("text", "").strip())

        for text in selected_text:
            append_unique(tui_updates, text)

    changelog["tui_updates"] = tui_updates
    return changelog


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Add updates that impact TUI users to legacy changelog JSON"
    )
    parser.add_argument("--input", required=True, help="Legacy changelog JSON")
    parser.add_argument("--output", required=True, help="Updated changelog JSON")
    parser.add_argument("--repo", required=True, help="GitHub repo (owner/name)")
    parser.add_argument("--channel", required=True, help="Release channel")
    parser.add_argument("--release-tag", required=True, help="Current release tag")
    args = parser.parse_args()

    base_ref = previous_release_cut(args.release_tag, args.channel)
    release_prs = collect_prs(args.repo, base_ref, args.release_tag)["prs"]
    with open(args.input) as f:
        changelog = json.load(f)

    changelog = add_tui_updates(changelog, release_prs)
    with open(args.output, "w") as f:
        json.dump(changelog, f, indent=2)
        f.write("\n")

    print(
        f"Added {len(changelog['tui_updates'])} TUI updates "
        f"from {base_ref}..{args.release_tag}"
    )


if __name__ == "__main__":
    main()
