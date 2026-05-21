# Remaining i18n Board Scan (2026-05-07)

## Scope

This is a board-level scan of likely remaining untranslated user-facing English in
render paths under `app/src`.

The scan intentionally focuses on UI construction sites such as:

- `wrappable_text(...)`
- `Text::new(...)`
- `Text::new_inline(...)`
- `with_text_label(...)`
- `tool_tip(...)`
- `link(...)`

It excludes obvious tests, logs, protocol files, and generated content.

This is a heuristic report, not a perfect runtime truth table. Some hits are
technical or intentionally left in English. The point is to identify the
remaining high-value surfaces to inspect and finish.

## High-Signal Remaining Boards

### 1. `workspace/view.rs`

Still the largest pool of user-visible English candidates.

Examples surfaced by scan:

- ` + Add new repo`
- `Troubleshoot notifications`
- `Some features may be unavailable offline`
- several toasts / helper labels in workspace shell, title bar, and launch flows

Assessment:

- High priority
- Large user-facing surface
- Mixed toolbar, toast, launch, and side-panel UI

### 2. `code_review/code_review_view.rs`

Still contains many user-visible empty / loading / action labels.

Examples surfaced by scan:

- `View changes`
- `Discard uncommitted changes?`
- `Commit`
- `Commit and create PR`
- `No open changes`
- `Reviewing code changes`
- `Diff is too large to render`
- `Binary file - no diff available`
- `No file selected`
- `No files to discard`

Assessment:

- High priority
- Very visible in right panel flows
- Good next target after keyboard shortcuts / settings surfaces

### 3. `settings_view/environments_page.rs`

This is the environments board the user already hit.

Examples surfaced by scan:

- `Environments`
- `Retry`
- `Launch agent`
- `Quick setup`
- `Suggested`
- setup-card descriptions and search placeholder

Assessment:

- High priority
- Entire empty-state / onboarding surface still has English candidates

### 4. `settings_view/mcp_servers/*`

Core list page and card/update/edit modal surfaces still need more coverage
despite the latest pass.

Examples surfaced by scan:

- edit/update modal titles and labels
- `Save`
- `Edit Variables`
- card-level explanatory text

Assessment:

- Medium-high priority
- We already touched the list page; edit/update/install dialogs likely still have
  follow-on gaps

### 5. `resource_center/*`

Main page and section cards still expose English copy outside the keyboard
shortcuts page.

Examples surfaced by scan:

- section item titles / descriptions
- changelog section copy
- footer labels

Assessment:

- Medium priority
- Keyboard shortcuts is much improved, but the rest of the resource center still
  needs a pass

### 6. `settings_view/billing_and_usage_page.rs`

Many clear user-facing labels remain.

Examples surfaced by scan:

- `Add-on credits`
- `Purchased this month`
- `Total overages`
- `Last 30 days`
- `No usage history`
- `Plan`
- `Contact support`

Assessment:

- High priority
- Dense settings surface with lots of commercial copy

### 7. `workspace/view/conversation_list/view.rs`

Examples:

- `No conversations yet`
- `New conversation`

Assessment:

- Medium priority
- Small but very visible empty-state surface

### 8. `workspace/view/right_panel.rs`

Examples:

- `Open repository`
- `Close panel`
- `Code review`

Assessment:

- Medium priority
- Small surface, easy cleanup

### 9. `settings_view/referrals_page.rs` / `billing_and_usage/overage_limit_modal.rs` /
`transfer_ownership_confirmation_modal.rs`

Examples:

- `Sign up`
- `Update`
- `Cancel`
- `Transfer`

Assessment:

- Medium priority
- Modal / transactional flows, small but user-visible

### 10. `auth/*`

Examples from scan:

- multiple `link(...)` and auth flow text in `login_slide.rs` / `auth_view_body.rs`

Assessment:

- Medium priority
- Login / signup surfaces still likely have residual English

## Lower-Signal / Review Before Changing

These need manual judgment; they may be technical or intentionally English:

- key names / keystroke labels like `Meta`, `ESC`, `Delete`
- URLs / mailto links
- debug-only labels like `[Debug] ...`
- brand / product nouns such as `Warp`, `MCP`, `GitHub`, `Slack`, `Oz`
- code / shell / repo labels that might be better left literal

## Recommended Next Sweep Order

1. `app/src/settings_view/environments_page.rs`
2. `app/src/code_review/code_review_view.rs`
3. `app/src/settings_view/billing_and_usage_page.rs`
4. `app/src/resource_center/main_page.rs` and remaining section views
5. `app/src/workspace/view.rs`
6. `app/src/workspace/view/conversation_list/view.rs`
7. `app/src/settings_view/mcp_servers/edit_page.rs`, `update_modal.rs`, `server_card.rs`
8. `app/src/auth/*`

## Practical Note

The keyboard shortcuts board required two separate fixes:

- add missing string entries to `app/src/i18n.rs`
- make both the Resource Center and Settings keybindings views render
  descriptions through the i18n layer

Other boards may have the same two-layer problem: string exists in source, but
the render path never passes through translation.

