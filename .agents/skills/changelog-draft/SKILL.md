---
name: changelog-draft
description: Generate a reviewable changelog draft from PRs merged in a release range. Extracts explicit CHANGELOG markers, classifies unmarked PRs, adds external contributor attribution, and outputs markdown + JSON artifacts. Does NOT mutate channel_versions.json.
---

# Changelog Draft Generator

## Inputs

| Parameter | Required | Description |
|-----------|----------|-------------|
| `channel` | yes | Release channel: `stable`, `preview`, or `dev` |
| `release_tag` | yes | The release tag to generate the changelog for (e.g. `v0.2026.05.06.09.12.stable_00`) |
| `output_dir` | no | Directory to write output files. Defaults to `$RUNNER_TEMP` or `/tmp/changelog-draft` |
| `attribution` | no | Attribution mode: `external-only` (default), `all`, or `none` |

## Workflow

### Step 1 — Determine the release range

Infer the previous release **cut** for comparison. Release tags follow the pattern `v0.YYYY.MM.DD.HH.MM.<channel>_NN`, where `_NN` is the RC/hotfix number within that release cut. Multiple tags can share the same date prefix (e.g. `_00`, `_01`, `_02` are all part of one release cut).

The base tag must be the `_00` tag of the **previous** release cut (i.e. a different date), not just the previous tag. For example, if generating a changelog for `v0.2026.04.29.08.57.stable_01`, the base should be `v0.2026.04.22.08.57.stable_00`, not `v0.2026.04.29.08.57.stable_00`.

```bash
# 1. Extract the date prefix from the release_tag (everything before _NN)
release_date_prefix="${release_tag%_*}"

# 2. List all _00 tags for the channel (these are release cut points), sorted descending
git tag --list "v0.*.${channel}_00" --sort=-version:refname

# 3. Pick the first _00 tag whose date prefix differs from release_date_prefix
```

Record the range as `previous_cut_tag..release_tag`.

### Step 2 — Fetch PR data

Run the `fetch_prs.py` script to collect all PRs merged in the release range and extract explicit changelog markers:

```bash
python3 .agents/skills/changelog-draft/scripts/fetch_prs.py \
  --repo warpdotdev/warp \
  --base-ref <previous_tag> \
  --head-ref <release_tag>
```

The script outputs JSON to stdout with this structure:
```json
{
  "range": { "base": "<previous_tag>", "head": "<release_tag>" },
  "prs": [
    {
      "number": 1234,
      "title": "...",
      "author": "username",
      "body": "...",
      "labels": ["..."],
      "merged_at": "2026-05-01T...",
      "explicit_entries": [
        { "category": "NEW-FEATURE", "text": "Added dark mode" }
      ],
      "linked_issues": [5678],
      "changed_files": ["app/src/ai/agent.rs", "crates/warp_features/src/lib.rs"]
    }
  ]
}
```

### Step 3 — Classify contributors

Run the `classify_contributors.py` script with the unique author logins from Step 2:

```bash
python3 .agents/skills/changelog-draft/scripts/classify_contributors.py \
  --org warpdotdev \
  --authors author1,author2,author3
```

Output JSON:
```json
{
  "internal": ["author1"],
  "external": ["author3"],
  "bot": ["author2"],
  "unknown": []
}
```

### Step 4 — Extract feature flags

Run the `extract_feature_flags.py` script to get the current flag gate lists:

```bash
python3 .agents/skills/changelog-draft/scripts/extract_feature_flags.py \
  --file crates/warp_features/src/lib.rs
```

Output JSON:
```json
{
  "release_flags": ["Autoupdate", "Changelog", ...],
  "preview_flags": ["Orchestration", ...],
  "dogfood_flags": ["LogExpensiveFramesInSentry", ...]
}
```

### Step 5 — Fetch issue reporters

Collect all unique `linked_issues` from Step 2 and fetch the original reporter for each. Pass `--org` so the script checks org membership and filters out internal reporters automatically:

```bash
python3 .agents/skills/changelog-draft/scripts/fetch_issue_reporters.py \
  --repo warpdotdev/warp \
  --org warpdotdev \
  --issues 5678,9012
```

Output JSON (only external reporters are included):
```json
{
  "issue_reporters": [
    {
      "issue_number": 5678,
      "title": "Crash when opening large file",
      "reporter": "community-user",
      "url": "https://github.com/warpdotdev/warp/issues/5678"
    }
  ]
}
```

The `--org` flag checks each reporter's org membership via the GitHub API, filtering out internal members so they aren't misattributed as external community reporters. These reporters will be credited in the "Community" section of the changelog.

### Step 6 — Classify unmarked PRs

For each PR that has no explicit `CHANGELOG-*` entries, decide whether to include it and under which category.

