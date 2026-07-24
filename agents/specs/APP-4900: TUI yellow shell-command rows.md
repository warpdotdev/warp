*Proposed change: Add a yellow background to TUI shell-command rows*

*Summary:* In the Warp TUI slash-command inline menu, rows backed by the `SlashCommand` action currently render with the same unselected background as saved prompts and skills. Carry the existing row-kind information through the TUI snapshot and paint those shell-command rows with the theme's yellow terminal color, matching the Figma design.

*Key design choices:*
- Represent the semantic row kind in the render-facing TUI row rather than inferring it from display text, so command names, saved-prompt names, and skill names can overlap safely.
- Apply the yellow background only to unselected `SlashCommand` rows; the existing cyan selected-row background remains the higher-priority state for every menu row.
- Keep the change in the shared inline-menu renderer and theme builder so slash-command, saved-prompt, and skill data sources retain their existing layout, text styles, navigation, and acceptance behavior.

*Design alternatives:*
- *Infer the row type from the title or prefix* — rejected because saved prompts and skills can have arbitrary names and because text is presentation data, not a stable discriminator.
- *Add a separate renderer only for the slash-command menu* — rejected because the TUI already centralizes row painting in `menu_result_row`; it would duplicate layout and selection logic and make future inline-menu variants diverge.
- *Use a hard-coded yellow color* — rejected because custom/light themes expose their terminal palette through `WarpTheme`; use the builder's theme-derived color, following the existing solid/opacity-blended background recipes.

*Root cause / approach:* `AcceptSlashCommandOrSavedPrompt` already distinguishes `SlashCommand`, `SavedPrompt`, and `Skill` in `app/src/terminal/input/slash_commands/mod.rs`. `TuiSlashCommandRow` retains the action while building query results, but `TuiSlashCommandModel::snapshot` currently maps every result to the same `TuiInlineMenuRowStyle::InlineMenuItem`, so `crates/warp_tui/src/inline_menu.rs:menu_result_row` cannot know which rows should receive the yellow treatment. Add a small semantic row-kind discriminator to the TUI row model/snapshot, populate it from the action variant, add a theme-derived yellow background recipe to `TuiUiBuilder`, and have `menu_result_row` select the background in this order: selected cyan background first, otherwise yellow for `SlashCommand`, otherwise the existing unset background. The selected text style and all existing title/description layout rules remain unchanged unless a contrast-preserving builder style is needed for the yellow fill.

*Affected files:*
- `crates/warp_tui/src/inline_menu.rs` — row-kind data and background selection in `menu_result_row`.
- `crates/warp_tui/src/slash_commands.rs` — preserve/propagate the action kind from `TuiSlashCommandRow` into `TuiInlineMenuRow` snapshots.
- `crates/warp_tui/src/tui_builder.rs` — theme-derived yellow shell-command row background (and, only if required by contrast, its paired foreground style).
- `crates/warp_tui/src/inline_menu_tests.rs` and/or `crates/warp_tui/src/slash_commands_tests.rs` — rendering and propagation regression coverage.

*Open questions resolved:*
- *Which rows count as shell-command rows?* Only rows whose `AcceptSlashCommandOrSavedPrompt` action is the `SlashCommand` variant (static slash commands). `SavedPrompt` and `Skill` rows keep their current unselected background.
- *What happens when a shell-command row is selected?* Selection wins: the existing cyan selection background and selected text style remain unchanged for the selected row, so yellow is visible only while the command row is unselected.
- *Should color be hard-coded or theme-aware?* Theme-aware: derive it from `WarpTheme.terminal_colors().normal.yellow` through `TuiUiBuilder`, using the same solid/pre-blended cell-color conventions as the existing cyan selection and other TUI surfaces.
- *Does the GUI inline menu change?* No. This ticket is TUI-only; GUI `SearchItem` rendering and its hover/selection backgrounds remain untouched.

*Risks / blast radius:* The main risk is losing the discriminator while converting query results to snapshots, which could color all inline-menu rows or none. A mixed-kind render test must assert each row independently. Selection precedence must remain explicit so keyboard navigation does not change its current appearance. The shared renderer is also used by conversation/model/MCP/skill menus; their rows must retain the current backgrounds and text/layout behavior. Theme-derived color and contrast tests should cover at least the mock/light theme path, while the full presubmit catches other theme/build regressions.

*Validation & verification criteria* (must ALL pass before merge):
1. *Kind propagation regression:* add a unit test that constructs representative `SlashCommand`, `SavedPrompt`, and `Skill` TUI rows/actions, obtains the slash-command snapshot, and asserts the render-facing rows preserve three distinct semantic kinds in the original order. The test must fail before the change because `TuiSlashCommandModel::snapshot` currently emits only `InlineMenuItem`.
2. *Yellow row rendering regression:* add a render-to-buffer test in `crates/warp_tui/src/inline_menu_tests.rs` with at least one unselected row of each semantic kind. Assert the unselected `SlashCommand` row's cell background equals the builder's theme-derived yellow background, while the unselected saved-prompt and skill rows retain the existing non-yellow background. Assert the command title/description columns and text colors remain present.
3. *Selection precedence:* extend the render test with a selected `SlashCommand` row and assert its background equals `slash_command_selection_background()` and its foreground/modifiers remain the existing selected style, not yellow. Also verify a selected saved prompt/skill continues to use cyan.
4. *No collateral menu behavior:* `cargo nextest run -p warp_tui --lib inline_menu slash_commands` passes, including existing layout, truncation, selection, scrolling, empty/loading, and exact-match tests; conversation/model/MCP/skill snapshots still render without yellow backgrounds unless their own existing selection state applies.
5. *Theme and formatting checks:* the new builder recipe is asserted against the theme's `normal.yellow` value (and any documented pre-blend) in `crates/warp_tui/src/tui_builder_tests.rs`; `cargo fmt --all -- --check` passes.
6. *Presubmit:* `./script/presubmit` passes from the target repository root.
7. *Visual proof for the user-facing TUI change:* run the affected TUI slash-command menu in a real PTY (using the repository's TUI verification path, with computer-use screenshot/video capture where available) and capture a frame showing mixed rows: an unselected static slash-command row with the yellow background, an unselected saved-prompt/skill row without yellow, and a selected row with the existing cyan background. Confirm the menu remains legible and the title/description columns are unchanged. Attach the proof to the task record and PR; do not commit media.
8. *GUI scope guard:* the implementation diff contains no changes under the GUI inline-menu renderer (`app/src/terminal/input/slash_commands/search_item.rs` or `app/src/terminal/input/inline_menu/styles.rs`), and existing GUI behavior remains out of scope.
