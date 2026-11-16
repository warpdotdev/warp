# TECH.md
Companion to `specs/REMOTE-1455/PRODUCT.md`.
## Context
Add a harness row to the conversation details panel. Behavior lives in `PRODUCT.md`; this plan covers where the row plugs in and how we source the harness value.
Relevant code:
- `app/src/ai/conversation_details_panel.rs (75-100)` — `PanelMode` enum (`Conversation` / `Task`) that backs the panel.
- `app/src/ai/conversation_details_panel.rs (177-198)` — `ConversationDetailsData` struct. All new data the row needs must flow through here.
- `app/src/ai/conversation_details_panel.rs (222-414)` — constructors: `from_conversation`, `from_task`, `from_task_id`, `from_conversation_metadata`.
- `app/src/ai/conversation_details_panel.rs (950-1028)` — `render_skill_section`, the closest existing precedent for a single icon + label row.
- `app/src/ai/conversation_details_panel.rs (1426-1781)` — `View::render` composes the sidebar; sections are appended to a `Flex::column` in a fixed order with `FIELD_SPACING` / `HEADER_SPACING` margins. This is where the new row is inserted.
- `app/src/ai/ambient_agents/task.rs (57-83)` — `AgentConfigSnapshot.harness: Option<HarnessConfig>`, where `HarnessConfig { harness_type: String }` — the source of truth for a task's harness once the snapshot is loaded.
- `crates/warp_cli/src/agent.rs (118-138)` — `Harness` enum (`Oz` / `Claude` / `Gemini`) with `clap::ValueEnum`; `Harness::from_str` (via `ValueEnum`) is the canonical parser for `harness_type` strings (values: `oz`, `claude`, `gemini`).
- `app/src/terminal/view/ambient_agent/harness_selector.rs (59-75)` — existing `display_name` / `icon_for` helpers used by the harness selector dropdown. We reuse and centralize these so the two surfaces cannot diverge.
- `app/src/ai/conversation_details_panel_tests.rs` — existing unit-test harness (`App::test`) used for `ConversationDetailsData`.
## Proposed changes
### 1. Centralize harness display metadata
Extract the display name + icon + brand color mapping out of `harness_selector.rs` into a small shared module (`app/src/ai/harness_display.rs`), exposing:
- `pub fn display_name(harness: Harness) -> &'static str` → `"Warp Agent"`, `"Claude Code"`, `"Gemini CLI"`.
- `pub fn icon_for(harness: Harness) -> Icon` → `Warp` / `ClaudeLogo` / `GeminiLogo`. Oz maps to `Icon::Warp` (not `Icon::OzCloud`) so first-party Warp surfaces visually match the existing skill row.
- `pub fn brand_color(harness: Harness) -> Option<ColorU>` → `None` for Oz (caller falls back to theme foreground), `CLAUDE_ORANGE` for Claude, `GEMINI_BLUE` for Gemini. `CLAUDE_ORANGE` is re-used from `crate::ai::blocklist`; `GEMINI_BLUE` mirrors the value already in `terminal::cli_agent`.
- `pub fn parse_harness_type(raw: &str) -> Option<Harness>` — thin wrapper around `Harness::from_str(raw, /* ignore_case */ true)`. Unknown strings return `None`.
- `impl From<AIAgentHarness> for Harness` — 1:1 map from the `ServerAIConversationMetadata.harness` enum (`Oz` / `ClaudeCode` / `Gemini`) to `Harness`. Used by the conversation-sourced constructors so they can resolve the real harness instead of hardcoding Oz.
Update `harness_selector.rs` to call into this module. This renames the selector's `"Oz"` label to `"Warp Agent"` and swaps its leading icon from `Icon::OzCloud` to `Icon::Warp`, matching `PRODUCT.md` invariant 2 and keeping the two surfaces in sync.
### 2. Data model: thread harness through `ConversationDetailsData`
Add `harness: Option<Harness>` to `ConversationDetailsData`. Resolution per constructor:
- `from_conversation` (WASM) — read `conversation.server_metadata().map(|m| Harness::from(m.harness))`; fall back to `Some(Harness::Oz)` when the conversation has no server metadata (pure local run).
- `from_conversation_metadata` — takes `harness: Option<Harness>` as an explicit parameter. The management view caller resolves it via `BlocklistAIHistoryModel::get_server_conversation_metadata(&conversation_id).map(|m| Harness::from(m.harness))`, falling back to `Some(Harness::Oz)` for pure local conversations. This covers the edge case where a `ManagementCardItemId::Conversation` card represents a cloud-task-backed conversation whose task row isn't in `self.tasks` (shadowing missed): the server-side `AIAgentHarness` on the merged metadata is the authoritative source.
- `from_task` — compute from `task.agent_config_snapshot`:
    - `Some(config)` with `config.harness = Some(HarnessConfig { harness_type })` → `parse_harness_type(&harness_type)`; unknown strings resolve to `None`.
    - `Some(config)` with `config.harness = None` → `Some(Harness::Oz)` (invariant 3: explicit default).
    - `None` (snapshot not loaded) → `None` (invariant 5: omit until known).
