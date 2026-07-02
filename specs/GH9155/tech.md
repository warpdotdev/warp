# Tech Spec: Search sessions by renamed tab and pane name

**Issue:** [warpdotdev/warp#9155](https://github.com/warpdotdev/warp/issues/9155)

## Context

The session navigation palette builds the text it matches a query against from a
single function shared by both search backends. Custom tab and pane names are
stored on different models and are not part of that text or of the
`SessionNavigationData` the palette consumes.

Relevant code:

- `app/src/search/command_palette/navigation/search.rs` — `searchable_session_string_and_ranges()` builds the searchable string (`[prompt] [command] [hint text]`). Both `FuzzySessionSearcher` and `FullTextSessionSearcher` (Tantivy) call it, so it is the single place that determines what is matchable. `render` of a row happens in `render.rs`.
- `app/src/session_management.rs` — `SessionNavigationData` is the per-session record the palette consumes (`prompt`, `command_context`, `pane_view_locator`, status, etc.). It carries no custom tab/pane name. `all_sessions()` → `Workspace::workspace_sessions` is the data source.
- `app/src/workspace/view.rs` — `workspace_sessions()` iterates tabs and calls `PaneGroup::pane_sessions()` per tab.
- `app/src/pane_group/mod.rs` — `pane_sessions()` (the level with the owning pane group) maps panes to `SessionNavigationData`. `PaneGroup::custom_title` holds the rename-tab title; `set_title()` (rename-tab via `Workspace::rename_tab`) writes it.
- `app/src/pane_group/pane/terminal_pane.rs` — `session_navigation_data()` constructs each `SessionNavigationData` from the terminal view.
- `app/src/pane_group/pane/mod.rs` — `PaneConfiguration::custom_vertical_tabs_title` holds the rename-pane name; `set_custom_vertical_tabs_title()` writes it (via `Workspace::set_custom_pane_name` / rename-pane). `title()` is the auto terminal title and is **not** what we want.
- `app/src/search/command_palette/navigation/render.rs` — `render_session_label()` lays out a row as a column (prompt, then command/status).

## Current state

`searchable_session_string_and_ranges()` reads only `prompt`, `command_context`,
and the status hint from `SessionNavigationData`. The custom tab name lives on
`PaneGroup` and the custom pane name on `PaneConfiguration`; neither is threaded
into `SessionNavigationData`, so the palette can neither match nor display them.

## Proposed changes

### 1. Carry the names on `SessionNavigationData` (`session_management.rs`)

Add two optional fields with getters/setters that drop empty strings:

```rust
tab_name: Option<String>,   // PaneGroup::custom_title (rename-tab)
pane_title: Option<String>, // PaneConfiguration::custom_vertical_tabs_title (rename-pane)
```

Add a `display_name()` helper returning `pane_title().or_else(|| tab_name())`
(pane name preferred, then tab name) for the render layer. Both fields default to
`None` in `new()`; they are stamped after construction by the two call sites that
have the respective data.

### 2. Stamp the tab name in `PaneGroup::pane_sessions` (`pane_group/mod.rs`)

`pane_sessions` is the level that owns the pane group, so it reads
`self.custom_title(app)` once and sets it on each session:

```rust
let tab_name = self.custom_title(app);
self.panes_of::<TerminalPane>().map(move |pane| {
    let mut session = pane.session_navigation_data(pane_group_id, window_id, app);
    session.set_tab_name(tab_name.clone());
    session
})
```

### 3. Stamp the pane name in `TerminalPane::session_navigation_data` (`terminal_pane.rs`)

Read the custom (not auto) pane title and set it on the session:

```rust
let pane_title = self
    .pane_configuration()
    .as_ref(app)
    .custom_vertical_tabs_title()
    .map(|t| t.to_string());
// ...build session...
session.set_pane_title(pane_title);
```

### 4. Append both to the searchable string (`search.rs`)

After the existing prompt/command/hint assembly, append each present name so
both backends match it. No highlight range is tracked for the names (matchable,
not range-highlighted):

```rust
for name in [session.tab_name(), session.pane_title()].into_iter().flatten() {
    searchable_string.push(' ');
    searchable_string.push_str(name);
}
```

### 5. Show the name in the row (`render.rs`)

In `render_session_label`, when `session.display_name()` is `Some`, add it as the
top line of the row (monospace, themed text color), above the prompt.

## Testing and validation

- **Unit/integration** (`app/src/workspace/view_tests.rs`): added
  `test_workspace_sessions_carry_custom_tab_and_pane_names`, which renames a tab
  and a pane and asserts the resulting `SessionNavigationData` carries
  `tab_name`/`pane_title` and that `display_name` prefers the pane name. Covers
  invariants 1–5 and 8 at the data-source level (the searchable string and row
  render are pure functions of these fields).
- **Backend coverage** (invariant 7): the names feed
  `searchable_session_string_and_ranges`, which both `FuzzySessionSearcher` and
  `FullTextSessionSearcher` consume, so a single change covers both.
- **Manual** (invariants 1–6): `./script/run`, rename tabs/panes, open the
  navigation palette, confirm name matches surface the session and the name is
  shown on the row; confirm un-named sessions are unchanged. Before/after
  screenshots attached to the PR.
- **Lint/format**: `cargo clippy -p warp --all-targets --tests -- -D warnings`
  and the project formatter pass.

## Risks / tradeoffs

- Including names in the matched text slightly broadens matches; scoped to
  user-assigned names only (not auto titles) to avoid noise.
- The names are matchable but not range-highlighted in the row; highlighting
  could be a follow-up if desired.
