# Alert Maintainers When There Are Duplicates Of A PR (GH-10395)

## Spec layout note

This spec is delivered as a single `SPEC.md` file but is internally
partitioned into the same product / tech contract sections that the
repository's spec convention expects to find in `PRODUCT.md` and
`TECH.md` siblings:

- **Product contract** (what the user / maintainer experiences): `Summary`,
  `Problem`, `Goals`, `Non-Goals`, `Behavior Contract` sections B1–B5
  (UI surface, detection signals, dismissal UX, detection cadence), the
  `Acceptance Criteria` block, and the `Open Questions` / `Telemetry`
  sections.
- **Tech contract** (how the system must behave, including security and
  state): `Behavior Contract` sections B-Secret (secret-material
  handling), B-Dedup (server-side dedupe + TTL), B3a (per-pair signal
  expansion vs aggregated payload), B4a (dismissal trust model), B4b
  (marker write / scan / reversal mechanics), the `Settings / API
  surface` table, `Implementation Pointers`, and `Tests`.

If reviewers prefer a strict two-file layout, the partition above is the
authoritative split; a follow-up cleanup may extract them into
`specs/GH10395/PRODUCT.md` and `specs/GH10395/TECH.md` without any
content change. The single-file form is retained for V1 to keep the
review thread anchored on one document.

## Summary

Surface a non-blocking maintainer alert inside the Warp Code Review experience when multiple open PRs appear to target the same underlying issue or duplicate each other's diff. Provide a cross-link list of candidate duplicates, one-click navigation between them, and emit a structured signal that can be routed to Slack or Linear so maintainers are notified outside Warp.

## Problem

Maintainers regularly receive multiple PRs for the same issue — particularly during hackathons, bug-bash days, or when an issue sits open long enough for several contributors to pick it up independently. Today there is no in-product signal that a PR being reviewed is one of several candidates. Reviewers either discover the duplicate by accident (a teammate mentions it, a CI conflict surfaces) or duplicate review effort is spent on PRs that will ultimately be closed in favor of another. This wastes reviewer time and is a poor experience for the contributor whose PR gets closed late.

## Goals

- Detect candidate-duplicate PRs using lightweight signals that do not require a backend index or LLM call.
- Surface a "Possible duplicates" callout in the Warp Code Review pane with the candidate PRs, the signal that triggered the match, and one-click navigation between them.
- Emit a structured `MaintainerAlert.DuplicatePr` signal that team admins can route to a Slack webhook and/or Linear issue.
- Allow per-PR dismissal of false positives so the same noise does not return on every open.
- Provide a per-user setting and team-level signal-destination configuration; both default to enabled but with no destinations configured.

## Non-Goals

- Auto-closing duplicate PRs. The maintainer always decides.
- Server-side merge orchestration or rebase automation.
- Cross-repo duplicate detection (V1 is single-repo; tracked as Open Question).
- Language-aware semantic duplicate detection (e.g., comparing AST diffs). V1 uses path-set and title token similarity only.
- Replacing or modifying the existing GitHub branch comparison view.

## Behavior Contract

### B1. Detection signals

Any one of the following triggers candidate-duplicate state for the open PR:

1. **Same issue reference.** Another open PR in the same repo references the same issue via `Closes #N`, `Fixes #N`, or `Resolves #N` (case-insensitive) in its body.
2. **File-overlap.** For PRs touching ≥3 files, Jaccard similarity of changed-file path sets ≥ 0.50 with another open PR.
3. **Small-PR heuristic.** For PRs touching 1–2 files, all changed paths match another open PR AND title TF-IDF cosine similarity ≥ 0.70.
4. **Rapid-fire same author.** Same author submitting more than one open PR within a 24-hour window where the changed-file path sets have Jaccard similarity ≥ 0.30 (path-only overlap, identical to the metric used in B1.2). V1 explicitly does **not** compute line-level diff overlap; the term `diff overlap` is defined here as `|paths(A) ∩ paths(B)| / |paths(A) ∪ paths(B)|` over the changed-file path sets only. (Captures force-push-as-new-PR cases.) Line-level overlap is deferred to V1.5 (see Open Questions).

