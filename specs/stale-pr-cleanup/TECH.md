# Stale PR Follow-up for Requested-Changes Reviews — Tech Spec
Product spec: `specs/stale-pr-cleanup/PRODUCT.md`
> Code references pinned to `warp` @ `ac4225c`.

## Problem
We need a periodic sweep that finds open external-contributor PRs with an active requested-changes review, nudges the author after a window of inactivity, and closes the PR if inactivity continues. There is no "PR went quiet for N days" webhook event, so this is a scheduled scan. It lives in `warpdotdev/warp` as a GitHub Action, mirroring the existing stale-PR cleanup workflow, and posts as `github-actions[bot]` via the default `GITHUB_TOKEN`.

## Relevant Code (`warp` @ `ac4225c`)
- `.github/workflows/close_stale_fix_prs.yml` — the pattern to mirror: a daily `schedule:` + `workflow_dispatch:` job running a single pinned `actions/github-script` step that paginates open PRs, filters, comments, and closes via the default `GITHUB_TOKEN`.
- `.github/workflows/label_external_contributors.yml` — applies the `external-contributor` label to fork PRs on open (skips bots); this is the v1 scope filter.
- `.github/workflows/check_approvals.yml` — precedent for reading PR review state inside a workflow (`gh pr view --json reviews --jq '.reviews[] | select(.state==...)'`).

## Approach
Add one scheduled GitHub Action that mirrors `close_stale_fix_prs.yml`. An `actions/github-script` step does detection, reminders, and closure — its logic factored into a standalone `.github/scripts/` JS module the step `require`s, rather than inline YAML — and reminder state lives in marker comments (no datastore). This was chosen over hosting the scan in the `oz-for-oss` control plane — see "Alternative considered." The only thing the `warp`-hosted GHA gives up is native `oz-for-oss` comment authorship, which `PRODUCT.md` invariant 9 treats as non-functional and deferred.

## Proposed Changes

### 1. New workflow `.github/workflows/stale_requested_changes_prs.yml`
- Triggers: `schedule:` with cron `7 12 * * *` — fields are `minute hour day-of-month month day-of-week`, so this is **12:07 UTC daily** (GitHub interprets cron in UTC). Minute `7` rather than `:00` avoids GitHub's top-of-hour scheduler congestion and the existing warp crons there. Also `workflow_dispatch:` with a `mode` input (`dry-run` | `reminder-only` | `full`).
- Least-privilege `permissions:` — `pull-requests: write`, `issues: write`, `contents: read` (note `close_stale_fix_prs.yml` declares none; do better here).
- Two steps: a sparse `actions/checkout` (cone limited to `.github/scripts`, `blob:none`) that makes the script available, then an `actions/github-script` step pinned to the same SHA already used by `close_stale_fix_prs.yml` that `require`s `.github/scripts/stale-requested-changes-prs.js` and invokes it with `{ github, context }`. The detection/reminder/closure logic lives in that standalone module for reviewability and `node`-based testing.

### 2. Eligibility + review state (in github-script JS)
Paginate open PRs; keep those that are non-draft, carry `external-contributor`, and lack the exemption label. Compute the latest *decisive* review state per PR from the reviews API — track `APPROVED` / `CHANGES_REQUESTED` / `DISMISSED` separately from `COMMENTED` so a later author `COMMENTED` reply doesn't look like a state change — and keep only PRs whose decisive state is `CHANGES_REQUESTED` (Oz- or human-authored). This re-implements `oz-for-oss`'s `_is_stale_oz_changes_requested_review` decision in JS; keep it small and documented.

### 3. Inactivity computation (author-driven)
`last_activity_at = max(server-recorded head-branch push, last comment by the author)`. The push time comes from GitHub-controlled signals via GraphQL — the head commit's `pushedDate` and `HeadRefForcePushedEvent.createdAt` — not commit author/committer dates (which the contributor controls), so a backdated or force-pushed commit still resets the timer. "Author comments" covers issue comments and inline review-thread replies; the author can't formally review their own PR, so there is no separate author-review signal. Comments by maintainers or other third parties do **not** count, nor do automation accounts (`github-actions[bot]`, `oz`, `oz-for-oss`) or bare `updated_at`. Rationale (`PRODUCT.md` invariant 2): the lifecycle chases author follow-up, so a maintainer comment must not reset the clock. Derive inactive days; cadence = reminders at 7 / 14 / 26 days, close at 30 (named constants).