- `from_task_id` → `None`.
`Harness` is `Copy`, so `Option<Harness>` stays cheap and keeps `ConversationDetailsData: Clone`.
### 3. Rendering: `render_harness_section`
Add `fn render_harness_section(&self, appearance: &Appearance) -> Option<Box<dyn Element>>` structured like `render_simple_field` (label-over-value) but with an icon in the value row:
- Returns `None` when `self.data.harness.is_none()` — enforces invariant 6 (no placeholder, no reserved slot).
- Emits a `Flex::column` with two children separated by `LABEL_VALUE_GAP`:
    1. A "Harness" `Text` label, colored with `blended_colors::text_sub(theme, theme.surface_1())` at `ui_font_size` (invariant 2).
    2. A value `Flex::row` containing a 16px `ConstrainedBox`'d `Icon` (margin-right 4px) and a selectable `Text` with the harness display name in `theme.foreground()` (invariants 2, 8).
- Icon tint = `brand_color(harness).map(Into::into).unwrap_or(theme.foreground())`. Warp Agent picks up the theme foreground; Claude and Gemini render in their brand colors (invariants 2 and 3).
- No click target, no copy button, no tooltip (invariant 7).
Insert in `View::render` directly below the Status section and above Artifacts / Directory / Run ID, wrapped in `Container::with_margin_bottom(FIELD_SPACING)` so the field has the same outer spacing as sibling `render_simple_field` fields (invariant 9). The placement is the same across both `PanelMode::Conversation` and `PanelMode::Task`. Because the slot is conditional, when a `from_task_id` stub later resolves into a full task, the panel only grows downward from that row — nothing above it moves (invariant 6's "no content moves out from under the cursor").
No new actions, mouse states, copy-feedback entries, or events. `handle_action` and `PanelMouseStates` are untouched.
### 4. No telemetry, no new surfaces
Row is read-only metadata. No telemetry, no changes to `ConversationDetailsPanelAction`, `ConversationDetailsPanelEvent`, or any caller of `set_conversation_details`.
## Testing and validation
Behavior invariants from `PRODUCT.md` map as follows:
- Invariants 1, 4, 5, 6 — unit tests on `ConversationDetailsData.harness`, added to `app/src/ai/conversation_details_panel_tests.rs`:
    - `from_conversation` with no `server_metadata` → `Some(Harness::Oz)`; with `server_metadata.harness = AIAgentHarness::ClaudeCode` → `Some(Harness::Claude)` (and analogous for Gemini).
    - `from_conversation_metadata` is a pass-through: each of `Some(Oz)` / `Some(Claude)` / `Some(Gemini)` / `None` round-trips into the data struct. (Resolution logic lives in the caller; that's exercised end-to-end via manual verification rather than a dedicated unit test, since it requires a populated `BlocklistAIHistoryModel`.)
    - `from_task_id` → `None`.
    - `from_task` with `agent_config_snapshot = None` → `None`.
    - `from_task` with `agent_config_snapshot = Some { harness: None, .. }` → `Some(Harness::Oz)`.
    - `from_task` with `harness_type` each of `"oz"`, `"claude"`, `"gemini"` → matching `Harness` variant.
    - `from_task` with an unknown `harness_type` string → `None`.
    - Parametrize one of the above across each `AmbientAgentTaskState` variant to lock invariant 5 ("regardless of run status").
- Invariants 2, 3 (icon + label + brand color mapping) — unit tests on the new `harness_display` module covering `parse_harness_type` edge cases. The `display_name` / `icon_for` / `brand_color` mappings are enum match arms and don't get dedicated tests; instead the harness selector's existing item rows and the details row resolve from the same shared helper so they can't drift.
- Invariants 7, 8, 9, 10 — covered structurally: `render_harness_section` has no click handler / mouse state, uses `with_selectable(true)`, is inserted at a fixed offset in `View::render`, and the panel is the sole renderer across all hosting surfaces. Verified by a manual smoke check rather than a dedicated test.
- Manual verification — `cargo run` and, in order: (a) open a local conversation's details panel → row shows "Warp Agent" + `Icon::Warp` in theme foreground; (b) open a cloud task launched with default config → row shows "Warp Agent"; (c) open a cloud task launched with `--harness claude` → row shows "Claude Code" + `ClaudeLogo` tinted Claude orange; (d) open a shared task that still resolves via `from_task_id` before the task loads → confirm no placeholder row, then confirm the row appears once the task payload arrives without rows above visibly jumping.
- Pre-PR — `./script/presubmit` (cargo fmt, cargo clippy, cargo nextest) per repo rules. No WASM-specific paths are added, so WASM build should be unaffected.
## Risks and mitigations
- **Renaming "Oz" → "Warp Agent" in the existing harness selector.** Separate user-visible change on another surface. Required for invariant 2; centralizing in `harness_display.rs` prevents drift. If contested, we can parameterize per-surface without re-splitting the spec.
- **Unknown `harness_type` strings from newer server versions.** Treating unknown strings as `None` means future harnesses temporarily render no row on older clients — strictly better than a wrong label. Mitigation: add the new `Harness` variant in `warp_cli` when the server does.
- **`from_task_id` → `from_task` transition causing layout jump.** Placing the row below Creator means growth is downward-only, so nothing above moves. If product later wants the row higher, we'd need a reserved slot or crossfade; defer until asked.
