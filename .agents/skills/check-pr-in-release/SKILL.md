---
name: check-pr-in-release
description: Verify whether a merged pull request or commit from the public repo warpdotdev/warp has shipped in the latest preview or stable Warp release. Use this skill whenever the user asks "is this PR/commit in the latest preview/stable release", "did this change ship to stable/preview", "which release includes PR #X", "verify a merged warp PR made it into a release", or otherwise wants to confirm release inclusion of a warpdotdev/warp change — even if they don't name the exact channel. Also use it when the user mentions a cherry-pick and wants to know if it landed in a release.
---

# Check whether a public warp PR shipped in a release

This skill confirms whether a merged PR (or commit) from the **public** repo `warpdotdev/warp` is present in the latest `preview` or `stable` Warp release. Releases are cut from the **private** repo `warpdotdev/warp-internal`, so the work is mostly about following a public merge through the repo-sync mirror and into a release branch.

## The one pitfall that breaks everything

The public (`warpdotdev/warp`) and private (`warpdotdev/warp-internal`) repos have **completely different commit SHAs**. The local checkout at `/Users/evelyn/Documents/repos/warp` is the **public mirror** — its `origin` is `git@github.com:warpdotdev/warp.git`.

Because of this:

- warp-internal merge SHAs and `*_release/*` branches **do not exist** in the local object database.
- Local git commands against internal SHAs/branches fail, e.g.
  `git merge-base --is-ancestor <internal-sha> origin/stable_release/...` or
  `git cat-file -t <internal-sha>` → `fatal: Not a valid commit name`.

**Therefore: drive every warp-internal check through `gh api` / `gh pr` / `gh run` against `warpdotdev/warp-internal`.** Assume there is **no** local warp-internal checkout. Only fall back to local git if the user explicitly tells you they have a warp-internal checkout.

## Prerequisites

- `gh` must be authenticated with an account that has read access to the **private** `warpdotdev/warp-internal` repo. Verify with `gh auth status`. If `gh api repos/warpdotdev/warp-internal/...` returns 404/403, stop and report that the token lacks warp-internal access — the check cannot proceed.

## Inputs to collect

1. The **public** warp PR number, PR URL, or merge commit SHA.
2. The **target channel**: `preview` or `stable`.
3. Optional: whether the change reached the release via a **cherry-pick** (changes the matching strategy — see step 6).

## Procedure

### Step 1 — (Conditional) Confirm the target channel even builds the change

Some code is compiled only for specific channels via feature flags, so a change can be "merged" yet absent from a given channel's binary. Channel targeting lives in three bundle scripts (there is no separate `oss` script — `oss` is a `RELEASE_CHANNEL` value handled inside these):

- `script/linux/bundle`
- `script/macos/bundle`
- `script/windows/bundle.ps1`

Inspect how `FEATURES` is set per channel (`local`, `dev`, `preview`, `stable`, `oss`):

```bash
grep -n "RELEASE_CHANNEL\|FEATURES" script/macos/bundle
```

For example, `preview_channel` is added only for `preview`, and debug-only features like `agent_mode_debug` only for `local`/`dev`. If the change is gated behind a channel-specific feature flag, confirm the target channel actually enables it. **If the change is not gated by any channel-specific flag, this step is N/A** — code ships to all channels by default.

### Step 2 — Find the mirror PR in warp-internal

The repo-sync bot (`app/warp-repo-sync`) mirrors each public merge onto warp-internal `master`, usually within minutes, on a branch named `repo-sync/public-to-private/<short-public-merge-sha>`. The mirror PR keeps the **same title** as the public PR, and that title typically embeds the public PR number, e.g. `... (#10596)`.

Find it by title (most reliable):

```bash
gh pr list --repo warpdotdev/warp-internal --state all \
  --search "<public PR title>" \
  --json number,title,state,headRefName,url,mergedAt,author --limit 10
```

Or scope the search to repo-sync branches (handy when matching by public PR number in the title):