### 4. Reminder state via marker comments (no datastore)
Each reminder body embeds a hidden marker, e.g. `<!-- stale-requested-changes:stage=7 -->`. The PR's comments are already fetched for the activity calc, so the markers come for free. **Only markers on comments authored by `github-actions[bot]` are trusted** — otherwise anyone could comment the final-warning marker to trigger a close without a warning. A stage counts as "already sent" only when such a marker comment was created at or after `last_activity_at`; older markers belong to a prior window and are ignored — giving the timer reset for free. **Closure additionally requires the day-26 final-warning marker to exist in the current window** (`PRODUCT.md` invariant 6), so no PR is closed without a prior warning — including the existing backlog on first enablement, which is warned first and closed on a later run. Closure otherwise needs no state: a closed PR leaves the open-PR scan. If a reminder is deleted the bot may re-post that stage — benign, since closure re-verifies review state and recomputes inactivity.

### 5. Identity
Comments and the close call use the default `GITHUB_TOKEN`, so they are authored by `github-actions[bot]`. On a `schedule` event the token runs in the base-repo context, so it can comment on and close fork PRs (unlike a fork `pull_request` trigger). Posting as `oz-for-oss` is deferred (Follow-ups).

### 6. Rollout mode
The `mode` input gates writes: `dry-run` logs eligible PRs + intended actions, `reminder-only` posts reminders but never closes, `full` does both. Scheduled runs default to `full` (the no-close-without-final-warning guard in #4 prevents surprise bulk closures of the existing backlog); `dry-run` and `reminder-only` remain available via `workflow_dispatch` for validating changes.

## Alternative considered: `oz-for-oss`-hosted scan
Run the scan in the `oz-for-oss` Vercel control plane instead. Upside: comments are natively authored by the `oz-for-oss` App, and it can reuse the Python review-state helpers (`_is_stale_oz_changes_requested_review`, decisive-state handling) rather than re-implementing them in JS. Downside: it moves `warp` PR-closure governance into a separate backend, and its only decisive advantage is the comment identity — which we've deemed non-functional. Deferred; revisit only if `oz-for-oss` comment authorship becomes a hard requirement.

## Testing and validation
Keep it light, consistent with repo norms (`close_stale_fix_prs.yml` ships with no tests).
- Validate via `workflow_dispatch` in `dry-run` against the live repo first, then `reminder-only`, before relying on scheduled `full` runs.
- If unit coverage is wanted, factor the pure decision function (PR metadata + `now` → action) into a small JS module beside the workflow and test it with `node`, covering: author activity resets the timer but a maintainer comment does not (`PRODUCT.md` invariant 2); each stage fires once; and closure requires the day-26 final-warning marker (invariant 6). Otherwise rely on the staged rollout.

## Risks and mitigations
- **Wrong activity signal closes an active PR.** Compute inactivity from GitHub-controlled push timestamps (`pushedDate` / force-push events) and author comment timestamps, never `updated_at` or contributor-controlled commit dates; exclude automation authors so the bot's own reminders never count.
- **Marker spoofing.** A contributor could paste the final-warning marker to fake a warning. Mitigated by only trusting markers on `github-actions[bot]`-authored comments.
- **Duplicate reminders / re-close.** Dedup off existing markers; closed PRs leave the scan; closure re-verifies eligibility.
- **Reminder comment deleted or edited away.** The bot may re-post that stage — benign; missing markers can never cause a wrongful close.
- **Logic drift from `oz-for-oss`.** The decisive-review-state rule is re-implemented in JS; keep it minimal, documented, and aligned with `_is_stale_oz_changes_requested_review`.
- **Author waiting on a maintainer.** Because only author activity resets the timer, a PR the author already responded to can still march to closure. Mitigation: maintainers apply `no-autoclose`; revisit the pause-on-author-response option (`PRODUCT.md` open question) if this proves noisy.

## Follow-ups
- **(Non-functional) Author comments as `oz-for-oss`.** Would require minting an `oz-for-oss` App installation token from the App private key placed in `warp` Actions secrets, or calling an `oz-for-oss` endpoint. Deferred; a possible future nicety.
- Generalize beyond `external-contributor` or to additional repos.
- Optionally backfill a least-privilege `permissions:` block onto `close_stale_fix_prs.yml` while we're here.
