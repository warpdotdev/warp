#!/usr/bin/env python3
"""Assemble the changelog draft from intermediate JSON artifacts (Steps 7 + 8).

Combines explicit changelog markers (from fetch_prs.py), deterministic and
agent-provided classifications (from classify_pr.py), contributor data, and
issue reporters into the two output files consumed by downstream tooling.

Usage:
    python3 assemble_changelog.py \\
        --channel <stable|preview|dev> \\
        --range-json <range.json> \\
        --prs-json <fetch_prs.json> \\
        --contributors-json <contributors.json> \\
        --issue-reporters-json <issue_reporters.json> \\
        --classifications-json <classifications.json> \\
        --output-dir <dir> \\
        [--attribution <external-only|all|none>] \\
        [--generated-at <iso8601>]

Outputs:
    <output-dir>/changelog-draft.md   — Human-reviewable markdown
    <output-dir>/changelog-draft.json — Machine-readable audit artifact

Exits non-zero if any PR appears in more than one bucket, or if no bucket
accounts for a PR that is in the range.

Constraints (read-only):
    - Does not push, create branches, or open PRs.
    - Does not write to channel_versions.json or any production config.
    - All output goes to output_dir only.
"""

import argparse
import json
import os
import sys
from datetime import datetime, timezone

# ---------------------------------------------------------------------------
# Constants
# ---------------------------------------------------------------------------

VALID_CHANNELS = frozenset({"stable", "preview", "dev"})
VALID_ATTRIBUTION = frozenset({"external-only", "all", "none"})

# Canonical category order for the changelog
CATEGORY_ORDER = ["NEW-FEATURE", "IMPROVEMENT", "BUG-FIX", "OZ", "IMAGE"]

CATEGORY_HEADINGS = {
    "NEW-FEATURE": "New Features",
    "IMPROVEMENT": "Improvements",
    "BUG-FIX": "Bug Fixes",
    "OZ": "Oz Updates",
    "IMAGE": "Images",
}


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------


def github_profile_link(username: str) -> str:
    """Format a GitHub username as a markdown profile link."""
    return f"[@{username}](https://github.com/{username})"


def format_pr_link(pr_number: int | None, url: str | None) -> str:
    """Format a PR reference as a markdown link, or empty string if URL is absent."""
    if url and pr_number:
        return f" ([#{pr_number}]({url}))"
    return ""


def attribution_suffix(
    author: str,
    is_external: bool,
    attribution: str,
) -> str:
    """Return the attribution suffix for a changelog entry, or empty string."""
    if not author or attribution == "none":
        return ""
    if attribution == "all" or (attribution == "external-only" and is_external):
        return f" — {github_profile_link(author)} ✨"
    return ""


# ---------------------------------------------------------------------------
# Assembly
# ---------------------------------------------------------------------------


def is_external_author(author: str, external_set: set[str]) -> bool:
    """Return True if the author is in the external contributor set."""
    return bool(author) and author in external_set


