# APP-4118: Toggle AI auto-gen + Enterprise gate for git operations
Linear: [APP-4118](https://linear.app/warpdotdev/issue/APP-4118/toggle-ai-auto-gen-enterprise-gate)
Parent: APP-3918 (git operations button) / APP-3071 (git operations in code review)
## Summary
Git-operation dialogs in the code review panel (Commit, Push, Create PR, and their chained variants) currently call the code-review AI endpoint unconditionally, governed only by `FeatureFlag::GitOperationsInCodeReview`. They ignore the user's global AI toggle and expose an AI-backed draft to enterprise customers who should not have AI reaching their source. This spec gates every AI call in the git-operations flow behind the user's AI toggle and a hard enterprise check, so the feature degrades to a pure-manual experience when AI is not allowed — without hiding the git buttons themselves.
## Behavior
### When AI auto-gen is available
1. "AI auto-gen is available" means ALL of the following are true:
   - `FeatureFlag::GitOperationsInCodeReview.is_enabled()` (the parent feature flag).
   - `AISettings::is_git_operations_autogen_enabled(app)` returns `true`. That getter is itself `is_active_ai_enabled(app) && *self.git_operations_autogen_enabled_internal`, matching the sibling `is_shared_block_title_generation_enabled` getter. `is_active_ai_enabled` in turn composes `is_any_ai_enabled` (global AI toggle + auth state + remote-session org policy) with the active-AI internal toggle and `AppExecutionMode::allows_active_ai()`, plus a dedicated per-feature toggle the user can flip independently.
   - Either the user's current team is not enterprise, **or** the team is on the Warp Plan (internal Warp team), **or** the build is a dogfood channel build. This matches `share_block_modal.rs::should_send_title_gen_request` (lines 1161-1174) exactly, which exists for the same reason: our internal Warp team and our dogfood team are both tagged as enterprise customers and would otherwise self-disable AI on internal builds.
2. When AI auto-gen is available, the Commit, Push, Create PR, Commit-and-push, and Commit-and-create-PR flows behave exactly as they do today (see `specs/APP-3918/PRODUCT.md`). No regressions.
### When AI auto-gen is not available
3. The primary git-operations split button, its chevron menu, and all dialog entry points remain visible and functional exactly as they are when AI is available. Git operations themselves do not depend on AI.
4. The Commit dialog opens with the `"Type a commit message"` placeholder immediately. It never shows the `"Generating…"` placeholder.
5. The Commit dialog does not issue any AI request when it opens. The message editor starts empty and remains empty until the user types something.
6. The Confirm button in the Commit dialog stays disabled until the user types a non-empty message, matching the existing "AI returned empty / failed" path — the user is never blocked on a network call that will not happen.
7. The `CommitAndPush` chain runs `run_commit` → `run_push` with the user-typed message, unchanged. It does not involve AI in any state, so it is unaffected.
8. The `CommitAndCreatePr` chain runs `run_commit` → `run_push` → creates the PR via `gh pr create --fill` (i.e. the existing AI-failure fallback path in `create_pr_with_ai_content`), without issuing any AI request for title or body. The resulting PR uses the latest commit's subject/body as title/body.
9. The standalone Create PR dialog (`GitDialogMode::CreatePr`) creates the PR via `gh pr create --fill` on confirm, without issuing any AI request for title or body.
10. Logs and telemetry emitted by the git-operations flow do not reference AI activity in this state. No `"Failed to autogenerate commit message"` warnings fire, because no request is attempted.
11. No toast, banner, or dialog copy tells the user AI is disabled. The absence of the `"Generating…"` state is the only user-visible signal; the feature simply degrades to manual.
### State transitions
12. If the user flips the global AI toggle while a Commit dialog is open, the in-flight state of that dialog is preserved — any already-generated draft stays, any already-showing `"Generating…"` placeholder stays until the in-flight request resolves or fails. The toggle only affects dialogs opened after the change.
13. If the user's team `customer_type` changes (e.g. workspace switch) while a Commit dialog is open, the same rule applies: the open dialog keeps whatever state it was in; only the next-opened dialog reflects the new gate.
14. Toggling the global AI off mid-session and then reopening the Commit dialog yields the not-available behavior above. Toggling it back on yields the available behavior on the next open.
14a. In-flight AI requests are never proactively cancelled, including when the user's team `customer_type` or global AI toggle changes mid-request. The gate is evaluated once when the dialog opens and the result sticks for that dialog's lifetime, including the confirm-time `CommitAndCreatePr` chain and the standalone Create PR confirm. Only the next-opened dialog reflects any subsequent toggle or team change.
### Interaction with existing gates
15. The overflow menu's `"Add diff set as context"` item continues to use its existing `AISettings::is_any_ai_enabled` gate (`code_review_view.rs:7104`). This spec does not change that gate, and does not add an enterprise check there — adding one is a follow-up if product decides.
16. The `FeatureFlag::GitOperationsInCodeReview` flag continues to gate the entire git-operations UI. When the flag is off, the gates in this spec are moot.
17. Remote-session org policy (`is_ai_disabled_due_to_remote_session_org_policy`) already flows through `is_any_ai_enabled`, which `is_active_ai_enabled` composes with — and the new `is_git_operations_autogen_enabled` getter composes with `is_active_ai_enabled`. Enterprise workspaces that disable AI in remote sessions get the not-available behavior without any separate plumbing.
18. The new per-feature setting `git_operations_autogen_enabled_internal` appears in the AI settings page alongside the other per-feature toggles (shared block title generation, code suggestions, etc.). Default: `true`. A user may flip it off independently of the global AI toggle.