### B2. Surface in Code Review pane

A "Possible duplicates" callout renders above the diff, collapsed by default with the candidate count visible (e.g., "Possible duplicates (2)"). Expanded, it lists each candidate row with:

- PR number, title, author avatar+handle.
- Signal label that triggered the match (e.g., "References same issue #1234", "62% file overlap", "Same author within 24h").
- Click target: opens that PR in a new Code Review pane (does not replace the current one).

### B3. Maintainer signal

When ≥1 candidate is detected for a PR, emit `MaintainerAlert.DuplicatePr { pr_number, candidate_pr_numbers, signal_types }`. Configured destinations are read from team-level settings; admins configure once. Supported destinations:

- Slack webhook URL.
- Linear issue creation using a configured template.

If both are configured, both fire. If neither is configured, no external signal fires (the in-product callout still renders).

#### B3a. Per-pair signal expansion vs aggregated payload

The detector produces an aggregated callout (one row per candidate in the in-product UI), but the **external signal** is conceptually a SET of per-pair tuples. Because the dedupe / cooldown key in B-Dedup is keyed on `({pr_a, pr_b}, signal_type)` (a per-pair, per-signal-type tuple), the aggregated payload MUST be FANNED OUT into per-pair tuples BEFORE dedupe is consulted. Mixed runs — where one detection run yields some pairs that are fresh and others that are within their cooldown window — are resolved as follows:

1. **Compute the candidate set.** The detector produces the full set of `(pr_number, candidate_pr_number, signal_type)` triples for the current run. Each triple is canonicalized to `({pr_a, pr_b}, signal_type)` per B-Dedup.
2. **Filter against dedupe.** For each canonical tuple, look up the server-side dedupe store. Tuples within TTL are MARKED `suppressed`; tuples outside TTL (or never seen) are MARKED `fresh`.
3. **Emit only the fresh subset.** The external signal is emitted **only if the fresh subset is non-empty**. The payload SHALL include only the fresh tuples — suppressed tuples MUST NOT appear in the emitted payload, even though they remain in the in-product callout. Specifically: `candidate_pr_numbers` and `signal_types` in the emitted `MaintainerAlert.DuplicatePr` are projections of the FRESH subset only.
4. **Record dedupe state atomically.** The dedupe store is updated for each fresh tuple in the same transaction (or equivalent atomic operation) as the emit. If the emit fails (network/transport error), the dedupe store MUST NOT be advanced for any tuple in that emit — the entire fresh subset is retried on the next detection run. Partial dedupe writes are forbidden.
5. **No empty-payload emits.** If every tuple in the run is suppressed (all within TTL), no external signal fires. The in-product callout still renders the full candidate set including the suppressed pairs.
6. **Multi-signal-type collisions.** A single `(pr_a, pr_b)` pair that matches under multiple signal types (e.g., both `same_issue_ref` and `file_overlap`) produces ONE tuple per signal type. Each tuple is independently deduped. The emitted payload's `signal_types` field is the union of signal types whose tuples were in the fresh subset.

Worked example:

> Run 1: pair `{12, 17}` matches `same_issue_ref`; pair `{12, 24}` matches `file_overlap`. Both are fresh. Emit one external signal with `pr_number=12`, `candidate_pr_numbers=[17, 24]`, `signal_types=[same_issue_ref, file_overlap]`. Record both tuples in the dedupe store.
>
> Run 2 (4 days later): pair `{12, 17}` still matches `same_issue_ref` (suppressed — within TTL); pair `{12, 33}` newly matches `file_overlap` (fresh). Emit ONE external signal with `pr_number=12`, `candidate_pr_numbers=[33]`, `signal_types=[file_overlap]`. The `{12, 17}` pair does NOT appear in the payload. Both pairs continue to appear in the in-product callout.

### B4. Suppression

The callout exposes a "Mark as not duplicate" button per candidate. Clicking it records a dismissal as a structured PR comment with marker `<!-- warp-dup-dismiss pr=<candidate_pr_number> -->`. On subsequent opens, the dismissed candidate is filtered out for that (PR, candidate) pair only. Dismissals are bidirectional — dismissing on PR A also hides PR A from PR B's candidate list.

