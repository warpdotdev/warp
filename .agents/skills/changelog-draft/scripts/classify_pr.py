#!/usr/bin/env python3
"""Deterministically classify unmarked PRs for the changelog-draft skill (Step 6).

Applies mechanical exclusion rules and channel flag gates to PRs from fetch_prs.py,
producing a candidate list for agent subjective classification.  When called again
with --agent-classifications, it validates and merges the agent's answers, enforcing
that mechanical excludes always win.

Two-pass workflow
-----------------
Pass 1 (no --agent-classifications):
    Emit deterministic preclassification: hard mechanical exclusions plus an
    agent_required list of candidate PRs the agent must classify subjectively.

Pass 2 (with --agent-classifications):
    Re-run the same deterministic checks, validate and merge the agent's
    subjective answers for candidates, and emit final Step 6 classifications.
    Mechanical excludes beat conflicting agent answers — conflicts exit non-zero.

Usage (pass 1):
    python3 classify_pr.py \\
        --channel <stable|preview|dev> \\
        --prs-json <fetch_prs.json> \\
        --feature-flags-json <feature_flags.json> \\
        --contributors-json <contributors.json> \\
        [--output <classifications.json>]

Usage (pass 2):
    python3 classify_pr.py \\
        --channel <stable|preview|dev> \\
        --prs-json <fetch_prs.json> \\
        --feature-flags-json <feature_flags.json> \\
        --contributors-json <contributors.json> \\
        --agent-classifications <agent_classifications.json> \\
        [--output <classifications.json>]

Output JSON schema:
    {
      "channel": "stable",
      "classifications": [
        {
          "pr_number": 1234,
          "include": false,
          "category": null,
          "text": null,
          "confidence": "high",
          "rationale": "Bot author: dependabot",
          "feature_flag": null,
          "feature_flags": [],
          "needs_review": false,
          "matched_rules": ["bot_author"],
          "source": "deterministic"
        }
      ],
      "agent_required": [
        {
          "pr_number": 5678,
          "title": "...",
          "url": "...",
          "author": "...",
          "changed_files": [...]
        }
      ],
      "unknown_contributors": ["author1"],
      "summary": {
        "total_unmarked": 10,
        "deterministic_exclude": 3,
        "agent_classify": 7,
        "included": 5,
        "needs_review": 2
      }
    }
"""

import argparse
import json
import re
import sys

# ---------------------------------------------------------------------------
# Constants
# ---------------------------------------------------------------------------

VALID_CHANNELS = frozenset({"stable", "preview", "dev"})
VALID_CATEGORIES = frozenset({"NEW-FEATURE", "IMPROVEMENT", "BUG-FIX", "OZ", "IMAGE"})
VALID_CONFIDENCES = frozenset({"high", "medium", "low"})

# Known bot usernames (in addition to the 'bot' bucket from contributors.json)
KNOWN_BOTS = frozenset(
    {
        "dependabot",
        "dependabot[bot]",
        "renovate",
        "renovate[bot]",
        "github-actions",
        "github-actions[bot]",
        "codecov",
        "codecov[bot]",
        "warp-bot",
        "warp-bot[bot]",
        # Repo-sync bot — never a user-facing PR author in the public repo context
        "warp-repo-sync",
        "warp-repo-sync[bot]",
        "app/warp-repo-sync",
    }
)

# Pattern to detect FeatureFlag references in PR title/body
FEATURE_FLAG_RE = re.compile(r"\bFeatureFlag::(\w+)\b")

# Path of the feature flag registry file
FEATURE_FLAG_FILE = "crates/warp_features/src/lib.rs"

# ---------------------------------------------------------------------------
# Path predicates — conservative; only exclude when ALL files match
# ---------------------------------------------------------------------------


