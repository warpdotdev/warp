# Official Master Sync Assessment (2026-05-07)

## Scope

Assess whether the current working branch can safely pull the latest official `origin/master`
without immediately merging on top of a dirty worktree.

This document is intentionally read-only guidance. It does not imply that a merge/rebase should
be performed on the current worktree yet.

## Branch State

- Working branch: `codex/i18n-sync-prep-20260502-043641`
- Local `HEAD`: `52754e6`
- Official latest fetched from `origin/master`: `0b3311fa`
- Relative state at assessment time: `ahead 5, behind 87`
- Current worktree: dirty, with active uncommitted i18n coverage changes

## Why We Did Not Pull Directly

The current worktree is not a good place to run `git pull` or `git merge origin/master` directly:

- there are local uncommitted changes in UI/i18n files
- the branch is already `behind 87`
- several of the files under active local modification are also touched by newer upstream behavior

That combination makes direct integration noisy and easy to get wrong.

## Safe Alternative Used

Instead of touching the dirty branch, a temporary worktree was created at:

- `/private/tmp/warp-origin-master-20260507`

That worktree points at official latest:

- `origin/master @ 0b3311fa`

This let us verify the latest official code can be built and launched without disturbing the local
i18n branch.

## Build / Launch Notes For Official Latest

Official latest did not build immediately because the machine was missing `protoc`.

Actions taken in the temporary worktree:

- installed `protobuf` via Homebrew
- reran `./script/run --dont-open` under:
  - `TERM=xterm-256color`
  - `COLORTERM=truecolor`

Result:

- latest official bundle successfully produced and signed:
  - `/private/tmp/warp-origin-master-20260507/target/debug/bundle/osx/WarpOss.app`

## Local Dirty Files

Current uncommitted local files:

- [app/src/code_review/code_review_view.rs](/Users/later0day/Desktop/warp/app/src/code_review/code_review_view.rs)
- [app/src/i18n.rs](/Users/later0day/Desktop/warp/app/src/i18n.rs)
- [app/src/resource_center/keybindings_page.rs](/Users/later0day/Desktop/warp/app/src/resource_center/keybindings_page.rs)
- [app/src/resource_center/view.rs](/Users/later0day/Desktop/warp/app/src/resource_center/view.rs)
- [app/src/settings_view/keybindings.rs](/Users/later0day/Desktop/warp/app/src/settings_view/keybindings.rs)
- [app/src/settings_view/mod.rs](/Users/later0day/Desktop/warp/app/src/settings_view/mod.rs)
- [app/src/workspace/view.rs](/Users/later0day/Desktop/warp/app/src/workspace/view.rs)
- [app/src/workspace/view/left_panel.rs](/Users/later0day/Desktop/warp/app/src/workspace/view/left_panel.rs)
- [app/src/workspace/view/right_panel.rs](/Users/later0day/Desktop/warp/app/src/workspace/view/right_panel.rs)
- [app/src/workspace/view/vertical_tabs.rs](/Users/later0day/Desktop/warp/app/src/workspace/view/vertical_tabs.rs)

## True Upstream Overlap

Using merge-base `3ce4239da86aa0f06587ceba15b50882b9671843`, only three of the currently dirty local
files were also changed by official upstream after that point:

- [app/src/workspace/view.rs](/Users/later0day/Desktop/warp/app/src/workspace/view.rs)
- [app/src/code_review/code_review_view.rs](/Users/later0day/Desktop/warp/app/src/code_review/code_review_view.rs)
- [app/src/settings_view/mod.rs](/Users/later0day/Desktop/warp/app/src/settings_view/mod.rs)

This is the important correction. The other local dirty files differ from current official master,
but not because official latest changed them after the branch diverged.

## Local-Only Dirty Files

These currently appear to be local-only and can be replayed or preserved with much lower risk:

- [app/src/i18n.rs](/Users/later0day/Desktop/warp/app/src/i18n.rs)
- [app/src/resource_center/keybindings_page.rs](/Users/later0day/Desktop/warp/app/src/resource_center/keybindings_page.rs)
- [app/src/resource_center/view.rs](/Users/later0day/Desktop/warp/app/src/resource_center/view.rs)
- [app/src/settings_view/keybindings.rs](/Users/later0day/Desktop/warp/app/src/settings_view/keybindings.rs)
- [app/src/workspace/view/left_panel.rs](/Users/later0day/Desktop/warp/app/src/workspace/view/left_panel.rs)
- [app/src/workspace/view/right_panel.rs](/Users/later0day/Desktop/warp/app/src/workspace/view/right_panel.rs)
- [app/src/workspace/view/vertical_tabs.rs](/Users/later0day/Desktop/warp/app/src/workspace/view/vertical_tabs.rs)

## Overlap Detail

### app/src/workspace/view.rs

Upstream change themes:

- local-to-cloud handoff UI
- orchestration pill updates
- new-session `+` dropdown alignment
- diff button behavior when code review button is hidden
- conversation model update distinctions
- markdown-viewer handling for `.md` links in AI rules/facts
- rename-active-pane keyboard binding support

Assessment:

- Highest risk of the three overlapping files
- This file is hot and user-facing
- Merge should prefer upstream behavior first, then replay local i18n text substitutions

### app/src/code_review/code_review_view.rs

Upstream change themes:

- `APP-4263` git operations flicker fix

Assessment:

- Medium risk
- Behavior changes are narrower than `workspace/view.rs`
- Local changes are mostly empty-state and localized labels

### app/src/settings_view/mod.rs

Upstream change themes:

- terminal Page Up / Page Down prompt scrolling fix touched flags exported through settings view

Assessment:

- Low to medium risk
- Small upstream delta
- Local changes are title / header localization related

## Recommended Merge Order

1. Preserve the current dirty state first.
   - commit it in a WIP commit, or branch/stash it in a recoverable way

2. Sync to official latest on a clean branch.
   - either reset a clean branch to `origin/master`
   - or create a new integration branch from `origin/master`

3. Reapply local-only files first.
   - `i18n.rs`
   - resource center keybindings files
   - left/right/vertical-tabs local tooltip and panel strings

4. Merge overlap files one at a time.
   - `workspace/view.rs`
   - `code_review/code_review_view.rs`
   - `settings_view/mod.rs`

5. Rebuild and launch.
   - `cargo check -p warp`
   - `git diff --check`
   - `./script/run --dont-open`

## Practical Recommendation

Do not pull official latest directly into the current dirty worktree.

The safer path is:

- keep the local i18n worktree as-is
- use official latest as the integration base
- replay local-only i18n files first
- then resolve the three true overlap files manually

