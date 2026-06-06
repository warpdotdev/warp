# Stale PR Follow-up for Requested-Changes Reviews — Product Spec
Linear: none provided (originates from warpdotdev/oz-for-oss#457)
Figma: none provided. GitHub automation with no UI beyond PR comments and the closed state.

## Summary
External-contributor PRs that receive a requested-changes review — from Oz or from a human — can sit indefinitely with no author follow-up, creating review backlog and giving contributors no clear expectation of when an inactive PR will be closed. This adds an inactivity lifecycle: after a period of no meaningful activity, automation posts reminder comments tagging the author, and if inactivity continues, closes the PR with an explanatory comment. Any new meaningful activity resets the timer.

## Problem
Oz turns a non-member `REJECT` verdict into a real `REQUEST_CHANGES` review, and humans can request changes directly. Nothing follows up afterward. The Ossie stale-PR report covers the complementary case (a PR waiting on an *internal reviewer*); this lifecycle owns the other half — a PR waiting on an *external author* after changes were requested.

## Scope
- v1 applies only to **external-contributor PRs** — fork PRs, already labeled `external-contributor` on open by `.github/workflows/label_external_contributors.yml`.
- Both **Oz-authored and human-authored** requested-changes reviews are in scope.
- Member/collaborator-authored PRs are out of scope: Oz only leaves comment-only feedback on them, and reviewer-side staleness is already surfaced elsewhere.

## Behavior
1. **Eligibility.** A PR is eligible only when all hold: it is open, not a draft, labeled `external-contributor`, its latest *decisive* review state is `CHANGES_REQUESTED` (Oz- or human-authored, not later dismissed or superseded by an approval), and it does not carry the exemption label.
2. **Activity definition (author-driven).** Inactivity is measured from the PR's last *author-driven* event: the most recent of (a) a head-branch push, measured by GitHub's server-recorded push time (so a backdated or force-pushed commit still counts), or (b) a comment by the **PR author** (including replies on review threads). Comments by maintainers/other third parties, automation-authored comments (including this lifecycle's own reminders), label changes, base-branch updates, and bare `updated_at` bumps do **not** reset the timer — the lifecycle exists to chase *author* follow-up, so a maintainer nudge must not extend the closure clock.
3. **Reminder at 7 days inactivity.** Automation posts one reminder comment tagging the author, explaining that changes were requested and that the PR may be closed after continued inactivity, and stating the closure deadline.
4. **Reminder at 14 days inactivity.** A second reminder is posted with the same content and an updated remaining-time statement.
5. **Final warning at 26 days inactivity.** A final reminder makes clear the PR will be closed in ~4 days if no activity occurs.
6. **Close at 30 days inactivity.** Automation re-verifies the PR is still eligible (still `CHANGES_REQUESTED`, not exempt, not draft) **and that the day-26 final-warning reminder was already posted in the current window**, then posts an explanatory closing comment (pointing the author at how to reopen/continue) and closes the PR. A PR is never closed without a prior final warning — this also makes the first enablement safe for the existing backlog of already-stale PRs, which receive a warning first and close only on a later run.
7. **Activity resets the timer.** Any new meaningful event (per invariant 2) resets inactivity to zero and re-arms all reminder stages; an active PR never advances toward closure.
8. **A later approval or dismissal exits the lifecycle.** If the requested-changes state is resolved (Oz/human approves, or the review is dismissed) the PR is no longer eligible and receives no further reminders or closure.
9. **Comment authorship.** Reminders and the closing comment are posted by the workflow's GitHub Actions identity (`github-actions[bot]`, via the default `GITHUB_TOKEN`). Authoring them as the `oz-for-oss` bot — to match other Oz comments on the PR — is a non-functional nicety, explicitly deferred (see the Tech Spec follow-ups), and must not block this work.
10. **No duplicate or repeated actions.** At most one comment is posted per reminder stage per inactivity window; re-running the scan does not post duplicate reminders or re-close an already-closed PR. After a timer reset, stages become eligible again for the new window.
11. **Exemption.** A maintainer-applied `no-autoclose` label removes the PR from the lifecycle immediately; no reminders or closure occur while it is present.
12. **Drafts are skipped.** Converting a PR to draft removes it from the lifecycle until it is marked ready for review again.

## Non-goals
- Acting on member/collaborator-authored PRs.
- Reacting to human `REQUEST_CHANGES` in real time (this is a periodic inactivity sweep, not a webhook trigger).
- Reopening or re-reviewing PRs after closure (the closing comment explains how the author can ask to reopen).
- Creating the `external-contributor` or exemption labels automatically.

## Decisions
- Cadence: reminders at 7 and 14 days, final warning at 26, close at 30 — a 4-day buffer after the final warning.
- Exemption label: `no-autoclose`.
- Initial rollout: full reminder-plus-close. The "no close without a prior final warning" rule (invariant 6) makes this safe for the existing backlog of already-stale PRs.

## Open questions
- A PR where the author has already responded and is now waiting on a maintainer still advances toward closure, since only author activity resets the timer (invariant 2). Maintainers can hold such PRs with `no-autoclose`. Acceptable for v1, or should "author has responded since the latest requested-changes review" pause the lifecycle? Pausing is fairer but lets a token commit dodge closure indefinitely.