#### B4b. Marker write and scan mechanics

The marker is a one-sided write (the user is acting on one PR's callout at a time), but it MUST behave bidirectionally on read. The contract:

- **Write side — exactly one marker comment per click, on the SOURCE PR only.**
  - Clicking "Mark as not duplicate" on PR A's callout for candidate PR B posts EXACTLY ONE PR comment on PR A with body `<!-- warp-dup-dismiss pr=B -->` (plus a brief human-readable suffix on the SAME comment for audit clarity, e.g., `<!-- warp-dup-dismiss pr=B --> Dismissed as not duplicate by @username`). No second comment is written on PR B; the cross-PR effect comes from the scan side below.
  - If a marker for the same `(A, B)` pair already exists on PR A (e.g., user double-clicks, or a previous session wrote one), the UI is a no-op — no duplicate marker comment is written. The detector MUST treat any number of identical markers as equivalent to one.
  - The marker comment is written via the GitHub Issues Comments API on the source PR's issue endpoint (PRs share the issue comment surface with the PR's discussion thread). Comments are written in plain text; no Markdown rendering hazard since the marker is an HTML comment.
- **Scan side — both PRs in the pair are scanned at detection time.**
  - For a candidate pair `(A, B)`, the detector fetches and scans the comment list of BOTH PR A and PR B. A trusted marker (per B4a) for the OTHER PR in the pair on EITHER side suppresses the pair. Specifically:
    - A marker `<!-- warp-dup-dismiss pr=B -->` on PR A by a trusted author suppresses the `(A, B)` pair.
    - A marker `<!-- warp-dup-dismiss pr=A -->` on PR B by a trusted author also suppresses the `(A, B)` pair.
    - Either side is sufficient; the pair is suppressed if at least one trusted marker exists on either side. This is what "bidirectional" means in B4 — the EFFECT is symmetric on read, not that two write operations happen.
  - The scan reads up to the most recent 100 comments per PR (the GitHub default page size); paginate if needed to cover older markers. Comments are scanned in reverse chronological order.
- **Reversal — explicit, by writing a counter-marker on the same PR.**
  - To reverse a dismissal, a trusted user writes a counter-marker comment on the SAME PR that previously dismissed, with body `<!-- warp-dup-undismiss pr=<candidate_pr_number> -->`. Reversal markers are subject to the same B4a trust model (`permission ∈ { admin, maintain, write }`).
  - At scan time, the detector resolves the pair's dismissal state by reading ALL `warp-dup-dismiss` and `warp-dup-undismiss` markers for the pair across both PRs and taking the MOST RECENT trusted marker by `created_at` timestamp as authoritative. A `warp-dup-undismiss` newer than every `warp-dup-dismiss` for the pair re-surfaces the candidate.
  - The UI exposes reversal as an "Un-dismiss" affordance on the suppressed candidate (visible to trusted users only); clicking it writes the `warp-dup-undismiss` marker.
- **Write count summary.**
  - Initial dismissal: 1 comment written on the SOURCE PR.
  - Reversal: 1 additional comment written on whichever PR the user is acting from (typically the same PR as the original dismissal, but either side is accepted because both are scanned).
  - Re-dismissal after reversal: 1 additional `warp-dup-dismiss` comment. The scan always uses the most recent trusted marker.
  - At most, a single pair accumulates one comment per state transition; there is no compaction in V1.

#### B4a. Dismissal trust model

The `<!-- warp-dup-dismiss pr=N -->` marker is **only respected when the comment author has actual `push` (write) permission or higher on the repository at the time the marker is evaluated**. This prevents external contributors — and lower-privileged org members — from forging suppression markers to hide duplicate alerts from maintainers.

**Authoritative permission check (required).** Trust is determined by an explicit per-author repository permission lookup, **not** by `author_association` alone. `author_association` only describes the author's relationship to the PR's discussion thread (e.g., `MEMBER` = member of the org that owns the repo, which does **not** imply repository write access). The detector calls:

