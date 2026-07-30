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

Run the `resolve_release_range.py` script to determine the previous release cut for comparison. Release tags follow the pattern `v0.YYYY.MM.DD.HH.MM.<channel>_NN`, where `_NN` is the RC/hotfix number within that release cut. The base tag is always the `_00` tag of the **previous** release cut (a different date-time prefix), not another tag from the same cut.

```bash
python3 .agents/skills/changelog-draft/scripts/resolve_release_range.py \
  --release-tag "${release_tag}" \
  --channel "${channel}" \
  --repo-dir . \
  > range.json
```

The script outputs JSON to stdout:
```json
{"base": "<prev_cut_00>", "head": "<release_tag>", "range": "<prev_cut_00>..<release_tag>"}
```

Save this to `range.json` and use the `base` and `head` values as `--base-ref` and `--head-ref` for Step 2. The script exits non-zero with a concise stderr message if the tag format is invalid, the channel mismatches, the head tag is missing, or no previous cut exists.

### Step 2 — Fetch PR data

Run the `fetch_prs.py` script to collect all public-release PRs merged in the release range and extract explicit changelog markers. Pass the repository that the workflow checked out, not necessarily the public repository. Release workflows run from `warpdotdev/warp-internal`, and the script deterministically resolves `warp-repo-sync[bot]` PRs back to their original public `warpdotdev/warp` PR metadata before emitting JSON. When running from `warpdotdev/warp-internal`, the script intentionally omits PRs that were not authored by the repo-sync bot, because those are private internal changes that must not be exposed to the changelog agent or generated artifacts.

```bash
# Extract base and head from range.json
base_tag=$(python3 -c "import json; d=json.load(open('range.json')); print(d['base'])")
head_tag=$(python3 -c "import json; d=json.load(open('range.json')); print(d['head'])")

python3 .agents/skills/changelog-draft/scripts/fetch_prs.py \
  --repo "${GITHUB_REPOSITORY:-warpdotdev/warp}" \
  --base-ref "${base_tag}" \
  --head-ref "${head_tag}" \
  > prs.json
```

The script outputs JSON to stdout with this structure:
```json
{
  "range": { "base": "<previous_tag>", "head": "<release_tag>" },
  "prs": [
    {
      "number": 1234,
      "url": "https://github.com/warpdotdev/warp/pull/1234",
      "title": "...",
      "author": "username",
      "body": "...",
      "labels": ["..."],
      "merged_at": "2026-05-01T...",
      "explicit_entries": [
        { "category": "NEW-FEATURE", "text": "Added dark mode" }
      ],
      "linked_issues": [5678],
      "changed_files": ["app/src/ai/agent.rs", "crates/warp_features/src/lib.rs"],
      "source_repo": "warpdotdev/warp",
      "internal_pr": {
        "number": 25712,
        "url": "https://github.com/warpdotdev/warp-internal/pull/25712",
        "author": "warp-repo-sync[bot]",
        "title": "...",
        "repo": "warpdotdev/warp-internal"
      }
    }
  ]
}
```