def _is_internal_only_file(path: str) -> bool:
    """Return True if the file is unambiguously non-user-facing.

    Conservative: only matches well-known internal/tooling paths.  Any
    uncertainty leaves the file as potentially user-facing.
    """
    filename = path.rsplit("/", 1)[-1]
    lower_filename = filename.lower()

    # CI configuration
    if path.startswith(".github/"):
        return True

    # Test files
    if (
        "/tests/" in path
        or path.startswith("tests/")
        or path.startswith("crates/integration/")
        or filename.endswith("_test.rs")
        or filename.endswith("_tests.rs")
        or (filename.startswith("test_") and filename.endswith(".py"))
        or filename == "mod_test.rs"
    ):
        return True

    # Documentation
    if filename.endswith(".md"):
        return True
    if lower_filename in {
        "readme",
        "readme.txt",
        "license",
        "license-agpl",
        "license-mit",
        "changelog",
        "contributing",
        "security",
        "code_of_conduct",
        "faq",
        "about.hbs",
        "about.toml",
    }:
        return True

    # Build and tooling scripts (not source code)
    if path.startswith("scripts/") or path.startswith("script/"):
        return True

    # Lock files have no user-facing behavior change
    if filename.endswith(".lock"):
        return True

    return False


def all_files_internal(changed_files: list[str]) -> bool:
    """Return True only when every changed file is clearly internal-only.

    An empty file list is treated as unknown, not as internal-only.
    """
    if not changed_files:
        return False
    return all(_is_internal_only_file(f) for f in changed_files)


# ---------------------------------------------------------------------------
# Bot detection
# ---------------------------------------------------------------------------


def is_bot_author(author: str, bot_bucket: set[str]) -> bool:
    """Return True if the author is a known bot."""
    if not author:
        return False
    return (
        author in KNOWN_BOTS
        or author in bot_bucket
        or author.endswith("[bot]")
    )


# ---------------------------------------------------------------------------
# Feature flag detection
# ---------------------------------------------------------------------------


def detect_feature_flags(pr: dict) -> list[str]:
    """Scan PR title and body for FeatureFlag::<Name> references."""
    title = pr.get("title", "") or ""
    body = pr.get("body", "") or ""
    text = f"{title}\n{body}"
    return sorted(set(FEATURE_FLAG_RE.findall(text)))


def is_hidden_by_flag_gate(
    flags: list[str], channel: str, feature_flags: dict
) -> tuple[bool, list[str]]:
    """Return (is_hidden, hidden_flags) based on channel visibility rules.

    Stable:  excludes preview_flags and dogfood_flags.
    Preview: excludes dogfood_flags; includes preview_flags.
    Dev:     no flag-based exclusion.
    """
    if channel == "dev" or not flags:
        return False, []

    preview_set = set(feature_flags.get("preview_flags", []))
    dogfood_set = set(feature_flags.get("dogfood_flags", []))

    hidden: list[str] = []
    for flag in flags:
        if channel == "stable" and (flag in preview_set or flag in dogfood_set):
            hidden.append(flag)
        elif channel == "preview" and flag in dogfood_set:
            hidden.append(flag)

    return bool(hidden), hidden


# ---------------------------------------------------------------------------
# Core classification
# ---------------------------------------------------------------------------