```
GET /repos/{owner}/{repo}/collaborators/{username}/permission
```

and treats the marker as trusted **only if** the response's `permission` field is one of `{ admin, maintain, write }`. The values `triage`, `read`, and `none` are rejected.

The result of this lookup is cached per `(repo, username)` for 10 minutes to bound API cost; cache misses re-fetch on the next detection run.

**`author_association` is used only as a fast-path negative filter** — comments with `author_association ∈ { CONTRIBUTOR, FIRST_TIME_CONTRIBUTOR, FIRST_TIMER, NONE, MANNEQUIN }` are rejected without making the permission API call, since these associations cannot correspond to write access. Comments with `author_association ∈ { OWNER, MEMBER, COLLABORATOR }` proceed to the authoritative `permission` lookup before being trusted.

Comments whose author fails the permission check carrying the same marker text are **IGNORED** — the candidate continues to surface in the callout as if the marker were absent. The detector logs a sanitized debug entry (no comment body; only `pr_number`, `author_association`, and resolved `permission`) when a non-trusted marker is encountered.

The "Mark as not duplicate" UI button is only enabled for users whose GitHub identity passes the same `permission ∈ { admin, maintain, write }` check against the repo; users without write permission see the button disabled with tooltip "Requires repo write access".

### B-Secret. Slack webhook secret-material handling

The Slack incoming-webhook URL is treated as **secret material** and follows team-secret-grade controls.

**Storage.**
- The webhook URL is stored in the team-level encrypted secret store, never in plain TOML, never in user-readable config files, never inline in this repo's settings tree.
- If Warp already has a `team_secrets`-style schema, the webhook is persisted there. If no such schema exists yet, this spec marks it as `(new) team-level encrypted secret store` infrastructure that must be in place before the feature ships.
- Encrypted at rest using the team's existing key material (envelope encryption against the team key); never written to disk in cleartext.

**Access control.**
- Only users with **team admin** role can READ or WRITE the webhook value.
- Non-admin team members see a redacted display only: `"•••••• (configured)"` if a webhook is set, `"(not configured)"` otherwise. The actual URL is never returned by the API to non-admin clients.
- Admin reads of the webhook (e.g., to copy it for rotation) are themselves an audited event in telemetry: `team.duplicate_alert.webhook_revealed { admin_user_id }`.

**Transmission.**
- The webhook is invoked over **TLS only**. Plain HTTP webhook URLs are rejected at save time with a validation error.
- **Host allowlist (anti-SSRF).** The webhook URL's host MUST match the Slack incoming-webhook host set, which is the EXACT pattern `hooks.slack.com` (case-insensitive). Subdomains, alternate Slack hosts, or unrelated hosts are rejected at save time AND at request time:
  - At **save time**, the URL is parsed (RFC 3986); the host component is compared against the allowlist; any non-match returns a validation error with the message `"Webhook URL host must be hooks.slack.com"`. The path component MUST also begin with `/services/` (the canonical Slack incoming-webhook path prefix); other paths are rejected.
  - At **request time** (immediately before the outbound HTTP call), the URL is re-parsed and re-validated against the same allowlist. This second check defends against TOCTOU between save and emit — a stored value that fails request-time validation is treated as misconfigured (the emit is dropped, telemetry records a `team.duplicate_alert.webhook_invalid` event, no fallback host is contacted).
  - The allowlist is a small, hard-coded constant in the emitter module. There is NO admin-configurable bypass, NO env-var override, and NO "trusted hosts" list.