Use the top-level `number`, `url`, `author`, `body`, `labels`, `changed_files`, and `source_repo` fields as the source of truth. `internal_pr` is audit-only and must never be used for contributor attribution or user-facing changelog links. If `url` is empty, omit the PR link from user-facing markdown rather than synthesizing one.

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
      "reporter_url": "https://github.com/community-user",
      "url": "https://github.com/warpdotdev/warp/issues/5678"
    }
  ]
}
```

The `--org` flag checks each reporter's org membership via the GitHub API, filtering out internal members so they aren't misattributed as external community reporters. These reporters will be credited in the "Community" section of the changelog.
Whenever the markdown draft credits a PR author, contributor, or issue reporter, render the username as a GitHub profile link such as `[@username](https://github.com/username)`.

### Step 6 — Classify unmarked PRs

Use a two-pass workflow with `classify_pr.py` to classify PRs that have no explicit `CHANGELOG-*` entries.

**Pass 1 — Deterministic preclassification:**

Run the script to apply mechanical exclusion rules and channel flag gates. It emits a `classifications` list (deterministic excludes) and an `agent_required` list (candidates needing subjective judgment).

```bash
python3 .agents/skills/changelog-draft/scripts/classify_pr.py \
  --channel "${channel}" \
  --prs-json prs.json \
  --feature-flags-json feature_flags.json \
  --contributors-json contributors.json \
  --output preclassifications.json
```

The `agent_required` list contains PRs where the script determined that user-visibility, category choice, and changelog text require subjective judgment. Classify each candidate using the guidance in `.agents/skills/classify-changelog-pr/SKILL.md`. Produce a JSON array of agent classifications:

```json
[
  {
    "pr_number": 1234,
    "include": true,
    "category": "IMPROVEMENT",
    "text": "Proposed changelog line",
    "confidence": "high",
    "rationale": "...",
    "needs_review": false
  }
]
```

Save this to `agent_classifications.json`.

Before saving classifications, edit every included entry as release copy:

- Lead with the user-visible outcome, not implementation details. Replace internal terms such as protocol names, stream topology, framework types, or feature-flag mechanics with the behavior users will notice.
- Use concise past tense: `Added`, `Improved`, `Fixed`, `Removed`, `Clarified`, or a direct outcome such as `Warp now ...`. Do not begin entries with imperative `Add`, `Fix`, or `Clarify`.
- Correct obvious spelling, grammar, and preposition errors. For example, use “Added the installation path to the Windows App Paths Registry,” not “into de Windows App Paths Registry.”
- Preserve literal product names, commands, settings labels, and platform names.
- Do not include placeholders such as `{{...}}`, raw `CHANGELOG-*` prefixes, internal repository references, or private PR links.
- Prefer one user-facing outcome per entry. When several PRs are implementation stages for one feature, select the completed outcome rather than listing every stage.
- Rank `OZ` entries by user impact. Only the first four are published, so place the four most important Oz updates first.

**Pass 2 — Validate and merge:**

Re-run the script with the agent classifications to produce the final classifications. Mechanical excludes always win over agent answers — conflicts cause a non-zero exit.

```bash
python3 .agents/skills/changelog-draft/scripts/classify_pr.py \
  --channel "${channel}" \
  --prs-json prs.json \
  --feature-flags-json feature_flags.json \
  --contributors-json contributors.json \
  --agent-classifications agent_classifications.json \
  --output classifications.json
```

Save the output as `classifications.json` for use in Step 7.

### Step 7+8 — Assemble the draft and write output files

Run `assemble_changelog.py` to combine all intermediate artifacts into the two output files. The script enforces the accounting invariant using unique PR numbers and exits non-zero if any PR appears in multiple buckets or is missing. A PR with multiple explicit markers creates multiple audit entry records but still counts as one unique PR.

```bash
python3 .agents/skills/changelog-draft/scripts/assemble_changelog.py \
  --channel "${channel}" \
  --range-json range.json \
  --prs-json prs.json \
  --contributors-json contributors.json \
  --issue-reporters-json issue_reporters.json \
  --classifications-json classifications.json \
  --output-dir "${output_dir}" \
  --attribution "${attribution:-external-only}"
```

The script writes two files to `output_dir`:

**`changelog-draft.md`** — Human-reviewable markdown ready for Slack/Notion. Contains only the changelog sections (New Features, Improvements, Bug Fixes, Oz Updates) and the Community attribution section. Does **not** include Needs Review or Skipped PR sections. The Oz Updates section is capped at four entries, preserving the curated source order.

**`changelog-draft.json`** — Machine-readable audit artifact retaining `entries`, `skipped`, `needs_review`, and `issue_reporters`. Every PR in the range appears in exactly one of these buckets.

### Step 9 — Generate release-pipeline JSON

Run the conversion script to deterministically produce `changelog-release.json` from the audit artifact:

```bash
python3 .agents/skills/changelog-draft/scripts/convert_to_release_json.py \
  --input <output_dir>/changelog-draft.json \
  --output <output_dir>/changelog-release.json
```

This produces the flat JSON structure consumed by the `create_release` workflow for Slack and the in-app "What's New" dialog. The converter deterministically caps `oz_updates` at four entries, preserving source order. Do **not** generate this file manually — always use the script so the output is deterministic and consistent.

## Constraints

- **Never** write to `channel_versions.json` or any production config file.
- **Never** push commits, create branches, or open PRs.
- All output goes to `output_dir` only.
- The markdown draft should be copy-pasteable into Slack or Notion for review.
- Keep the JSON artifact complete enough for audit: every PR in the range should appear in either `entries`, `skipped`, or `needs_review`.

## Validation

After generating output, verify:
1. Every unique PR number in the range is accounted for. Record counts may exceed unique PR counts when a PR has multiple explicit markers.
2. Explicit marker entries match what `fetch_prs.py` extracted (no dropped markers).
3. No duplicate PR numbers across sections.
4. Markdown and release JSON contain at most four Oz updates in the same order.
5. User-facing copy uses past tense, explains outcomes rather than implementation details, and contains no placeholders, raw marker prefixes, private links, or internal repository references.
6. The markdown renders cleanly (no broken links or formatting).