Follow the classification guidance in `.agents/skills/classify-changelog-pr/SKILL.md`.

For each unmarked PR, produce a classification:
```json
{
  "pr_number": 1234,
  "include": true,
  "category": "IMPROVEMENT",
  "text": "Proposed changelog line",
  "confidence": "high",
  "rationale": "...",
  "feature_flag": null,
  "needs_review": false
}
```

**Key rules:**
- PRs that only touch CI, tests, docs, or internal tooling → `include: false`
- PRs behind dogfood-only feature flags → `include: false` for stable channel
- PRs behind preview flags → `include: false` for stable, `include: true` for preview
- When in doubt, set `needs_review: true` and `confidence: "low"`
- Bot PRs (dependabot, renovate, etc.) → `include: false`

**Feature-flag detection:** Use the `changed_files` list from Step 2 to check if any PR touches `crates/warp_features/src/lib.rs` or references a `FeatureFlag` variant in its title/body. Cross-reference with the flag lists from Step 4 to determine channel visibility.

**Unknown contributors:** Authors in the `unknown` bucket (org membership check failed due to auth) should be treated conservatively — do not attribute them as external. Note them in the output for manual verification.

### Step 7 — Assemble the draft

Combine explicit entries (Step 2) and inferred entries (Step 6) into the final report. Group by category in this order:

1. `NEW-FEATURE` — New Features
2. `IMPROVEMENT` — Improvements
3. `BUG-FIX` — Bug Fixes
4. `OZ` — Oz Updates

PRs marked with `CHANGELOG-NONE` are explicitly opted out and must never appear in the changelog markdown.

### Step 8 — Write output files

Write two files to `output_dir`:

**`changelog-draft.md`** — Human-reviewable markdown, ready for Slack/Notion:

```markdown
# Changelog Draft
**Channel:** stable
**Range:** v0.2026.05.01... → v0.2026.05.06...
**Generated:** 2026-05-06T15:00:00Z

## New Features
- Added dark mode ([#1234](https://github.com/warpdotdev/warp/pull/1234)) — @external-contributor ✨

## Improvements
- Faster tab switching ([#1235](https://github.com/warpdotdev/warp/pull/1235))

## Bug Fixes
- Fixed crash on startup ([#1236](https://github.com/warpdotdev/warp/pull/1236))

## Oz Updates
- Improved agent memory ([#1237](https://github.com/warpdotdev/warp/pull/1237))

## Community
### Contributors
- @contributor1 — [#1234](https://github.com/warpdotdev/warp/pull/1234)  ✨

### Issue Reporters
Thanks to the community members who reported issues fixed in this release:
- @reporter1 — [#5678](https://github.com/warpdotdev/warp/issues/5678) "Crash when opening large file"
```

The markdown draft must **not** include "Needs Review" or "Skipped PRs" sections — those are internal details that belong only in the JSON audit artifact.

**`changelog-draft.json`** — Machine-readable audit artifact (internal only):

```json
{
  "channel": "stable",
  "range": { "base": "v0...", "head": "v0..." },
  "generated_at": "2026-05-06T15:00:00Z",
  "entries": [
    {
      "pr_number": 1234,
      "category": "NEW-FEATURE",
      "text": "Added dark mode",
      "source": "explicit",
      "author": "external-contributor",
      "is_external": true,
      "confidence": "high",
      "rationale": null,
      "feature_flag": null
    }
  ],
  "skipped": [...],
  "needs_review": [...],
  "issue_reporters": [...]
}
```

The JSON artifact retains `skipped`, `needs_review`, and `issue_reporters` for audit purposes — every PR in the range must appear in either `entries`, `skipped`, or `needs_review`.

### Step 9 — Generate release-pipeline JSON

Run the conversion script to deterministically produce `changelog-release.json` from the audit artifact:

```bash
python3 .agents/skills/changelog-draft/scripts/convert_to_release_json.py \
  --input <output_dir>/changelog-draft.json \
  --output <output_dir>/changelog-release.json
```

This produces the flat JSON structure consumed by the `create_release` workflow for Slack and the in-app "What's New" dialog. Do **not** generate this file manually — always use the script so the output is deterministic and consistent.

## Constraints

- **Never** write to `channel_versions.json` or any production config file.
- **Never** push commits, create branches, or open PRs.
- All output goes to `output_dir` only.
- The markdown draft should be copy-pasteable into Slack or Notion for review.
- Keep the JSON artifact complete enough for audit: every PR in the range should appear in either `entries`, `skipped`, or `needs_review`.

## Validation

After generating output, verify:
1. Every PR in the range is accounted for (entries + skipped + needs_review = total PRs).
2. Explicit marker entries match what `fetch_prs.py` extracted (no dropped markers).
3. No duplicate PR numbers across sections.
4. The markdown renders cleanly (no broken links or formatting).