- **SSRF-safe HTTP client configuration.** The HTTP client used for Slack emission MUST be configured with the following constraints, separately from Warp's general-purpose HTTP clients:
  - **Redirect handling: disabled.** The client MUST NOT follow HTTP redirects. A `3xx` response is treated as a failed emit (logged via the redacted sanitizer); the response body is dropped. Following redirects would let a (hypothetically) compromised Slack response steer the request to an internal host.
  - **DNS resolution: public-only.** The resolved IP for `hooks.slack.com` MUST NOT fall into RFC 1918 private space (`10.0.0.0/8`, `172.16.0.0/12`, `192.168.0.0/16`), loopback (`127.0.0.0/8`, `::1`), link-local (`169.254.0.0/16`, `fe80::/10`), unique-local IPv6 (`fc00::/7`), multicast, or the IPv4-mapped IPv6 equivalents of any of the above. If the resolved address falls into any of these ranges, the emit is dropped and telemetry records a `team.duplicate_alert.webhook_blocked_address` event. This guards against DNS rebinding and against any future allowlist host that could be coerced via DNS.
  - **Timeouts: bounded.** Connect timeout ≤ 5s, total request timeout ≤ 10s. Slow-loris responses cannot tie up the emitter indefinitely.
  - **Method: POST only.** The emitter only issues POST; the HTTP client MUST NOT permit method override via the response.
  - **No proxy fallback.** The emitter MUST NOT honor `HTTP_PROXY` / `HTTPS_PROXY` env variables that would route the request through an attacker-controlled proxy. The Slack client uses direct outbound only.
- **Arbitrary outbound webhooks: explicitly NOT supported in V1.** This spec deliberately does NOT support arbitrary HTTP/HTTPS webhook destinations. Routing to anything other than `hooks.slack.com` requires a future spec that defines an explicit destination registry, per-host SSRF posture, and an admin-controlled allowlist; until that lands, the validation above is the authoritative constraint.
- The webhook URL is **never** included in logs, telemetry payloads, error messages, panic traces, or crash reports — emit error metadata stripped of URL components. A dedicated sanitizer wraps every log/error path that could touch the URL: it replaces the URL with `[redacted-slack-webhook]`.

**Rotation.**
- Admins may replace the webhook value at any time via team settings. The previous value is overwritten in place — there is no history kept and no way for the API to return prior values.
- Rotation takes effect immediately for the next signal emission; in-flight emissions complete against whichever value they captured at request time.
- Setting the value to empty string disables external Slack signaling entirely (the in-product callout still renders).

**Linear template handling.**
- The Linear issue template is **not** treated as secret material (it is a body template with placeholders, not a credential). However, it is admin-write / team-read so non-admins cannot inject content into outgoing Linear issues.

### B5. Detection cadence

Detection runs:

- On Code Review pane open for a PR.
- When a PR-reviewed event fires upstream and other open PRs touching overlapping paths exist.
- Hourly in the background while the review pane stays open.

Each run is bounded to the PRs returned by GitHub's open-PR list for the repo (capped at the most recent 200 open PRs).

## Settings / API surface

User-level (Settings → Code Review → "Duplicate PR alerts"):

- `code.review.duplicate_alert.enabled` (bool, default `true`) — when `false`, suppresses the in-product "Possible duplicates" callout in the Code Review pane for **this user only**. This setting has **no effect** on team-level external signal emission (Slack/Linear); see the independence rules below for the authoritative scope of each layer.

Team-level (admin-managed; surfaced under team settings UI):

- `team.duplicate_alert.signal.slack_webhook` (string, default empty) — Slack incoming-webhook URL.
- `team.duplicate_alert.signal.linear_issue_template` (string, default empty) — Linear issue body template; supports `{pr_number}`, `{candidates}`, `{signals}` placeholders.

### Scope of toggles — user vs team

The user-level toggle and team-level external-routing config govern **independent** layers. They do not chain. The table below is the authoritative contract; any other prose in this spec that appears to contradict it is superseded by this section.

| Setting | Scope | What it controls | What it does NOT control | Default |
|---|---|---|---|---|
| `code.review.duplicate_alert.enabled` | Per-user | Whether **this user** sees the in-product "Possible duplicates" callout in the Code Review pane. | Team-level external signal emission (Slack/Linear). External signals are governed solely by team-level destination config and the dedupe rules in B-Dedup. | `true` |
| `team.duplicate_alert.signal.slack_webhook` | Per-team (admin) | Slack destination for the team-level external alert. | Per-user in-product callout rendering. | empty (off) |
| `team.duplicate_alert.signal.linear_issue_template` | Per-team (admin) | Linear issue template for the team-level external alert. | Per-user in-product callout rendering. | empty (off) |

