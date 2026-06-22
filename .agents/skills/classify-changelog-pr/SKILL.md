---
name: classify-changelog-pr
description: Reference guidance for classifying whether an unmarked PR should appear in the changelog and under which category. Used inline by the changelog-draft skill — not dispatched as a separate agent.
---

# Classify Changelog PR

This document provides classification rules for PRs that lack explicit `CHANGELOG-*` markers. The changelog-draft agent follows these rules inline when deciding whether to include an unmarked PR.

## Categories

- **NEW-FEATURE** — A substantial new user-facing capability. Reserve for features that would warrant docs, marketing, or social media attention.
- **IMPROVEMENT** — Enhances an existing feature in a way users would notice (performance, UX, new options).
- **BUG-FIX** — Fixes a user-visible bug or regression.
- **OZ** — Changes to Oz / AI agent capabilities. At most 4 per release in the stable changelog.
- **NONE** — Explicitly opt out of changelog inclusion. Handled upstream by `fetch_prs.py` marker extraction.

## Decision rules

### Always exclude
- PRs with an explicit `CHANGELOG-NONE` marker (contributor opted out)
- PRs authored by known bots (dependabot, renovate, github-actions, codecov)
- PRs that exclusively modify CI workflows (`.github/workflows/`), test files, or dev tooling
- PRs that only update internal docs, comments, or README files
- Dependency bumps with no user-facing behavior change
- Refactors with no observable behavior change (code moves, renames, formatting)

### Always include
- PRs with explicit `CHANGELOG-*` markers (handled before this guidance applies)
- PRs that fix a crash, data loss, or security issue — even without a marker

### Conditional on channel
- **Stable channel:** Only include changes that are live for all users. Exclude PRs gated behind `DOGFOOD_FLAGS` or `PREVIEW_FLAGS`.
- **Preview channel:** Include PRs gated behind `PREVIEW_FLAGS`. Still exclude `DOGFOOD_FLAGS`-only changes.
- **Dev channel:** Include everything that's user-visible, regardless of flag gates.

### Feature-flagged PRs
If a PR mentions a `FeatureFlag` variant in its diff or title:
1. Check which flag list it belongs to (`RELEASE_FLAGS`, `PREVIEW_FLAGS`, `DOGFOOD_FLAGS`).
2. Apply the channel rules above.
3. If the flag is in `RELEASE_FLAGS` or enabled by default in `app/Cargo.toml`, treat it as live.
4. Set `feature_flag` in the classification output to the flag name.

### Confidence levels
- **high** — Clear user-visible change with obvious category.
- **medium** — Likely user-visible but category or scope is somewhat ambiguous.
- **low** — Unclear whether users would notice; or the PR touches both internal and user-facing code. Set `needs_review: true`.

## Writing changelog text

- Write from the user's perspective: "Added X", "Fixed Y", "Improved Z".
- Keep it to one sentence, ≤ 120 characters.
- Don't reference internal implementation details, file paths, or function names.
- Don't start with "PR" or the PR number — those are added as metadata.
- Use active voice and present tense for new features ("Adds dark mode"), past tense for fixes ("Fixed crash on startup").