def assemble(
    channel: str,
    range_data: dict,
    prs: list[dict],
    contributors: dict,
    issue_reporters_data: dict,
    classifications: list[dict],
    attribution: str,
    generated_at: str,
) -> tuple[dict, str]:
    """Build the draft data and markdown string.

    Returns (draft_dict, markdown_str).
    Raises SystemExit with non-zero status if any PR appears in multiple
    buckets or is missing from all buckets.
    """
    # Build lookup structures
    external_set: set[str] = set(contributors.get("external", []))

    # Index classifications by PR number
    class_by_pr: dict[int, dict] = {}
    for c in classifications:
        pn = c.get("pr_number")
        if pn is not None:
            class_by_pr[int(pn)] = c

    entries: list[dict] = []
    skipped: list[dict] = []
    needs_review: list[dict] = []

    # Track PR numbers seen across ALL buckets to enforce one-bucket rule
    seen: dict[int, str] = {}  # pr_number -> bucket name

    def _add_to_bucket(pr_number: int, bucket: str) -> bool:
        """Return False and record error if PR already placed in another bucket."""
        if pr_number in seen:
            return False
        seen[pr_number] = bucket
        return True

    for pr in prs:
        pr_number = int(pr.get("number", 0))
        url = pr.get("url", "") or ""
        author = pr.get("author", "") or ""
        source_repo = pr.get("source_repo", "")
        internal_pr = pr.get("internal_pr")
        explicit_entries = pr.get("explicit_entries") or []

        is_external = is_external_author(author, external_set)

        if explicit_entries:
            # PR has explicit CHANGELOG-* markers
            is_none = (
                len(explicit_entries) == 1
                and explicit_entries[0].get("category") == "NONE"
            )
            if is_none:
                # Explicit opt-out
                ok = _add_to_bucket(pr_number, "skipped")
                if not ok:
                    print(
                        f"error: PR #{pr_number} appears in multiple buckets "
                        f"(already in '{seen.get(pr_number)}', now trying 'skipped')",
                        file=sys.stderr,
                    )
                    sys.exit(1)
                skipped.append(
                    {
                        "pr_number": pr_number,
                        "url": url or None,
                        "rationale": "Explicit CHANGELOG-NONE opt-out",
                        "source_repo": source_repo,
                        "internal_pr": internal_pr,
                    }
                )
            else:
                # Explicit non-NONE markers → one entry per marker
                ok = _add_to_bucket(pr_number, "entries")
                if not ok:
                    print(
                        f"error: PR #{pr_number} appears in multiple buckets "
                        f"(already in '{seen.get(pr_number)}', now trying 'entries')",
                        file=sys.stderr,
                    )
                    sys.exit(1)
                for marker in explicit_entries:
                    category = marker.get("category", "")
                    text = marker.get("text", "")
                    if not category or category == "NONE":
                        continue
                    entries.append(
                        {
                            "pr_number": pr_number,
                            "url": url or None,
                            "category": category,
                            "text": text,
                            "source": "explicit",
                            "author": author,
                            "is_external": is_external,
                            "confidence": "high",
                            "rationale": None,
                            "feature_flag": None,
                            "source_repo": source_repo,
                            "internal_pr": internal_pr,
                        }
                    )
        else:
            # Unmarked PR — look up classification
            c = class_by_pr.get(pr_number)
            if c is None:
                # No classification found — this PR was not accounted for
                # Treat as needs_review so the accounting invariant holds
                ok = _add_to_bucket(pr_number, "needs_review")
                if not ok:
                    print(
                        f"error: PR #{pr_number} appears in multiple buckets",
                        file=sys.stderr,
                    )
                    sys.exit(1)
                needs_review.append(
                    {
                        "pr_number": pr_number,
                        "url": url or None,
                        "reason": "No classification found for this PR",
                        "source_repo": source_repo,
                        "internal_pr": internal_pr,
                    }
                )
                continue

            include = c.get("include", False)
            pr_needs_review = c.get("needs_review", False)

            if pr_needs_review:
                ok = _add_to_bucket(pr_number, "needs_review")
                if not ok:
                    print(
                        f"error: PR #{pr_number} appears in multiple buckets "
                        f"(already in '{seen.get(pr_number)}', now trying 'needs_review')",
                        file=sys.stderr,
                    )
                    sys.exit(1)
                needs_review.append(
                    {
                        "pr_number": pr_number,
                        "url": url or None,
                        "reason": c.get("rationale", "Flagged for manual review"),
                        "confidence": c.get("confidence"),
                        "source_repo": source_repo,
                        "internal_pr": internal_pr,
                    }
                )
            elif include:
                ok = _add_to_bucket(pr_number, "entries")
                if not ok:
                    print(
                        f"error: PR #{pr_number} appears in multiple buckets "
                        f"(already in '{seen.get(pr_number)}', now trying 'entries')",
                        file=sys.stderr,
                    )
                    sys.exit(1)
                entries.append(
                    {
                        "pr_number": pr_number,
                        "url": url or None,
                        "category": c.get("category"),
                        "text": c.get("text", ""),
                        "source": "inferred",
                        "author": author,
                        "is_external": is_external,
                        "confidence": c.get("confidence"),
                        "rationale": c.get("rationale"),
                        "feature_flag": c.get("feature_flag"),
                        "source_repo": source_repo,
                        "internal_pr": internal_pr,
                    }
                )
            else:
                # Deterministic or agent exclude, not needs_review
                ok = _add_to_bucket(pr_number, "skipped")
                if not ok:
                    print(
                        f"error: PR #{pr_number} appears in multiple buckets "
                        f"(already in '{seen.get(pr_number)}', now trying 'skipped')",
                        file=sys.stderr,
                    )
                    sys.exit(1)
                skipped.append(
                    {
                        "pr_number": pr_number,
                        "url": url or None,
                        "rationale": c.get("rationale", ""),
                        "matched_rules": c.get("matched_rules", []),
                        "source_repo": source_repo,
                        "internal_pr": internal_pr,
                    }
                )

    # Verify every PR appears in exactly one bucket
    all_prs_in_range = {int(pr.get("number", 0)) for pr in prs}
    missing = all_prs_in_range - set(seen.keys())
    if missing:
        print(
            f"error: {len(missing)} PR(s) not accounted for in any bucket: "
            + ", ".join(f"#{n}" for n in sorted(missing)),
            file=sys.stderr,
        )
        sys.exit(1)

    issue_reporters = issue_reporters_data.get("issue_reporters", [])

    draft_dict: dict = {
        "channel": channel,
        "range": range_data,
        "generated_at": generated_at,
        "entries": entries,
        "skipped": skipped,
        "needs_review": needs_review,
        "issue_reporters": issue_reporters,
    }

    # Collect external contributors for Community section
    # Only PRs in entries with external authors; store the entry's url to avoid
    # synthesizing PR URLs (spec constraint: use stored url, omit link when empty).
    ext_contributors: dict[str, list[dict]] = {}
    for entry in entries:
        if entry.get("is_external") and attribution != "none":
            a = entry.get("author", "")
            pn = entry.get("pr_number")
            if a and pn is not None:
                ext_contributors.setdefault(a, [])
                if not any(e["pr_number"] == pn for e in ext_contributors[a]):
                    ext_contributors[a].append({"pr_number": pn, "url": entry.get("url") or None})

    markdown = _build_markdown(
        channel=channel,
        range_data=range_data,
        generated_at=generated_at,
        entries=entries,
        ext_contributors=ext_contributors,
        issue_reporters=issue_reporters,
        attribution=attribution,
    )

    return draft_dict, markdown