**Independence rules.**

- A user with `enabled = false` sees no callout in their UI. This affects that user only and has no effect on signal emission.
- The team-level external signal (Slack/Linear) fires **once per dedupe-tuple** (see B-Dedup) regardless of any individual user's toggle. Maintainers receive the routed signal even if individual reviewers have callouts disabled.
- Disabling the user-level toggle does **not** suppress team-level routing. Disabling team-level routing (admin clears the destinations) does not suppress per-user callouts.

### B-Dedup. Persistent dedupe / cooldown for external signals

External signals (Slack/Linear) are deduped by the **unordered pair** `({pr_a, pr_b}, signal_type)` where `signal_type ∈ { same_issue_ref, file_overlap, small_pr_match, rapid_fire_same_author }`. The pair is canonicalized by sorting the two PR numbers ascending before keying the dedupe store, so opening either PR in the pair produces the same key. This matches the spec's promise that the cooldown applies when **either** PR in the pair is opened — there is no separate `(A→B)` vs `(B→A)` cooldown.

- Dedupe state is persisted in a **server-side store** (e.g., the team-settings DB or a per-team Redis namespace), not in process memory and not in client-side storage. This survives client app restarts, different reviewers opening the same PR, and Warp version upgrades.
- TTL: **7 days**. Within the TTL window, the same tuple does **not** re-fire — even across review sessions, app restarts, or different reviewers opening either PR in the pair.
- When the TTL expires, the next detection event re-fires the external signal **once**, and the TTL clock resets. This handles long-lived PRs where the duplicate situation persists and needs another nudge.
- The in-product callout is **not** subject to dedupe — it always renders the current candidate set on every pane open. Dedupe only governs the team-level external signal.
- Dismissal (B4) is a stronger suppression than dedupe: a dismissed `(source_pr, candidate_pr)` pair never re-fires the external signal regardless of TTL, until both sides delete their dismissal markers.

## Acceptance Criteria

- A1. A PR whose body references an issue already referenced by another open PR triggers the callout with signal label "References same issue #N".
- A2. Two PRs touching ≥3 files with ≥50% Jaccard path-set overlap each show the other in their callout with signal label "X% file overlap".
- A3. A 1–2 file PR matches another only when path sets are identical AND title cosine ≥ 0.70.
- A4. A dismissed candidate stays hidden for that (PR, candidate) pair on subsequent opens, in both directions.
- A5. The `MaintainerAlert.DuplicatePr` external signal is deduped by the canonical unordered pair `({pr_a, pr_b}, signal_type)` with a 7-day TTL on a **server-side persistent** store; it does not re-fire within TTL across sessions, app restarts, or different reviewers, and opening either PR in the pair hits the same dedupe key.
- A5a. After the 7-day TTL elapses, the next detection of the same canonical pair re-fires the external signal exactly once and resets the TTL.
- A6. With `code.review.duplicate_alert.enabled = false`, the **in-product callout** does not render for that user. Team-level external signals continue to fire (deduped per A5) regardless of any individual user's toggle.
- A7. Clicking a candidate row opens that candidate PR in a new Code Review pane without replacing the current one.
- A8. A `<!-- warp-dup-dismiss pr=N -->` marker on a comment whose author returns `permission ∈ { admin, maintain, write }` from `GET /repos/{owner}/{repo}/collaborators/{username}/permission` is honored; the candidate is suppressed.
- A9. The same marker text on a comment whose author returns `permission ∈ { triage, read, none }` (or whose `author_association` is in the negative-filter set `{CONTRIBUTOR, FIRST_TIME_CONTRIBUTOR, FIRST_TIMER, NONE, MANNEQUIN}`) is **ignored** and the candidate continues to surface — even if `author_association` is `MEMBER`.
- A10. The Slack webhook URL is never returned by any API to a non-admin caller, never appears in logs/telemetry/error messages, and is rejected at save time if the URL scheme is not HTTPS.

## Implementation Pointers

Verified paths:

