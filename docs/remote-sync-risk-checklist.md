# Remote Sync Risk Checklist

## Goal

Create a practical merge/rebase checklist for bringing the current i18n branch state forward onto the latest `origin/master` without accidentally regressing behavior.

## Current State

- Current working branch: `codex/i18n-sync-prep-20260502-043641`
- Local `HEAD`: `874a257`
- Remote `origin/master`: `aee0570`
- Branch status at review time: `ahead 1, behind 91`
- Working tree: large uncommitted i18n and language-setting changes
- Overlap between upstream-touched files and current working-tree files: `32`
- Repository history has now been unshallowed and `git merge-base HEAD origin/master` resolves successfully.

## Prep Status

- [x] Create a safety branch from the dirty i18n state.
- [x] Deepen repository history with `git fetch --unshallow origin`.
- [x] Re-run overlap and branch-state analysis with a valid merge base available.

## Recommended Order

- [ ] Phase 1: Land or park the pure i18n infrastructure first.
- [ ] Phase 2: Re-apply low-risk string substitutions.
- [ ] Phase 3: Reconcile medium-risk UI files where upstream changed adjacent behavior.
- [ ] Phase 4: Manually merge high-risk hot files that upstream changed functionally.
- [ ] Phase 5: Rebuild, run i18n tests, and do targeted UI verification.

## Low-Risk Files

These mostly add language-setting registration or swap hardcoded UI text for translated text.

- [ ] [app/src/settings/init.rs](/Users/later0day/Desktop/warp/app/src/settings/init.rs)
- [ ] [app/src/settings/mod.rs](/Users/later0day/Desktop/warp/app/src/settings/mod.rs)
- [ ] [app/src/settings/language.rs](/Users/later0day/Desktop/warp/app/src/settings/language.rs)
- [ ] [app/src/i18n.rs](/Users/later0day/Desktop/warp/app/src/i18n.rs)
- [ ] [app/src/lib.rs](/Users/later0day/Desktop/warp/app/src/lib.rs)
- [ ] [app/src/auth/paste_auth_token_modal.rs](/Users/later0day/Desktop/warp/app/src/auth/paste_auth_token_modal.rs)

## Medium-Risk Files

These are mostly UI files, but upstream touched behavior close enough that merges should be reviewed, not replayed blindly.

- [ ] [app/src/code/file_tree/view.rs](/Users/later0day/Desktop/warp/app/src/code/file_tree/view.rs)
- [ ] [app/src/code_review/code_review_view.rs](/Users/later0day/Desktop/warp/app/src/code_review/code_review_view.rs)
- [ ] [app/src/code_review/git_dialog/mod.rs](/Users/later0day/Desktop/warp/app/src/code_review/git_dialog/mod.rs)
- [ ] [app/src/uri/mod.rs](/Users/later0day/Desktop/warp/app/src/uri/mod.rs)
- [ ] [app/src/settings_view/features_page.rs](/Users/later0day/Desktop/warp/app/src/settings_view/features_page.rs)
- [ ] [app/src/settings_view/settings_page.rs](/Users/later0day/Desktop/warp/app/src/settings_view/settings_page.rs)
- [ ] [app/src/settings_view/update_environment_form.rs](/Users/later0day/Desktop/warp/app/src/settings_view/update_environment_form.rs)
- [ ] [app/src/terminal/input/inline_history/view.rs](/Users/later0day/Desktop/warp/app/src/terminal/input/inline_history/view.rs)
- [ ] [app/src/terminal/view/pane_impl.rs](/Users/later0day/Desktop/warp/app/src/terminal/view/pane_impl.rs)
- [ ] [app/src/workspace/view/global_search/view.rs](/Users/later0day/Desktop/warp/app/src/workspace/view/global_search/view.rs)
- [ ] [app/src/workspace/view/vertical_tabs.rs](/Users/later0day/Desktop/warp/app/src/workspace/view/vertical_tabs.rs)

## High-Risk Files

These are hot paths where upstream changed behavior, not just presentation. These need manual, side-by-side review.

- [ ] [app/src/terminal/input.rs](/Users/later0day/Desktop/warp/app/src/terminal/input.rs)
  Upstream themes: cloud mode input v2, slash commands, host and harness selectors, history menu behavior.
- [ ] [app/src/terminal/input/slash_commands/mod.rs](/Users/later0day/Desktop/warp/app/src/terminal/input/slash_commands/mod.rs)
  Upstream themes: new slash commands such as `/host`, `/harness`, `/environment`, plus menu behavior.
- [ ] [app/src/terminal/view.rs](/Users/later0day/Desktop/warp/app/src/terminal/view.rs)
  Upstream themes: agent view orchestration pills, SSH and remote server flows, pending remote session state, platform conditionals.
- [ ] [app/src/workspace/view.rs](/Users/later0day/Desktop/warp/app/src/workspace/view.rs)
  Upstream themes: onboarding suppression in headless mode, pending SSH remote-session handling, panel background logic, active-session updates.
- [ ] [app/src/settings_view/appearance_page.rs](/Users/later0day/Desktop/warp/app/src/settings_view/appearance_page.rs)
  Local themes: new display-language setting, dropdown synchronization, category localization.
- [ ] [app/src/settings_view/ai_page.rs](/Users/later0day/Desktop/warp/app/src/settings_view/ai_page.rs)
  Upstream themes: denylist and settings behavior; local changes are large and UI-heavy.
- [ ] [app/src/ai/execution_profiles/editor/mod.rs](/Users/later0day/Desktop/warp/app/src/ai/execution_profiles/editor/mod.rs)
- [ ] [app/src/ai/execution_profiles/editor/ui_helpers.rs](/Users/later0day/Desktop/warp/app/src/ai/execution_profiles/editor/ui_helpers.rs)

## Merge Strategy

- [ ] Keep the language setting and `i18n.rs` as the base abstraction.
- [ ] Re-apply menu and toast string substitutions after upstream behavior is in place.
- [ ] Prefer replaying local i18n edits onto upstream code rather than trying to preserve current file bodies wholesale.
- [ ] In the high-risk files, treat upstream behavior as authoritative first, then port the localized labels and placeholders back in.
- [ ] Avoid merging generated or reordered code mechanically if the same user-visible result can be re-applied in a smaller patch.

## Validation After Rebase

- [ ] `cargo fmt --all`
- [ ] `cargo check -p warp`
- [ ] `cargo test -p warp i18n::tests::all_i18n_keys_have_non_empty_catalog_entries`
- [ ] `git diff --check`
- [ ] Launch the app and spot-check:
- [ ] Settings > Appearance language selector
- [ ] Top app menus
- [ ] Auth flows
- [ ] Terminal context menus
- [ ] Agent and Drive menus
- [ ] Shared-session and toast flows

## Practical Recommendation

- [ ] Do not merge upstream directly into the dirty worktree.
- [ ] First create a safety branch from the current state.
- [ ] Then either commit the i18n work in logical chunks or stash it in a way that can be replayed file-by-file.
- [ ] Rebase or merge onto a clean branch with full history available.
- [ ] Start conflict resolution with the high-risk files listed above before spending time on the easy string-only files.
