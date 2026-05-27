# zh-CN Localization Review Evidence

## Scope

This record captures the local rebase and validation state for the zh-CN
localization work on branch `feat/localization-settings-upstream-rebuild`.

## Branch State

- Branch: `feat/localization-settings-upstream-rebuild`
- Validated upstream base: `2566f54a`
- Review branch comparison: two local commits ahead of
  `upstream/master`; `upstream/master` itself resolved to `2566f54a` during the
  latest validation run.
- Rebase state: no `.git/rebase-merge`, `.git/rebase-apply`, or
  `.git/index.lock`
- Stash state: the May 27 rebase autostash remains as a safety backup, and the
  pre-existing May 19 stash remains
- Review state: all localization changes were restored and validated against the
  current `upstream/master` base. The latest pass has a focused dirty
  worktree for the async find upstream follow-up and this evidence document.

## Rebase Result

Fetched `upstream` with prune, then rebased the local branch onto
`upstream/master`.

The first fetch updated `upstream/master` from `fc110333` to `c99b9546`,
leaving the local branch behind by 5 commits. `git rebase --autostash
upstream/master` created autostash `3b733739`, applied it successfully, and
updated `feat/localization-settings-upstream-rebuild` to `c99b9546`.

A later fetch updated `upstream/master` from `c99b9546` to `37f104a1`. A second
`git rebase --autostash upstream/master` created autostash `1ecc3210`, applied
it successfully, and updated the branch to `37f104a1`. No conflicts were
reported.

The latest fetch updated `upstream/master` from `37f104a1` to `43e3f58c`. A
third `git rebase --autostash upstream/master` created autostash `4e04a08d`,
applied it successfully, and updated the branch to `43e3f58c`. No conflicts
were reported.

The next fetch updated `upstream/master` from `43e3f58c` to `edef7f83`. A
fourth `git rebase --autostash upstream/master` created autostash `371d65fd`,
applied it successfully, and updated the branch to `edef7f83`. No conflicts
were reported.

The latest fetch updated `upstream/master` from `edef7f83` to `ade38b08`. A
fifth `git rebase --autostash upstream/master` created autostash `b0d810a4`,
applied it successfully, and updated the branch to `ade38b08`. No conflicts
were reported.

The latest fetch updated `upstream/master` from `ade38b08` to `175faadc`. A
sixth `git rebase --autostash upstream/master` created autostash `b4bd0ca4`,
applied it successfully, and updated the branch to `175faadc`. No conflicts
were reported. There are no unmerged paths and no active rebase state.

The latest fetch updated `upstream/master` from `175faadc` to `8f8ff4a8`. A
seventh `git rebase --autostash upstream/master` created autostash `9e34dc21`,
updated the branch to `8f8ff4a8`, then reported one autostash replay conflict
in `app/src/settings_view/billing_and_usage/billing_cycle_usage_section.rs`.
That conflict was resolved by keeping the upstream split billing legend path
while restoring catalog-backed render calls. There are no unmerged paths and no
active rebase state.

The dirty local worktree was protected during rebase and restored afterward.
No user changes were intentionally reverted.


After the `2566f54a` upstream refresh, the upstream async find feature toggle
introduced two new direct English strings and one call-site argument mismatch.
They are now backed by `settings.features.async_find.label` and
`settings.features.async_find.description` in both locale catalogs, and
`AsyncFindWidget` now passes `app` as the first `render_body_item_label`
argument.

## Post-Rebase Fixes

After the upstream refresh, three new direct English menu literals were wired
through localization keys:

- `workspace.menu.new_tab_group` for `New tab group`
- `tab.menu.new_group_with_tab` for `New group with tab`
- `tab.menu.move_to_group` for `Move to group`

Matching keys were added to both `en-US.json` and `zh-CN.json`.

After the `8f8ff4a8` refresh, the upstream billing combined legend tooltip
introduced one new direct English string. It is now backed by
`settings.billing.credits.legend.combined_tooltip` in both locale catalogs and
covered by the localization catalog test list.