- Code Review pane: `app/src/code_review/code_review_view.rs`, `app/src/code_review/code_review_view_integration.rs`.
- Code Review header (likely callout host): `app/src/code_review/code_review_header/header_revamp.rs`, `app/src/code_review/code_review_header/mod.rs`.
- Settings (user-level): `app/src/settings/` (sibling files like `app/src/settings/accessibility.rs` show the pattern; new `app/src/settings/code_review.rs` recommended if not yet present).
- Team server API (signal destinations): `app/src/server/server_api/team.rs`.
- Telemetry hosts: existing `telemetry.rs` files under `app/src/ai/agent/telemetry.rs` etc. show the per-domain pattern; recommend `app/src/code_review/telemetry.rs` for the new event.

New modules:

- `app/src/code_review/duplicate_detector.rs` (new) — pure detection logic over an open-PR list snapshot.
- `app/src/code_review/duplicate_callout.rs` (new) — UI for the callout above the diff.
- `app/src/code_review/duplicate_signal.rs` (new) — Slack/Linear emitter.

## Tests

- T1. Same `Closes #N` body in two open PRs produces a candidate match.
- T2. File-set Jaccard ≥ 0.50 over ≥3 files produces a match; below 0.50 does not.
- T3. Small-PR (1–2 files) path requires identical path set AND title cosine ≥ 0.70.
- T4. Dismissal persists via PR-comment marker and is bidirectional.
- T5. External signal does not re-fire for the same canonical unordered pair `({pr_a, pr_b}, signal_type)` within the 7-day TTL — and opening PR B after PR A within the window also does **not** re-fire (verifies bidirectional keying).
- T_dedup_ttl. After exactly 7 days have elapsed since the previous emission, the next detection of the same canonical pair re-fires once and resets the TTL.
- T_dedup_across_sessions. Dedupe state survives client restart, multiple reviewers opening either PR, and is read from the server-side store (not from in-process memory).
- T6. User toggle OFF mutes the per-user in-product callout but does **not** suppress team-level external signals.
- T7. Background re-detection picks up newly opened PRs without requiring pane refresh.
- T8. Click navigation opens the candidate in a new pane.
- T_dismissal_trust_write_respected. A `warp-dup-dismiss` marker on a comment whose author returns `permission ∈ { admin, maintain, write }` from the collaborators-permission API suppresses the candidate.
- T_dismissal_trust_member_without_write_ignored. A marker on a comment whose `author_association = MEMBER` but whose collaborators-permission lookup returns `read` (e.g., an org member who is not a repo collaborator) is **ignored** and the candidate continues to surface — verifies that `author_association` alone is not trusted.
- T_dismissal_trust_external_ignored. The same marker text on a comment from a CONTRIBUTOR/FIRST_TIME_CONTRIBUTOR/NONE author is rejected by the fast-path `author_association` filter without an API call; the candidate still surfaces.
- T_webhook_secret_redacted. Logs, telemetry payloads, error messages, and panic traces produced during a Slack emission do not contain the webhook URL — only the literal `[redacted-slack-webhook]`.
- T_webhook_admin_only. A non-admin team member's read of the webhook value returns the redacted display only; the API never returns the cleartext URL to non-admins.
- T_webhook_https_only. Saving a webhook with `http://` scheme is rejected at validation time.

## Open Questions

- Should detection extend to other repositories within the same team when a team has multiple? Suggest V1 single-repo only; V1.5 may extend with a team-level allowlist of repos to cross-check.
- Should the diff-overlap signal compute on raw line-add/remove sets or only on file paths? V1 uses paths only to keep the detector cheap; line-overlap is a V1.5 extension.

## Telemetry

- New event: `code_review.duplicate_detected { pr_number, candidate_count, signal_types }` — fires once per detection run that produced ≥1 candidate.
- New event: `code_review.duplicate_dismissed { pr_number, candidate_pr_number }` — fires on the "Mark as not duplicate" action.
- New event: `code_review.duplicate_signal_emitted { destination }` — fires per Slack or Linear emission.
- Reuse: existing `code_review.opened` event remains the per-pane open event.