def _build_markdown(
    channel: str,
    range_data: dict,
    generated_at: str,
    entries: list[dict],
    ext_contributors: dict[str, list[dict]],
    issue_reporters: list[dict],
    attribution: str,
) -> str:
    """Build the human-reviewable markdown changelog draft."""
    lines: list[str] = []

    lines.append("# Changelog Draft")
    lines.append(f"**Channel:** {channel}")
    lines.append(
        f"**Range:** {range_data.get('base', '')} → {range_data.get('head', '')}"
    )
    lines.append(f"**Generated:** {generated_at}")
    lines.append("")

    # Group entries by category in canonical order
    # Use pr_number to deduplicate entries within the same PR (multiple explicit markers
    # can appear per PR; each appears as a separate entry).
    by_category: dict[str, list[dict]] = {cat: [] for cat in CATEGORY_ORDER}
    for entry in entries:
        cat = entry.get("category", "")
        if cat in by_category:
            by_category[cat].append(entry)
        # Unknown categories are silently ignored (shouldn't occur in practice)

    # Emit each category section (skip IMAGE in markdown — it's JSON-only)
    for cat in CATEGORY_ORDER:
        if cat == "IMAGE":
            continue  # IMAGE entries are for JSON/release conversion only
        cat_entries = by_category.get(cat, [])
        if not cat_entries:
            continue
        heading = CATEGORY_HEADINGS.get(cat, cat)
        lines.append(f"## {heading}")
        for entry in cat_entries:
            text = entry.get("text", "")
            pr_number = entry.get("pr_number")
            url = entry.get("url", "") or ""
            author = entry.get("author", "")
            is_external = entry.get("is_external", False)

            pr_link = format_pr_link(pr_number, url)
            attr = attribution_suffix(author, is_external, attribution)
            lines.append(f"- {text}{pr_link}{attr}")
        lines.append("")

    # Community section
    has_contributors = bool(ext_contributors) and attribution != "none"
    has_reporters = bool(issue_reporters)

    if has_contributors or has_reporters:
        lines.append("## Community")

        if has_contributors:
            lines.append("### Contributors")
            for author, pr_entries in sorted(ext_contributors.items()):
                pr_links = ", ".join(
                    f"[#{e['pr_number']}]({e['url']})" if e.get("url") else f"#{e['pr_number']}"
                    for e in sorted(pr_entries, key=lambda x: x["pr_number"])
                )
                lines.append(f"- {github_profile_link(author)} — {pr_links}  ✨")
            lines.append("")

        if has_reporters:
            lines.append("### Issue Reporters")
            lines.append(
                "Thanks to the community members who reported issues fixed in this release:"
            )
            for reporter in issue_reporters:
                reporter_handle = reporter.get("reporter", "")
                reporter_url = reporter.get("reporter_url", "")
                issue_number = reporter.get("issue_number")
                issue_url = reporter.get("url", "")
                issue_title = reporter.get("title", "")

                if reporter_url:
                    reporter_link = f"[@{reporter_handle}]({reporter_url})"
                else:
                    reporter_link = github_profile_link(reporter_handle)

                issue_ref = ""
                if issue_url and issue_number:
                    issue_ref = f" — [#{issue_number}]({issue_url})"
                    if issue_title:
                        issue_ref += f' "{issue_title}"'

                lines.append(f"- {reporter_link}{issue_ref}")
            lines.append("")

    return "\n".join(lines)