The visual review then found two Agent conversation list regressions and one
terminology inconsistency:

- Section headers `ACTIVE` / `PAST` were still hard-coded in the conversation
  list. They now use `workspace.conversation_list.section.active` and
  `workspace.conversation_list.section.past`, rendering as `当前` / `历史` in
  zh-CN.
- Conversation relative timestamps used the shared English-only time formatter,
  rendering `just now`. Conversation list rows now use localized relative time
  keys such as `workspace.conversation_list.time.just_now`, rendering `刚刚` in
  zh-CN.
- Settings schema descriptions still used `智能体` while the catalog standard is
  `Agent`. These 27 zh-CN values were normalized to `Agent`.

Menu custom item titles are now covered by
`app_menu_custom_items_do_not_use_direct_english_literals` in
`crates/localization/tests/localization_tests.rs`. The check scans
`CustomMenuItem::new` and `CustomMenuItem::new_with_submenu` title arguments in
`app/src/app_menus.rs` and fails on direct English UI text.

The zh-CN visual smoke now also includes a runtime assertion for app menu and
Dock menu titles. It builds menus from the real integration app context and
checks the Dock title/item plus root menu titles such as `文件`, `编辑`, `视图`,
`标签页`, `块`, `AI`, `Drive`, `窗口`, and `帮助`.

## Terminology Review

The zh-CN catalog keeps product and technical names in English when they are
user-facing brand or protocol names:

- `Agent`
- `Warp Drive`
- `Notebook`
- `Cloud Oz`
- `MCP`
- `API`
- `CLI`
- `ID`
- `JSON`
- `Shell`
- `Slug`
- `Drive`

The review fixed ordinary UI copy that was still English:

- `Copyright 2026 Warp` -> `版权所有 2026 Warp`
- `Workflows` in descriptive prose -> `工作流`
- `"Use Agent" footer` -> `“使用 Agent”底栏`
- Appearance app icon style names, including `Aurora`, `Comets`, `Cow`,
  `Glass Sky`, `Glitch`, `Glow`, `Holographic`, `Mono`, `Neon`,
  `Starburst`, and `Sticker`
- Settings schema descriptions using `智能体` -> `Agent`
- Conversation list section headers `ACTIVE` / `PAST` -> `当前` / `历史`
- Conversation list timestamp `just now` -> `刚刚`

The latest ASCII-only review checked the 71 ASCII-only zh-CN values. Remaining
ASCII-only values are intentional brand/product names, commands, table fields,
placeholders, or formatting fragments such as `Agent`, `Notebook`, `Warp Drive`,
`Cloud Oz`, `GitHub Action`, `nvm install node`, `Slug`, `ID`, `UUID`, `JSON`,
`Shell`, `\n`, and `{credits} / {price}`.

## Locale Catalog Checks

Current catalog stats after the latest visual-review fixes:

- `en-US` keys: 5576
- `zh-CN` keys: 5576
- Missing in `zh-CN`: 0
- Extra in `zh-CN`: 0
- Placeholder mismatches: 0
- Identical values: 60
- ASCII-only zh-CN values: 71
- ASCII-with-CJK zh-CN values: 1745

Term scan counts:

- `prompt`: 6
- `pane`: 0
- `handoff`: 0
- `snapshot`: 0
- `payload`: 0
- `pull request`: 0
- `workflow`: 0
- `workspace`: 1
- `Workflows`: 0
- `Notebooks`: 0
- `Use Agent`: 0
- `Copyright`: 0
- `智能体`: 0

## Verification

Commands run after the `2566f54a` upstream refresh:

```bash
git diff --check && git diff --cached --check && \
jq empty app/assets/bundled/locales/en-US.json app/assets/bundled/locales/zh-CN.json
```

Result: pass.

```bash
cargo fmt --all -- --check
```

Result: pass.

```bash
cargo test -p warp_localization -- --nocapture
```