def classify_one_pr(
    pr: dict,
    channel: str,
    feature_flags: dict,
    bot_bucket: set[str],
    unknown_bucket: set[str],
) -> dict:
    """Apply deterministic rules to a single unmarked PR.

    Returns a classification dict (source='deterministic') if the PR is
    deterministically excluded, or a candidate dict (source='candidate') if
    it requires agent judgment.

    Classification dict shape (deterministic exclude):
        {pr_number, include, category, text, confidence, rationale,
         feature_flag (compat), feature_flags, needs_review, matched_rules, source}

    Candidate dict shape (agent required):
        {pr_number, title, url, author, changed_files, source_repo,
         [detected_feature_flags], [uncertain_flag_touch], [unknown_contributor], source}
    """
    pr_number = pr.get("number")
    author = pr.get("author", "") or ""
    changed_files = pr.get("changed_files") or []

    # --- Rule: bot author ---
    if is_bot_author(author, bot_bucket):
        return {
            "pr_number": pr_number,
            "include": False,
            "category": None,
            "text": None,
            "confidence": "high",
            "rationale": f"Bot author: {author}",
            "feature_flag": None,
            "feature_flags": [],
            "needs_review": False,
            "matched_rules": ["bot_author"],
            "source": "deterministic",
        }

    # --- Rule: CI/test/docs/tooling-only files ---
    if all_files_internal(changed_files):
        return {
            "pr_number": pr_number,
            "include": False,
            "category": None,
            "text": None,
            "confidence": "high",
            "rationale": "All changed files are CI, tests, docs, or internal tooling",
            "feature_flag": None,
            "feature_flags": [],
            "needs_review": False,
            "matched_rules": ["internal_files_only"],
            "source": "deterministic",
        }

    # --- Feature flag detection ---
    detected_flags = detect_feature_flags(pr)
    touches_flag_file = FEATURE_FLAG_FILE in changed_files

    # Flag-registry touch without an identifiable flag → low-confidence candidate
    uncertain_flag_touch = touches_flag_file and not detected_flags

    if detected_flags:
        hidden, hidden_flags = is_hidden_by_flag_gate(detected_flags, channel, feature_flags)
        if hidden:
            return {
                "pr_number": pr_number,
                "include": False,
                "category": None,
                "text": None,
                "confidence": "high",
                "rationale": (
                    f"PR is gated behind {channel}-excluded feature flag(s): "
                    + ", ".join(hidden_flags)
                ),
                "feature_flag": hidden_flags[0] if len(hidden_flags) == 1 else None,
                "feature_flags": hidden_flags,
                "needs_review": False,
                "matched_rules": ["feature_flag_gate"],
                "source": "deterministic",
            }

    # --- Candidate: requires agent judgment ---
    # If there's an uncertain flag file touch, annotate for the agent
    candidate: dict = {
        "pr_number": pr_number,
        "title": pr.get("title", ""),
        "url": pr.get("url", ""),
        "author": author,
        "changed_files": changed_files,
        "source_repo": pr.get("source_repo", ""),
        "source": "candidate",
    }
    if detected_flags:
        candidate["detected_feature_flags"] = detected_flags
    if uncertain_flag_touch:
        candidate["uncertain_flag_touch"] = True
    if author in unknown_bucket:
        candidate["unknown_contributor"] = True

    return candidate


# ---------------------------------------------------------------------------
# Two-pass logic
# ---------------------------------------------------------------------------


def run_pass1(
    prs: list[dict],
    channel: str,
    feature_flags: dict,
    bot_bucket: set[str],
    unknown_bucket: set[str],
) -> dict:
    """Pass 1: emit deterministic excludes + candidate list."""
    classifications: list[dict] = []
    agent_required: list[dict] = []
    skipped_explicit = 0
    unknown_contributors: list[str] = []

    for pr in prs:
        pr_number = pr.get("number")
        author = pr.get("author", "") or ""
        explicit_entries = pr.get("explicit_entries") or []

        # PRs with explicit changelog entries (including NONE) are handled by
        # assemble_changelog.py; skip them here.
        if explicit_entries:
            skipped_explicit += 1
            continue

        if author in unknown_bucket and author not in unknown_contributors:
            unknown_contributors.append(author)

        det = classify_one_pr(pr, channel, feature_flags, bot_bucket, unknown_bucket)
        if det.get("source") == "deterministic":
            classifications.append(det)
        else:
            agent_required.append(det)

    deterministic_exclude = len(classifications)
    agent_classify = len(agent_required)

    return {
        "channel": channel,
        "classifications": classifications,
        "agent_required": agent_required,
        "unknown_contributors": unknown_contributors,
        "summary": {
            "total_unmarked": len(prs) - skipped_explicit,
            "skipped_explicit_entries": skipped_explicit,
            "deterministic_exclude": deterministic_exclude,
            "agent_classify": agent_classify,
        },
    }