# ---------------------------------------------------------------------------
# Entry point
# ---------------------------------------------------------------------------


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Assemble changelog-draft.md and changelog-draft.json from intermediate artifacts"
    )
    parser.add_argument(
        "--channel",
        required=True,
        choices=sorted(VALID_CHANNELS),
        help="Release channel: dev, preview, or stable",
    )
    parser.add_argument(
        "--range-json",
        required=True,
        help="Path to resolve_release_range.py output JSON",
    )
    parser.add_argument(
        "--prs-json",
        required=True,
        help="Path to fetch_prs.py output JSON",
    )
    parser.add_argument(
        "--contributors-json",
        required=True,
        help="Path to classify_contributors.py output JSON",
    )
    parser.add_argument(
        "--issue-reporters-json",
        required=True,
        help="Path to fetch_issue_reporters.py output JSON",
    )
    parser.add_argument(
        "--classifications-json",
        required=True,
        help="Path to classify_pr.py (pass 2) output JSON",
    )
    parser.add_argument(
        "--output-dir",
        required=True,
        help="Directory to write changelog-draft.md and changelog-draft.json",
    )
    parser.add_argument(
        "--attribution",
        default="external-only",
        choices=sorted(VALID_ATTRIBUTION),
        help="Attribution mode: all, external-only (default), or none",
    )
    parser.add_argument(
        "--generated-at",
        help=(
            "ISO 8601 timestamp for the generated_at field. "
            "Defaults to current UTC time. Pass a fixed value in tests for determinism."
        ),
    )
    args = parser.parse_args()

    # Resolve generated_at
    if args.generated_at:
        generated_at = args.generated_at
    else:
        generated_at = datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")

    # Load inputs
    with open(args.range_json) as f:
        range_data: dict = json.load(f)

    with open(args.prs_json) as f:
        prs_data: dict = json.load(f)
    prs: list[dict] = prs_data.get("prs", [])

    with open(args.contributors_json) as f:
        contributors: dict = json.load(f)

    with open(args.issue_reporters_json) as f:
        issue_reporters_data: dict = json.load(f)

    with open(args.classifications_json) as f:
        classifications_data: dict = json.load(f)
    classifications: list[dict] = classifications_data.get("classifications", [])

    # Ensure output directory exists
    os.makedirs(args.output_dir, exist_ok=True)

    draft_dict, markdown = assemble(
        channel=args.channel,
        range_data=range_data,
        prs=prs,
        contributors=contributors,
        issue_reporters_data=issue_reporters_data,
        classifications=classifications,
        attribution=args.attribution,
        generated_at=generated_at,
    )

    md_path = os.path.join(args.output_dir, "changelog-draft.md")
    json_path = os.path.join(args.output_dir, "changelog-draft.json")

    with open(md_path, "w") as f:
        f.write(markdown)
        if not markdown.endswith("\n"):
            f.write("\n")

    with open(json_path, "w") as f:
        json.dump(draft_dict, f, indent=2)
        f.write("\n")

    # Summary to stderr/stdout for CI logs
    n_entries = len(draft_dict.get("entries", []))
    n_skipped = len(draft_dict.get("skipped", []))
    n_review = len(draft_dict.get("needs_review", []))
    total = n_entries + n_skipped + n_review
    print(
        f"assemble_changelog: {n_entries} entries, {n_skipped} skipped, "
        f"{n_review} needs_review ({total} total PRs)"
    )
    print(f"  wrote: {md_path}")
    print(f"  wrote: {json_path}")


if __name__ == "__main__":
    main()
