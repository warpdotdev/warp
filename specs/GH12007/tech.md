# Double-click selection CJK and full-width punctuation boundaries — Tech Spec
Product spec: `specs/GH12007/product.md`
GitHub issue: https://github.com/warpdotdev/warp/issues/12007
Inspected commit: `ac4225c1805811a46bfa9df7531e6a4f0058ab12`
## Context
The product behavior is specified in `specs/GH12007/product.md`. The current double-click path expands `SelectionType::Semantic` through terminal/grid word-boundary helpers, while the previously fixed path-detection issue uses a separate separator helper.
Relevant code:
- [`crates/warpui_core/src/text/words.rs:2 @ ac4225c`](https://github.com/warpdotdev/warp/blob/ac4225c1805811a46bfa9df7531e6a4f0058ab12/crates/warpui_core/src/text/words.rs#L2) defines `DEFAULT_WORD_BOUNDARY_CHARS` as whitespace plus a fixed mostly-ASCII punctuation list, with only `«` and `»` as non-ASCII entries.
- [`crates/warpui_core/src/text/words.rs (34-35) @ ac4225c`](https://github.com/warpdotdev/warp/blob/ac4225c1805811a46bfa9df7531e6a4f0058ab12/crates/warpui_core/src/text/words.rs#L34-L35) implements `is_default_word_boundary` as `c.is_whitespace() || DEFAULT_WORD_BOUNDARY_CHARS.contains(&c)`, which explains why `，` is currently treated as word content.
- [`crates/warpui_core/src/text/word_boundaries.rs (14-18) @ ac4225c`](https://github.com/warpdotdev/warp/blob/ac4225c1805811a46bfa9df7531e6a4f0058ab12/crates/warpui_core/src/text/word_boundaries.rs#L14-L18) defines `WordBoundariesPolicy::Default`, `Custom`, and `OnlyWhitespace`; [`word_boundaries.rs (152-158) @ ac4225c`](https://github.com/warpdotdev/warp/blob/ac4225c1805811a46bfa9df7531e6a4f0058ab12/crates/warpui_core/src/text/word_boundaries.rs#L152-L158) routes the default policy through `is_default_word_boundary`.
- [`crates/warp_core/src/semantic_selection/mod.rs (192-213) @ ac4225c`](https://github.com/warpdotdev/warp/blob/ac4225c1805811a46bfa9df7531e6a4f0058ab12/crates/warp_core/src/semantic_selection/mod.rs#L192-L213) bridges user settings to word-boundary behavior. `is_word_boundary_char` calls `is_default_word_boundary` unless Smart Select is disabled and a character is explicitly allowlisted; `word_boundary_policy` builds a `WordBoundariesPolicy` for editor/selectable text paths.
- [`app/src/terminal/model/selection.rs (479-519) @ ac4225c`](https://github.com/warpdotdev/warp/blob/ac4225c1805811a46bfa9df7531e6a4f0058ab12/app/src/terminal/model/selection.rs#L479-L519) expands semantic selection by first computing fallback left/right word bounds, then letting Smart Select widen the range only when it finds a larger recognized object.
- [`app/src/terminal/model/grid/grid_handler.rs (1987-2030) @ ac4225c`](https://github.com/warpdotdev/warp/blob/ac4225c1805811a46bfa9df7531e6a4f0058ab12/app/src/terminal/model/grid/grid_handler.rs#L1987-L2030) implements `semantic_search_left` and `semantic_search_right`; both accept an `is_word_boundary_char` callback from `Selection::range_semantic`.
- [`app/src/terminal/model/blocks/selection.rs (1217-1338) @ ac4225c`](https://github.com/warpdotdev/warp/blob/ac4225c1805811a46bfa9df7531e6a4f0058ab12/app/src/terminal/model/blocks/selection.rs#L1217-L1338) converts block-list selections to grid selections and calls `selection.to_range(&grid.grid_handler, semantic_selection)`.
- [`app/src/terminal/model/grid/grid_handler.rs (27,105-122) @ ac4225c`](https://github.com/warpdotdev/warp/blob/ac4225c1805811a46bfa9df7531e6a4f0058ab12/app/src/terminal/model/grid/grid_handler.rs#L27-L122) already uses `unicode-general-category` in `is_file_link_separator` to treat non-ASCII open/close/initial/final/other punctuation as file-link boundaries. That prior fix deliberately excludes connector and dash punctuation for filename reasons; double-click word selection has different parity requirements because ASCII `-` is already a word boundary.
- [`app/Cargo.toml:215 @ ac4225c`](https://github.com/warpdotdev/warp/blob/ac4225c1805811a46bfa9df7531e6a4f0058ab12/app/Cargo.toml#L215) already depends on `unicode-general-category` via the workspace. [`crates/warpui_core/Cargo.toml @ ac4225c`](https://github.com/warpdotdev/warp/blob/ac4225c1805811a46bfa9df7531e6a4f0058ab12/crates/warpui_core/Cargo.toml) does not yet declare that dependency, even though `words.rs` lives in `warpui_core`.
## Proposed changes
### 1. Add a shared non-ASCII Unicode punctuation predicate in `warpui_core`
In `crates/warpui_core/src/text/words.rs`, add a helper such as:
- `pub fn is_non_ascii_unicode_punctuation(c: char) -> bool`
The helper should:
1. Return `false` for ASCII characters so the existing explicit ASCII boundary list remains the source of truth for ASCII behavior.
2. Return `true` for Unicode General Category punctuation classes:
   - `ConnectorPunctuation`
   - `DashPunctuation`
   - `OpenPunctuation`
   - `ClosePunctuation`
   - `InitialPunctuation`
   - `FinalPunctuation`
   - `OtherPunctuation`
3. Return `false` for letters, marks, numbers, symbols, and separators other than whitespace.
Add `unicode-general-category.workspace = true` to `crates/warpui_core/Cargo.toml` and import `get_general_category` / `GeneralCategory` in `words.rs`.
Rationale: the reporter and maintainer explicitly want broad full-width punctuation coverage, not a hard-coded comma fix. Using Unicode general categories avoids an incomplete manual list and aligns conceptually with the previous #10245 path-detection fix.
### 2. Extend default word-boundary behavior
Update `is_default_word_boundary` in `crates/warpui_core/src/text/words.rs` to return true when:
1. `c.is_whitespace()`
2. `DEFAULT_WORD_BOUNDARY_CHARS.contains(&c)`
3. `is_non_ascii_unicode_punctuation(c)`
This makes terminal semantic selection, editor word selection under the default policy, selectable prompt text, and other consumers of the shared helper pick up the same default behavior.
### 3. Preserve custom word-character allowlist semantics
`SemanticSelection::is_word_boundary_char` already short-circuits to `false` when Smart Select is disabled and the user explicitly allowlists a character. Keep that behavior so a user can intentionally make a Unicode punctuation character part of a word in terminal grid selection.
The editor/selectable text path needs equivalent care because `SemanticSelection::word_boundary_policy` returns `WordBoundariesPolicy::Custom(...)` when Smart Select is disabled. If `WordBoundariesPolicy::Custom` simply calls `is_non_ascii_unicode_punctuation(c)` unconditionally, the custom allowlist cannot override Unicode punctuation. Prefer one of these approaches:
1. Add a new policy variant that represents "default boundaries minus this allowlist" and routes through `is_default_word_boundary(c) && !allowlist.contains(&c)`.
2. Or change `WordBoundariesPolicy::Custom` to carry enough information to distinguish explicit custom boundary sets from the SemanticSelection allowlist case.
Keep existing tests for `WordBoundariesPolicy::Custom(HashSet::from(['{', '}']))` passing. That policy is used as a literal custom separator set in tests and should not accidentally become "default plus custom" unless all callers are reviewed.
### 4. Keep Smart Select widening behavior unchanged
Do not change the regexes in `crates/warp_core/src/semantic_selection/mod.rs` for this issue. The fallback word bounds should stop at Unicode punctuation; Smart Select may still widen only when a recognized object contains the click point and yields a larger range.
Important examples:
- `「https://warp.dev」`: fallback word bounds stop at `「` and `」`; URL smart-select widens to `https://warp.dev`, not the brackets.
- `test，next`: no smart-select regex should match the whole string, so selection remains `test` or `next`.
No new setting or feature flag is required.
### 5. Avoid changing file-link separator behavior unless intentionally deduping
`is_file_link_separator` already uses Unicode categories but excludes connector and dash punctuation because paths and identifiers commonly contain those characters. This double-click fix should not broaden or narrow clickable-path behavior as a side effect.
If implementation deduplicates category checks between `words.rs` and `grid_handler.rs`, expose a low-level category helper (for example, `unicode_punctuation_category(c) -> Option<GeneralCategory>`) rather than forcing file-link detection to share the exact same predicate as word selection.
## End-to-end flow
1. The user double-clicks inside terminal text, producing `SelectionType::Semantic` from the existing mouse click-count mapping.
2. `BlockList::expand_selection` converts the block-list point into a grid `Selection` and calls `Selection::to_range`.
3. `Selection::range_semantic` computes fallback word bounds with `GridHandler::semantic_search_left/right`.
4. Those functions call `SemanticSelection::is_word_boundary_char`, which now treats non-ASCII Unicode punctuation as a default word boundary.
5. Fallback bounds stop before/after CJK or full-width punctuation.
6. Smart Select runs as it does today and may widen only for recognized larger objects.
7. `selection_to_string` and rendering consume the corrected range, so selected/copied text no longer includes adjacent punctuation or neighboring words.
## Testing and validation
### Unit tests
1. `crates/warpui_core/src/text/word_boundaries_tests.rs`
   - Add a test for default word starts/ends over a string such as `test，next。終わり？done`.
   - Assert boundaries occur around each punctuation mark.
   - Add a control assertion that `你好世界` and `ｔｅｓｔ１２３` are each treated as continuous words.
2. `crates/warp_core/src/semantic_selection/mod_tests.rs`
   - Add a test for `SemanticSelection::mock(true, "").is_word_boundary_char('，') == true`.
   - Add tests for representative punctuation from each Unicode punctuation category used by the helper.
   - Add a test that a non-punctuation CJK ideograph such as `你` and a full-width digit such as `１` are not word boundaries.
   - Add a Smart Select disabled allowlist test, for example `SemanticSelection::mock(false, "，").is_word_boundary_char('，') == false`.
3. `app/src/terminal/model/grid/grid_handler_tests.rs`
   - Extend `test_semantic_search` or add a sibling test around `mock_blockgrid("test，next\r\n你好世界\r\n")`.
   - Assert `semantic_search_left/right` from inside `test` stop at `test`, from inside `next` stop at `next`, and from inside `你好世界` return the full ideograph sequence.
4. `app/src/terminal/model/blocks/selection_tests.rs`
   - Add a regression test mirroring the issue reproduction: insert output `test，next\ntest,next\n`, start/update a `SelectionType::Semantic` selection inside each `test`, and assert `selection_to_string(...) == Some("test".to_string())`.
   - Add a smart-select regression around surrounding CJK brackets, such as output `「https://warp.dev」`, asserting a double-click inside the URL selects only `https://warp.dev`.
5. If a new `WordBoundariesPolicy` variant is introduced for allowlist semantics, add direct tests for both the new variant and the existing literal `Custom(HashSet<char>)` behavior so callers cannot confuse the two.
### Manual validation
1. Run `printf 'test，next\ntest,next\n'` in Warp.
2. Double-click `test` on both lines.
3. Copy the selection each time and confirm the clipboard contains exactly `test`.
4. Run or paste examples containing `，。！？；：、「」『』（）《》〈〉【】〔〕` and confirm words on either side select independently.
5. Verify `你好世界` selects as one word.
6. Verify `「https://warp.dev」` selects the URL without surrounding CJK brackets when double-clicking inside the URL.
### Suggested commands
Run targeted Rust tests first:
- `cargo test -p warpui_core text::word_boundaries`
- `cargo test -p warp_core semantic_selection`
- `cargo test -p warp terminal::model::grid::grid_handler_tests::test_semantic_search`
- `cargo test -p warp terminal::model::blocks::selection_tests`
Then run the repository's normal formatting/check workflow for touched Rust files before the implementation PR.
## Parallelization
Parallel implementation is not recommended for this issue. The production change should be a small, tightly-coupled update to shared word-boundary logic plus targeted tests across downstream consumers. Splitting the helper change and tests across multiple agents would likely create coordination overhead and merge conflicts that outweigh any wall-clock savings.
If review uncovers a broader refactor of word-boundary policies, that follow-up could split into two streams: one agent for `warpui_core` policy design and tests, and one agent for terminal/editor call-site validation. That is not necessary for the initial bug fix.
## Risks and mitigations
### Risk: changing a shared helper affects non-terminal selectable text
`is_default_word_boundary` is intentionally shared by terminal semantic selection, editor/selectable text word selection, prompt selectable areas, and utility link helpers. A broad default change could alter double-click behavior outside terminal output.
Mitigation: this is mostly desirable for consistency, but tests should cover terminal output as the required fix and include smoke coverage for `WordBoundariesPolicy::Default`. The implementation PR should call out any observed editor/prompt behavior changes.
### Risk: custom allowlist behavior regresses
The terminal path can keep the existing allowlist short-circuit, but `WordBoundariesPolicy::Custom` may not be expressive enough to mean "default minus allowlist" once default includes Unicode categories.
Mitigation: add explicit tests for Smart Select disabled with a Unicode punctuation allowlist. If the existing enum cannot express the desired behavior without breaking literal custom separator sets, add a new policy variant rather than overloading `Custom`.
### Risk: dependency placement increases compile surface
Moving Unicode general category checks into `warpui_core` requires adding `unicode-general-category` to that crate.
Mitigation: the dependency is already in the workspace and used by `app`; the helper is small and deterministic. Keep it in `words.rs` with no app-layer dependencies.
### Risk: treating all Unicode punctuation as boundaries differs from file-link separators
The previous file-link fix excludes connector and dash punctuation. Word selection should include dash punctuation because ASCII `-` is already a default word boundary, but connector punctuation may be debatable because ASCII `_` is not a default boundary.
Mitigation: product behavior requires broad full-width punctuation coverage and custom allowlist preservation. If reviewers decide connector punctuation should follow underscore parity, update both specs and tests before implementation; otherwise include all Unicode punctuation categories and document the choice in tests.
## Follow-ups
1. Consider consolidating file-link and word-boundary Unicode punctuation helpers behind a shared lower-level category utility if another code path needs the same logic.
2. Consider documenting the terminal word-character allowlist behavior for Unicode punctuation if users report needing full-width punctuation inside selected words.