Result: pass, 20 tests on current `b563723c` plus the focused async find
worktree changes. Latest run compiled in 9.21s and the catalog test binary
finished in 20.00s.

```bash
cargo test -p warp --lib localization_tests -- --nocapture
```

Result: earlier compile/runner evidence only. It completed on `175faadc`, ran
0 tests with 4655 filtered, and confirmed the `warp` test binary compiled.
Actual app localization tests are registered under `localization::tests`.

```bash
cargo test -p warp --lib localization::tests -- --nocapture
```

Result: pass, 8 tests on current `b563723c` plus the focused async find
worktree changes, with 4656 filtered. Latest run compiled in 54m 05s and the
filtered test run finished in 1.58s.

```bash
cargo check -p warp --lib --message-format=short
```

Result: pass on current `b563723c` plus the focused async find worktree
changes, 31m 29s.

```bash
cargo build -p integration --bin integration
```

Result: pass on current `b563723c` plus the focused async find worktree
changes, 62m 00s.

The current-head visual smoke was then run directly through the built binary to
avoid retriggering Cargo build/link work:

```bash
WARPUI_USE_REAL_DISPLAY_IN_INTEGRATION_TESTS=1 \
WARP_INTEGRATION_TEST_ARTIFACTS_DIR="$PWD/target/zh-cn-visual-artifacts" \
WARP_INTEGRATION=1 \
target/debug/integration test_zh_cn_localization_visual_smoke
```

Result: pass on current `b563723c` plus the focused async find worktree
changes, exit code 0. The integration run used a real display, executed 15
steps, included the runtime app menu and Dock menu title assertion, and saved a
fresh artifact set under
`target/zh-cn-visual-artifacts/test_zh_cn_localization_visual_smoke/2026-05-27T18-02-47`.

Earlier current-branch attempts through `cargo run -p integration --bin
integration -- ...` and the ignored `cargo test -p integration --test
integration ... --ignored` path were terminated while still in the integration
binary link/build stage and did not reach test execution. The direct binary run
above is the current completed result.

Latest completed visual artifacts:

- `target/zh-cn-visual-artifacts/test_zh_cn_localization_visual_smoke/2026-05-27T18-02-47/settings-appearance-language-zh-cn.png`
- `target/zh-cn-visual-artifacts/test_zh_cn_localization_visual_smoke/2026-05-27T18-02-47/terminal-input-zh-cn.png`
- `target/zh-cn-visual-artifacts/test_zh_cn_localization_visual_smoke/2026-05-27T18-02-47/context-chips-zh-cn.png`
- `target/zh-cn-visual-artifacts/test_zh_cn_localization_visual_smoke/2026-05-27T18-02-47/command-search-zh-cn.png`
- `target/zh-cn-visual-artifacts/test_zh_cn_localization_visual_smoke/2026-05-27T18-02-47/agent-input-zh-cn.png`
- `target/zh-cn-visual-artifacts/test_zh_cn_localization_visual_smoke/2026-05-27T18-02-47/command-palette-zh-cn.png`
- `target/zh-cn-visual-artifacts/test_zh_cn_localization_visual_smoke/2026-05-27T18-02-47/toast-zh-cn.png`
- `target/zh-cn-visual-artifacts/test_zh_cn_localization_visual_smoke/2026-05-27T18-02-47/dialog-launch-config-zh-cn.png`

Each artifact is a `2560 x 1600` PNG.

Manual spot-check: the current `agent-input-zh-cn.png` shows the
conversation section header as `当前` and the row timestamp as `刚刚`.

## Completion Audit

Goal item status against the current local branch:

- Rebase or merge latest `upstream/master`: satisfied locally. The review
  candidate is based directly on `upstream/master` at `2566f54a`.
- App-level verification: satisfied locally. The listed `cargo test`,
  `cargo check`, `jq empty`, and `git diff --check` commands have passing
  results recorded above.
