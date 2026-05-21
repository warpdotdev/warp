# Settings i18n and Chinese Language Support TODO

## Goal

Add a language selector in Settings and build the first i18n slice needed to support Simplified Chinese in the Warp UI.

## Scope

- Add a persisted display-language setting.
- Add a Settings > Appearance language selector.
- Add a small i18n lookup layer with English and Simplified Chinese catalogs.
- Localize the Settings sidebar and the initial Appearance language/category surface.
- Keep stable identifiers, telemetry names, config keys, and API values in English.
- Use English fallback for untranslated strings.

## Phase 1 - MVP

- [x] Document the implementation plan in `docs/`.
- [x] Add a typed language preference setting:
  - `system`
  - `english`
  - `chinese_simplified`
- [x] Register the language setting with the settings manager.
- [x] Add an i18n module that resolves the current locale from the language setting.
- [x] Add English and Simplified Chinese translations for the Settings navigation shell.
- [x] Add a language dropdown to Settings > Appearance.
- [x] Re-render Settings navigation and the Appearance page when the language setting changes.
- [x] Preserve English labels for logs, storage keys, telemetry, and backwards-compatible parsing.
- [x] Run formatting/checks for the touched Rust files.

## Phase 2 - Settings Coverage

- [x] Localize shared Settings page titles and category headings.
- [x] Localize remaining in-page headings rendered inside monolith widgets.
- [x] Localize all Appearance page labels, descriptions, and dropdown item names.
- [x] Localize Warp Drive and Warpify page labels, descriptions, placeholders, and dropdown items.
- [x] Localize Account page labels, plan actions, referral CTA, and update status strings.
- [x] Localize Privacy page labels, descriptions, dropdown items, and add-regex modal strings.
- [x] Localize Code page indexing, editor/code review, LSP status, and external editor controls.
- [x] Localize Keyboard shortcuts page labels, editor actions, search placeholder, and sync notice.
- [x] Localize Cloud platform API keys page, create-key modal, table labels, empty state, and API-key toasts.
- [x] Localize Features and AI settings pages.
  - [x] Localize Features page main widgets, dropdowns, helper rows, notifications, and hotkey controls.
  - [x] Localize AI settings main sections: Warp Agent, usage, Active AI, Input, MCP, Knowledge, Voice, Other, third-party CLI agents, agent attribution, and cloud-agent experimental controls.
  - [x] Localize AI profiles, execution-profile cards, BYOK/API-key settings, AWS Bedrock settings, and core AI permission dropdown labels.
  - [x] Localize remaining AI-owned nested editors and dropdown item names that come from shared model/display APIs.
- [x] Add localized search terms while preserving English aliases.
- [x] Add missing-key diagnostics for development builds.
- [x] Add tests that fail when Chinese and English catalogs drift.

## Phase 3 - App-Wide Coverage

- [ ] Localize common buttons, menus, toasts, modals, and error banners.
  - [x] Native app menu bar and dock menu hardcoded labels in `app/src/app_menus.rs`.
  - [x] Workspace new-session menu and tab/session core menus in `app/src/workspace/view.rs` and `app/src/tab.rs`.
  - [x] Terminal high-frequency context menu items for selected text, blocks, prompts, AI blocks, and pane splits.
  - [x] Terminal inline banners, share-block modal, and shared-session controls.
  - [x] Warp Drive object menus, new-object menus, notebooks, workflows, and environment-variable collection pane actions.
  - [x] Warp Drive sharing dialog, import modal, object-limit banners, and low-frequency tooltips.
  - [x] Code editor, file tree, code review, comment list, and diff-set menus.
  - [x] Settings residuals: Billing and usage, Teams, MCP server cards/modals, Environments, Shared blocks, and pane context menus.
- [x] Localize onboarding and update flows.
  - [x] Auth/sign-in views, SSO link flow, paste-token modal, welcome tips, get-started panes, and first-run NUX.
- [x] Localize terminal-adjacent UI that is not shell output.
- [ ] Localize AI feature chrome without localizing model responses.
  - [x] Agent management filters, model selectors, prompt alerts, credit banners, permissions prompts, and AI document menus.
- [ ] Add pseudo-locale support for layout stress testing.
- [ ] Add screenshot checks for common desktop sizes.

## Coverage Audit Notes

- A heuristic scan for direct UI strings now finds 10 remaining call sites outside tests.
- The remaining 10 are intentionally preserved technical text: key names (`ESC`), file names (`launch_config.yaml`), shell commands (`nvm install node`, `aws login`), example placeholders, regex examples, numeric placeholders, and copyright text.
- A wider scan for common UI constructors (`label`, `link`, `Dialog::new`, `FormattedTextElement::from_str`, `button::Content::Label`, `wrappable_text`) is clean outside the intentionally preserved technical scan.
- 2026-05-02 follow-up covered native menu hardcoded labels, workspace new-session menu labels, tab context menus, Settings pane split/close menu labels, and common terminal context menu labels. Some `CustomAction` labels still come from the keybinding description registry and need a broader binding-description localization pass.
- 2026-05-02 follow-up also covered Warp Drive new-object/context menus, trash title/empty-trash/offline/team zero-state labels, Notebook/Workflow pane menus and restore actions, and Environment Variables overflow/secret menus plus save/variables/trash-banner controls.
- 2026-05-02 follow-up covered Code editor and Code Review chrome: file tree context menus, diff navigation, local code editor/LSP menus, pending diff accept/reject buttons, diff selector and go-to-line placeholders, code pane overflow items, comment composer/actions, git operation menus, and file navigation/tooltips.
- 2026-05-02 broad coverage pass added `tr_static` for low-reuse UI literals and covered high-frequency Agent Management, AI document/block chrome, terminal banners/share/shared-session controls, Notebook link/block/file actions, search empty/loading states, theme chooser hints, Teams/MCP/Billing residual buttons, and several onboarding/credit/action modals.
- 2026-05-02 final sweep covered auth/sign-in copy, welcome tips, notification mailbox/toasts, shared-block dialogs, terminal-adjacent buttons/tooltips, WASM NUX, build-plan migration modal, and residual Settings/Drive labels. `cargo check -p warp`, i18n catalog test, and `git diff --check` passed after this sweep.
- Keep technical identifiers, telemetry names, config keys, URL labels that are product names, shell commands, model/provider names, file paths, regex examples, and API values in English unless product explicitly wants translated display aliases.

## Follow-Up Decisions

- [ ] Decide whether to replace the MVP catalog with Fluent `.ftl` resources.
- [ ] Decide whether language preference should sync globally or remain local-only.
- [ ] Add robust platform locale detection for the `system` option.
- [ ] Decide how to expose language choice to CLI/headless flows, if at all.