```bash
gh pr list --repo warpdotdev/warp-internal --state all \
  --search "head:repo-sync/public-to-private <public PR # or title>" \
  --json number,title,headRefName,mergedAt,url --limit 10
```

Get the internal merge commit and timestamps:

```bash
gh pr view <internal-pr-#> --repo warpdotdev/warp-internal \
  --json mergeCommit,mergedAt,baseRefName,state,title
```

`mergeCommit.oid` is the **internal SHA** used for the ancestor test below. The internal master history is at https://github.com/warpdotdev/warp-internal/commits/master/.

### Step 3 — Find the latest cut release for the target channel

List recent release branches (their names encode the cut time as `<channel>_release/v0.YYYY.MM.DD.HH.MM.<channel>`):

```bash
gh api "repos/warpdotdev/warp-internal/git/matching-refs/heads/stable_release" -q '.[].ref' | sort | tail -n 5
gh api "repos/warpdotdev/warp-internal/git/matching-refs/heads/preview_release" -q '.[].ref' | sort | tail -n 5
```

The newest ref (last line) is the latest cut. To corroborate against the cut workflow (`cut_new_releases.yml`):

```bash
gh run list --workflow cut_new_releases.yml --repo warpdotdev/warp-internal \
  --json databaseId,headBranch,displayTitle,status,conclusion,createdAt -L 10
```

The authoritative release timestamp is the **tip commit** of the release branch:

```bash
gh api repos/warpdotdev/warp-internal/commits/<release-branch> -q '.commit.committer.date'
```

(`<release-branch>` is the path after `refs/heads/`, e.g. `stable_release/v0.2026.06.10.09.27.stable`.)

### Step 4 — Timestamp sanity check (NOT authoritative)

Compare the internal mirror PR `mergedAt` against the release cut time from step 3. Merged-before-cut suggests inclusion, but timing alone is never conclusive — always confirm with step 5.

### Step 5 — Ancestor test (AUTHORITATIVE)

Ask GitHub whether the internal merge commit is contained in the release branch:

```bash
gh api "repos/warpdotdev/warp-internal/compare/<internal-merge-sha>...<release-branch>" -q '.status'
```

Interpretation:

- `ahead` or `identical` → the commit **IS** in the release.
- `diverged` or `behind` → the commit is **NOT** in the release.

Optional cross-check via the delta between the previous and target release branches (the `.commits[]` list is populated even when `.status` is `diverged`):

```bash
gh api "repos/warpdotdev/warp-internal/compare/<prev-release-branch>...<release-branch>" \
  --paginate -q '.commits[].sha' | grep <internal-merge-sha>
```

A match means the commit is part of what the new release added relative to the previous one.

### Step 6 — Cherry-pick case (ONLY when the user says it was cherry-picked)

A cherry-pick produces a **different SHA**, so the step 5 ancestor test on the original SHA will report "not included" even though the change is physically present. Match by the PR-number reference or commit subject instead:

```bash
gh api "repos/warpdotdev/warp-internal/compare/<prev-release-branch>...<release-branch>" \
  --paginate -q '.commits[] | select(.commit.message | contains("#<internal-pr-#>") or contains("<subject snippet>")) | .sha'
```

A returned SHA is the cherry-picked commit on the release branch. (If — and only if — a warp-internal checkout is available, content-based tools like `git cherry`, `git log --grep`, or `git patch-id` compare by content rather than SHA.)

## Report format

Do **not** write a markdown file — print the findings directly:

- **Conclusion:** YES / NO that the target change is in the target release.
- **Mirror PR:** warp-internal mirror PR link and the internal merge commit SHA.
- **Timestamps:** the mirror PR/commit timestamp AND the stable/preview release timestamp.
- **Ancestor check:** the returned status (`ahead` / `identical` / `diverged` / `behind`).
- **If cherry-pick:** the exact command run and the SHA (or empty result) it returned.
