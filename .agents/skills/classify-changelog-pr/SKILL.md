---
name: classify-changelog-pr
description: Subjective-candidate guidance for classifying whether an unmarked PR should appear in the changelog and under which category. Used inline by the changelog-draft skill for PRs that passed the deterministic script filters — not dispatched as a separate agent.
---

# Classify Changelog PR (Subjective Candidates)

This document provides guidance for classifying **candidate PRs** that the `classify_pr.py` script (Step 6 of `changelog-draft`) has already passed through its deterministic filters. By the time you reach this guidance, the script has already deterministically excluded:

- Known bot authors (dependabot, renovate, github-actions, and authors ending in `[bot]`)
- PRs that exclusively touch CI, test, docs, or internal tooling files
- PRs gated behind channel-excluded feature flags (dogfood flags on stable, dogfood flags on preview)

**You are only classifying the remaining candidates.** Do not re-apply mechanical rules the script already enforced — trust its output.

## Categories

- **NEW-FEATURE** — A substantial new user-facing capability. Reserve for features that would warrant docs, marketing, or social media attention.
- **IMPROVEMENT** — Enhances an existing feature in a way users would notice (performance, UX, new options).
- **BUG-FIX** — Fixes a user-visible bug or regression.
- **OZ** — Changes to Oz / AI agent capabilities. At most 4 per release in the stable changelog.

## Subjective classification guidance

### Is this change user-visible?
- **Yes — include:** Changes to the visible UI, behavior the user controls, or outcomes the user can observe.
- **No — exclude:** Pure internal refactors, code moves, renames, or formatting with no observable behavior change.
- **Unclear — low confidence:** Set `confidence: "low"` and `needs_review: true`. The script will preserve this as a manual review item.

### Crashes, data loss, and security fixes
Always include PRs that fix a crash, data loss, or security issue — even if the diff appears small or purely internal.

### Refactors
Exclude refactors with no observable behavior change (code moves, renames, formatting). If the refactor enables a new capability, classify it by that capability.

### Feature-flagged PRs
The script has already excluded hidden flags. For candidate PRs that reference a visible flag (e.g. `RELEASE_FLAGS` or `unknown` flag registry changes), apply your judgment on user-visibility and category as normal.

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