def validate_agent_entry(entry: dict) -> str | None:
    """Validate an agent-provided classification entry.

    Returns an error string if invalid, or None if valid.
    """
    pr_number = entry.get("pr_number")

    include = entry.get("include")
    if include is None:
        return f"PR #{pr_number}: 'include' field is required"

    if not isinstance(include, bool):
        return f"PR #{pr_number}: 'include' must be a boolean"

    if include:
        category = entry.get("category", "")
        if category not in VALID_CATEGORIES:
            return (
                f"PR #{pr_number}: invalid category '{category}'; "
                f"must be one of {sorted(VALID_CATEGORIES)}"
            )
        text = entry.get("text", "")
        if not text or not text.strip():
            return f"PR #{pr_number}: 'text' must be a non-empty changelog string"

    confidence = entry.get("confidence", "")
    if confidence not in VALID_CONFIDENCES:
        return (
            f"PR #{pr_number}: invalid confidence '{confidence}'; "
            f"must be one of {sorted(VALID_CONFIDENCES)}"
        )

    return None


def run_pass2(
    prs: list[dict],
    channel: str,
    feature_flags: dict,
    bot_bucket: set[str],
    unknown_bucket: set[str],
    agent_classifications: list[dict],
) -> dict:
    """Pass 2: re-run deterministic checks, validate + merge agent answers."""
    # Index agent classifications by PR number
    agent_by_pr: dict[int, dict] = {}
    for entry in agent_classifications:
        pr_number = entry.get("pr_number")
        if pr_number is not None:
            agent_by_pr[int(pr_number)] = entry

    classifications: list[dict] = []
    unknown_contributors: list[str] = []
    conflicts: list[str] = []
    skipped_explicit = 0

    for pr in prs:
        pr_number = pr.get("number")
        author = pr.get("author", "") or ""
        explicit_entries = pr.get("explicit_entries") or []

        # PRs with explicit entries are handled by assemble_changelog.py
        if explicit_entries:
            skipped_explicit += 1
            continue

        if author in unknown_bucket and author not in unknown_contributors:
            unknown_contributors.append(author)

        # Re-run deterministic checks
        det = classify_one_pr(pr, channel, feature_flags, bot_bucket, unknown_bucket)

        if det.get("source") == "deterministic":
            # Deterministic exclude — check for agent conflict
            agent_entry = agent_by_pr.get(pr_number)
            if agent_entry and agent_entry.get("include") is True:
                conflicts.append(
                    f"PR #{pr_number}: agent tried to include a mechanically excluded PR "
                    f"(rules: {det['matched_rules']}); mechanical exclude wins"
                )
            # Emit the deterministic result regardless
            classifications.append(det)
        else:
            # Candidate — derive flag annotations from the returned candidate dict
            detected_flags = det.get("detected_feature_flags", [])
            uncertain_flag_touch = det.get("uncertain_flag_touch", False)

            agent_entry = agent_by_pr.get(pr_number)
            if agent_entry is None:
                # No agent answer for this candidate — flag as needs_review
                classifications.append(
                    {
                        "pr_number": pr_number,
                        "include": False,
                        "category": None,
                        "text": None,
                        "confidence": "low",
                        "rationale": "No agent classification provided for candidate PR",
                        "feature_flag": None,
                        "feature_flags": detected_flags,
                        "needs_review": True,
                        "matched_rules": [],
                        "source": "deterministic",
                    }
                )
                continue

            # Validate the agent's answer
            err = validate_agent_entry(agent_entry)
            if err:
                print(f"error: invalid agent classification: {err}", file=sys.stderr)
                sys.exit(1)

            # Enforce: low confidence forces needs_review
            confidence = agent_entry.get("confidence", "low")
            needs_review = agent_entry.get("needs_review", False)
            if confidence == "low":
                needs_review = True

            # If agent marked as uncertain flag touch and still a candidate, preserve that
            if uncertain_flag_touch and not needs_review:
                needs_review = True
                confidence = "low"

            include = agent_entry.get("include", False)
            category = agent_entry.get("category") if include else None
            text = agent_entry.get("text") if include else None

            classifications.append(
                {
                    "pr_number": pr_number,
                    "include": include,
                    "category": category,
                    "text": text,
                    "confidence": confidence,
                    "rationale": agent_entry.get("rationale", ""),
                    "feature_flag": detected_flags[0] if len(detected_flags) == 1 else None,
                    "feature_flags": detected_flags,
                    "needs_review": needs_review,
                    "matched_rules": [],
                    "source": "agent",
                }
            )

    # Fail non-zero if any conflicts occurred
    if conflicts:
        print("error: mechanical excludes conflict with agent classifications:", file=sys.stderr)
        for c in conflicts:
            print(f"  {c}", file=sys.stderr)
        sys.exit(1)

    included = sum(1 for c in classifications if c.get("include"))
    needs_review = sum(1 for c in classifications if c.get("needs_review"))
    deterministic_exclude = sum(1 for c in classifications if c.get("source") == "deterministic")

    return {
        "channel": channel,
        "classifications": classifications,
        "agent_required": [],
        "unknown_contributors": unknown_contributors,
        "summary": {
            "total_unmarked": len(prs) - skipped_explicit,
            "skipped_explicit_entries": skipped_explicit,
            "deterministic_exclude": deterministic_exclude,
            "agent_classify": len(classifications) - deterministic_exclude,
            "included": included,
            "needs_review": needs_review,
        },
    }


