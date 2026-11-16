# APP-4118: Toggle AI auto-gen + Enterprise gate — Tech Spec
Product spec: `specs/APP-4118/PRODUCT.md`
Parent: APP-3918 (git operations button)
## Context
APP-3918 landed the git-operations split button and wired three dialogs (Commit, Push, Create PR) plus two chains (`CommitAndPush`, `CommitAndCreatePr`). Every AI-backed path in those flows is gated only on `FeatureFlag::GitOperationsInCodeReview`; none of them respect `AISettings::is_any_ai_enabled` or enterprise status. This spec mirrors the `share_block_modal.rs::should_send_title_gen_request` pattern: a dedicated per-feature AI setting, plus a shared `should_send_git_ops_ai_request` helper that folds the feature flag, the per-feature AI toggle, and an enterprise check (with Warp-plan exception and dogfood override) together and routes every AI call site in the git-operations flow through it.
### Relevant code
- `app/src/code_review/git_dialog/commit.rs:116-120` — chooses `GENERATING_PLACEHOLDER_TEXT` vs `FALLBACK_PLACEHOLDER_TEXT` based on the feature flag only.
- `app/src/code_review/git_dialog/commit.rs:209-222` — kicks off `generate_commit_message` on dialog open when there are changes.
- `app/src/code_review/git_dialog/commit.rs:260-320` — `generate_commit_message` helper; unconditionally calls `code_review_ai.generate_code_review_content`.
- `app/src/code_review/git_dialog/commit.rs:385-396` — `CommitAndCreatePr` chain calls `create_pr_with_ai_content`.
- `app/src/code_review/git_dialog/pr.rs:101-126` — standalone Create PR confirm calls `create_pr_with_ai_content`.
- `app/src/code_review/git_dialog/pr.rs:134-184` — `create_pr_with_ai_content` issues two `generate_code_review_content` requests (title + body) and falls back to `gh pr create --fill` on failure / empty content.
- `app/src/settings/ai.rs:1468-1477` — `AISettings::is_any_ai_enabled` (already folds in auth + remote-session org policy).
- `app/src/settings/ai.rs:815-823` — `shared_block_title_generation_enabled_internal` setting, shape we are copying for the new `git_operations_autogen_enabled_internal`.
- `app/src/settings/ai.rs:1536-1538` — `is_shared_block_title_generation_enabled` getter, shape we are copying for the new `is_git_operations_autogen_enabled`.
- `app/src/terminal/share_block_modal.rs:1161-1174` — `should_send_title_gen_request`, the AI-title-gen gate we are mirroring (`is_active_ai_enabled` via per-feature getter, enterprise check with Warp-plan exception, dogfood override).
- `app/src/workspaces/workspace.rs:562` — `BillingMetadata::is_warp_plan()` accessor used by the Warp-plan exception.
- `app/src/workspaces/user_workspaces.rs` — `UserWorkspaces`, `current_team` accessor.
- `crates/graphql/src/api/workspace.rs` — `CustomerType` enum definition (reached via `billing_metadata`).
## Current state
The commit dialog opens, sets the `"Generating…"` placeholder, fires a diff-build plus a `generate_code_review_content` call, and swaps the placeholder + writes the generated text when the future resolves. Failure and empty-content cases already exist and are well-tested: both fall back to `FALLBACK_PLACEHOLDER_TEXT` with an empty editor. The PR flow already handles AI failure via `gh pr create --fill`. In other words: the "skip AI entirely" code paths already exist — we just need to take them deterministically when the gate is closed.
## Proposed changes
### 1. New per-feature AI setting
Add `git_operations_autogen_enabled_internal` to the `define_settings_group!(AISettings, ...)` block in `app/src/settings/ai.rs`, matching the shape of `shared_block_title_generation_enabled_internal` (`ai.rs:815-823`):
```rust path=null start=null
git_operations_autogen_enabled_internal: GitOperationsAutogenEnabled {
    type: bool,
    default: true,
    supported_platforms: SupportedPlatforms::ALL,
    sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
    private: false,
    toml_path: "agents.oz.active_ai.git_operations_autogen_enabled",
    description: "Controls whether AI auto-generates commit messages and PR title/body in the code review dialogs.",
}
```
And the getter on `AISettings`, matching `is_shared_block_title_generation_enabled` (`ai.rs:1547-1549`):
```rust path=null start=null
pub fn is_git_operations_autogen_enabled(&self, app: &AppContext) -> bool {
    self.is_active_ai_enabled(app) && *self.git_operations_autogen_enabled_internal
}
```
`is_active_ai_enabled` composes `is_any_ai_enabled` (global AI toggle + auth + remote-session org policy) with the active-AI internal toggle and `AppExecutionMode::allows_active_ai()`, so the new getter transitively inherits all of those checks.
Expose it in the AI settings page (`app/src/settings_view/ai_page.rs`) alongside the existing per-feature toggles.
### 2. New helper: `should_send_git_ops_ai_request`
Location: `app/src/code_review/git_dialog/mod.rs` (new private helper, co-located with the other shared git-dialog utilities like `show_toast` and `user_facing_git_error`). Structure copied from `share_block_modal.rs::should_send_title_gen_request` (`share_block_modal.rs:1161-1174`):
```rust path=null start=null
fn should_send_git_ops_ai_request(app: &AppContext) -> bool {
    FeatureFlag::GitOperationsInCodeReview.is_enabled()
        && AISettings::as_ref(app).is_git_operations_autogen_enabled(app)
        && (!UserWorkspaces::as_ref(app)
            .current_team()
            .is_some_and(|team| team.billing_metadata.customer_type == CustomerType::Enterprise)
            // Allow the Warp Stable team to use this.
            || UserWorkspaces::as_ref(app)
                .current_team()
                .is_some_and(|team| team.billing_metadata.is_warp_plan())
            // Override the enterprise check for dogfood builds, as our dogfood team
            // is an enterprise team.
            || ChannelState::channel().is_dogfood())
}
```
Shared helper (vs inlining like `share_block_modal.rs`) because we call it from both `commit::new_state` and `pr::new_state`, and keeping the enterprise / Warp-plan / dogfood details out of `commit.rs` and `pr.rs` avoids pulling billing + channel imports into those files.
### 2a. Gate evaluation
`should_send_git_ops_ai_request` is called directly at each decision point rather than snapshotted onto dialog state. In `commit::new_state` it is called once to set the initial placeholder and decide whether to kick off open-time autogen. In `commit::start_confirm` and `pr::start_confirm` it is called again at confirm time to decide whether to materialize a `code_review_ai` handle. This is consistent with how the sibling `should_send_title_gen_request` is used.
### 3. Gate `commit.rs` open-time autogen
In `commit::new_state`, read `ai_autogen_enabled = should_send_git_ops_ai_request(ctx)` once and use it for both:
- `initial_placeholder` becomes `GENERATING_PLACEHOLDER_TEXT` only when `ai_autogen_enabled`; otherwise `FALLBACK_PLACEHOLDER_TEXT`.
- The `generate_commit_message` call inside the spawn's resolution is only issued when `ai_autogen_enabled && has_changes`. When the gate is closed (or there are no changes), the spawn resolution skips autogen and the editor stays on `FALLBACK_PLACEHOLDER_TEXT`.
No changes to `generate_commit_message` itself — it is unreachable when the gate is closed.
### 4. Gate `pr.rs::start_confirm`
`start_confirm` calls `should_send_git_ops_ai_request(ctx)` directly and branches:
- AI path: `create_pr_with_ai_content` with the `code_review_ai` handle.
- Fallback path: `create_pr(repo_path, None, None)` (`gh pr create --fill`).
The `code_review_ai` handle is only materialized on the AI branch so we do not hold an unused handle in the AI-off path. Existing success/error handling is unchanged — both branches return `Result<PrInfo>`.
### 5. Gate the `CommitAndCreatePr` chain
`commit::start_confirm` calls `should_send_git_ops_ai_request(ctx)` and uses the result to decide whether to materialize `code_review_ai`. The `Option<Arc<dyn AIClient>>` is captured into the `ctx.spawn` async body; the `CommitAndCreatePr` arm matches on it:
```rust path=null start=null
CommitIntent::CommitAndCreatePr => {
    run_push(&repo_path, &branch_name).await?;
    let pr = match code_review_ai {
        Some(ai) => create_pr_with_ai_content(&repo_path, &branch_name, ai.as_ref()).await?,
        None => create_pr(&repo_path, None, None).await?,
    };
    CommitOutcome::PrCreated(pr)
}
```
### 6. Imports
The helper in `git_dialog/mod.rs` needs:
- `warp_core::features::FeatureFlag`
- `crate::settings::AISettings`
- `crate::workspaces::user_workspaces::UserWorkspaces`
- `CustomerType` (match the path used by `share_block_modal.rs:32`)
- `warp_core::channel::ChannelState` (match the path used by `share_block_modal.rs`)
`commit.rs` and `pr.rs` each import `should_send_git_ops_ai_request` from `super`. `commit::new_state` calls it to set the initial placeholder; `commit::start_confirm` and `pr::start_confirm` each call it again at confirm time.
## Testing and validation
No unit tests are added for this change. Validation is manual; maps to Behavior invariants 2, 4–9, 12–14:
- With AI enabled + non-enterprise: commit / push / PR flows unchanged from APP-3918 (no regressions). Covered by re-running APP-3918's manual validation list.
- Toggle `is_any_ai_enabled` off → open Commit dialog → editor shows `"Type a commit message"` immediately, no `"Generating…"`, no AI request in the network log. (Invariants 4–6, 10.)
- Still off → confirm a typed message → commit succeeds. `Commit and push` chain with a typed message → commit + push succeed. (Invariants 7, 11.)
- Still off → `Commit and create PR` with a typed message → PR gets created with the latest commit's subject/body (via `--fill`). (Invariant 8.)
- Still off → open Create PR dialog → confirm → PR gets created via `--fill`. (Invariant 9.)
- Enterprise user (AI on): same behaviors as AI-off. (Invariant 1, second clause.)
- Toggle AI off *while* the dialog is open mid-`"Generating…"`: the dialog keeps its in-flight state; close and reopen → now behaves as AI-off. (Invariants 12, 14.)
### Verification aid
The existing log line `"Failed to autogenerate commit message"` at `commit.rs:310` is a useful canary: it must not fire in AI-off/enterprise runs of the manual validation (because no request is attempted). Invariant 10 depends on this.
## Risks and mitigations
- **Drift between commit-message autogen and PR-creation gate decisions**: each call to `should_send_git_ops_ai_request` evaluates the same predicate, so open-time autogen (evaluated in `new_state`) and confirm-time PR creation (evaluated in `start_confirm`) both reflect the current AI state at the time they run. Invariant 14a (gate evaluated at dialog-open) holds for the initial placeholder/autogen decision; confirm re-evaluates but in practice the user cannot change AI state between open and confirm in normal usage.
- **Enterprise detection via `current_team()` returns `None`**: if a signed-in user has no current team, `is_some_and` yields `false` for both the enterprise branch and the Warp-plan branch. The enterprise-guard's outer negation means no-team users pass the gate — same behavior as `should_send_title_gen_request`.
- **Dogfood override on production builds**: `ChannelState::channel().is_dogfood()` is false on Preview/Stable, so the override only fires where it is intended.
- **Per-feature setting drift**: new `git_operations_autogen_enabled_internal` setting needs to be exposed in the AI settings page alongside the other per-feature toggles — easy to forget. Covered under §1.
## Follow-ups
- Consider adding the same enterprise check to `code_review_view.rs:7104`'s `"Add diff set as context"` gate if product wants a uniform enterprise AI posture across code review (Invariant 15 explicitly scopes this out for now).
- If telemetry ever needs to distinguish "AI skipped because disabled" from "AI attempted and failed", add an event in the gate's `false` branch — not required by this spec.
- If other code-review AI call sites (e.g. future review-comment summarization) land, reuse `should_send_git_ops_ai_request` or a sibling helper rather than inlining the enterprise-plus-Warp-plan-plus-dogfood clause again.