- zh-CN visual review: satisfied for the current `8f8ff4a8` run. The
  real-display smoke covers Settings language selection, menus/Dock assertions,
  Agent input, Terminal input, Search, Context chips, toast, and dialog
  screenshots, with fresh artifacts under
  `target/zh-cn-visual-artifacts/test_zh_cn_localization_visual_smoke/2026-05-27T18-02-47`.
- Terminology review: satisfied for the catalog-level review scope recorded
  above. The 71 ASCII-only zh-CN values and ASCII-with-CJK values were reviewed;
  remaining English fragments are intentional product, protocol, command, field,
  placeholder, or formatting text.
- Terminology consistency: satisfied for the reviewed terms. Catalog term scans
  report zero remaining `智能体`, `Workflows`, `Notebooks`, `Use Agent`,
  `Copyright`, `pane`, `handoff`, `snapshot`, `payload`, and `pull request`
  matches in zh-CN values.
- Key and placeholder integrity: satisfied. `en-US` and `zh-CN` both contain
  5576 keys, with 0 missing keys, 0 extra keys, and 0 placeholder mismatches.
- Local commit readiness: satisfied locally for this pass. The large
  localization commit is already present locally, and the latest async find
  follow-up plus this evidence update were validated together before commit.
- PR creation or update: pending push.

## Review Handoff Draft

Suggested local commit title:

```text
Localize Warp UI for zh-CN
```

Suggested review summary:

- Adds the `warp_localization` crate, bundled `en-US` and `zh-CN` catalogs, and
  app language settings for System, English, and Simplified Chinese.
- Migrates UI copy across Agent, Settings, Terminal, Search, Workspace, menus,
  dialogs, toasts, and shared UI components to catalog-backed strings.
- Adds catalog integrity tests, direct-English regression checks, app-level
  localization tests, and a manual real-display zh-CN visual smoke test with
  screenshot artifacts.
- Keeps product and protocol terms such as `Agent`, `Warp Drive`, `Notebook`,
  `MCP`, `API`, `CLI`, `ID`, `JSON`, and `Shell` in English intentionally.

## Branch Relationship Notes

The original branch names referenced by the handoff are no longer the active
completion basis for this local worktree:

- Active local branch: `feat/localization-settings-upstream-rebuild`, based on
  `upstream/master` at `2566f54a`.
- `origin/feat/localization-settings` remains at `2f6fcabb`. Compared with the
  active branch, `git rev-list --left-right --count
  HEAD...origin/feat/localization-settings` returns `60 7`, with merge base
  `f3dd3768`.
- `origin/feat/localization-settings-reviewed` remains at `1ce47b43`. Compared
  with the active branch, `git rev-list --left-right --count
  HEAD...origin/feat/localization-settings-reviewed` returns `114 2`, with
  merge base `be5b39ae`.
- `origin/feat/localization-settings-upstream-validated` remains at `58ef5d5f`.
  Compared with the active branch, `git rev-list --left-right --count
  HEAD...origin/feat/localization-settings-upstream-validated` returns `18 8`,
  with merge base `0b737e22`.

Recommendation: treat `feat/localization-settings-upstream-rebuild` as the
current review candidate and leave the older `reviewed` branch as historical
reference only. Avoid force-updating or deleting older remote branches during
review setup.

## Remaining Risk

- PR review remains pending until the latest local follow-up is committed, the
  candidate branch is pushed, and a GitHub pull request is linked.
- The May 27 rebase autostash remains in the stash list as a conservative
  backup; it was not dropped during this pass.
- Visual coverage now includes Settings language selection, terminal input,
  context chips, command search, Agent input, command palette, a localized
  workspace toast, and a Launch Config save dialog. App menu and Dock menu
  titles have runtime integration coverage plus static regression coverage for
  custom item titles, but this still does not exhaustively cover every platform
  surface.
- `origin/feat/localization-settings-reviewed` is an older divergent reference
  and is not used as the completion basis for this branch.