# ---------------------------------------------------------------------------
# Entry point
# ---------------------------------------------------------------------------


def main() -> None:
    parser = argparse.ArgumentParser(
        description=(
            "Deterministically classify unmarked PRs for the changelog-draft skill. "
            "Without --agent-classifications, emits a candidate list. "
            "With --agent-classifications, merges and validates agent answers."
        )
    )
    parser.add_argument(
        "--channel",
        required=True,
        choices=sorted(VALID_CHANNELS),
        help="Release channel: dev, preview, or stable",
    )
    parser.add_argument(
        "--prs-json",
        required=True,
        help="Path to fetch_prs.py output JSON",
    )
    parser.add_argument(
        "--feature-flags-json",
        required=True,
        help="Path to extract_feature_flags.py output JSON",
    )
    parser.add_argument(
        "--contributors-json",
        required=True,
        help="Path to classify_contributors.py output JSON",
    )
    parser.add_argument(
        "--agent-classifications",
        help=(
            "Path to agent-provided classification JSON (pass 2). "
            "When omitted, runs pass 1 (emit candidates)."
        ),
    )
    parser.add_argument(
        "--output",
        help="Path to write output JSON (default: stdout)",
    )
    args = parser.parse_args()

    # Load inputs
    with open(args.prs_json) as f:
        prs_data = json.load(f)
    prs: list[dict] = prs_data.get("prs", [])

    with open(args.feature_flags_json) as f:
        feature_flags: dict = json.load(f)

    with open(args.contributors_json) as f:
        contributors: dict = json.load(f)

    bot_bucket: set[str] = set(contributors.get("bot", []))
    unknown_bucket: set[str] = set(contributors.get("unknown", []))

    if args.agent_classifications:
        with open(args.agent_classifications) as f:
            agent_classifications: list[dict] = json.load(f)
        result = run_pass2(
            prs, args.channel, feature_flags, bot_bucket, unknown_bucket,
            agent_classifications,
        )
    else:
        result = run_pass1(
            prs, args.channel, feature_flags, bot_bucket, unknown_bucket,
        )

    output_text = json.dumps(result, indent=2) + "\n"

    if args.output:
        with open(args.output, "w") as f:
            f.write(output_text)
        # Print summary to stderr so stdout remains clean
        s = result["summary"]
        print(
            f"classify_pr: {s.get('total_unmarked', 0)} unmarked PRs; "
            f"{s.get('deterministic_exclude', 0)} excluded; "
            f"{s.get('agent_classify', 0)} for agent",
            file=sys.stderr,
        )
    else:
        sys.stdout.write(output_text)


if __name__ == "__main__":
    main()
