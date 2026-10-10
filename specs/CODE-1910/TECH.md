# Shared `@` context menu core

Groundwork for bringing the `@` context menu to the headless TUI. This spec covers the first
branch in a three-PR stack; it is a behavior-preserving refactor with no user-visible change, so
there is no sibling `PRODUCT.md`.

## Context

The `@` menu lets users insert files, code symbols, blocks, Drive objects, plans, rules, diff sets,
conversations, and skills into the agent input. Everything about it lives inside a single GUI view.

- [`app/src/search/ai_context_menu/view.rs` @ 02fe6e4](https://github.com/warpdotdev/warp/blob/02fe6e4e4d08f96fad8faa0a7e85d8df824c15ea/app/src/search/ai_context_menu/view.rs) —
  1711 lines holding the category enum, the navigation state machine, per-category search-source
  wiring, and all rendering. `get_categories_for_mode` (369-503) and `reset_mixer` (824-1031) are
  private methods on the view taking `ViewContext<Self>`.
- [`app/src/terminal/input.rs` @ 02fe6e4](https://github.com/warpdotdev/warp/blob/02fe6e4e4d08f96fad8faa0a7e85d8df824c15ea/app/src/terminal/input.rs) —
  the `@` grammar and buffer rewriting: `should_enable_ai_context` (12112-12167),
  `should_close_ai_context_menu` (10075-10137), and the action-to-text mapping in the
  `AcceptAIContextMenuItem` arm (11258-11360). All private to `Input`.
- [`app/src/search/ai_context_menu/mixer.rs` @ 02fe6e4](https://github.com/warpdotdev/warp/blob/02fe6e4e4d08f96fad8faa0a7e85d8df824c15ea/app/src/search/ai_context_menu/mixer.rs) —
  `AIContextMenuMixer` (a `SearchMixer`) and `AIContextMenuSearchableAction`, both already
  surface-neutral.

Two properties make this extractable rather than a rewrite. The mixer already lives in the
platform-agnostic `warp_search_core` crate, which the TUI drives today for slash commands. And
accepting an item never produces a special attachment object — it rewrites the input buffer with
plain text that `parse_context_attachments` resolves at submission time, on a controller the TUI
already owns.

The blocker is that the reusable logic is entangled with GUI-only state. Category availability,
navigation, and the `@` grammar are surface-neutral in meaning but private to a view and an input.

## Proposed changes

### New `app/src/search/ai_context_menu/core.rs`

- `AIContextMenuCategory` and `NavigationState`, relocated from `view.rs`. `icon()` stays in
  `view.rs` as a separate inherent impl, since it returns SVG asset paths that mean nothing to a
  cell-grid front-end.
- `AtContextMenuGates` — the four availability conditions currently held as loose bools on
  `AIContextMenuState`. Front-ends set the ones that are meaningful for them; conditions a
  front-end has no concept of stay `false`.
- `AtContextMenuCoreState` — the navigation state machine: category availability and filtering,
  category selection with wrap-around, the "filter matches no category, fall through to searching
  all of them" transition, and the consecutive-empty-results counter that closes a menu making no
  progress. It also holds the session working directory (see Risks).
- `is_at_menu_trigger` / `should_close_at_menu` — the `@` grammar, lifted out of `Input`.
- `shared_inserted_text` — the buffer text an accepted action inserts, for the actions whose
  representation is identical on every surface. Returns `None` for `InsertFilePath` (rewritten
  against the working directory in shell mode) and `InsertDiffSet` (attaches context instead of
  inserting text).

### Extended `mixer.rs`

`install_sources_for_category`, `install_sources_for_all_categories`, and
`at_context_menu_query`, mirroring
[`build_slash_command_mixer` / `slash_command_query` @ 02fe6e4](https://github.com/warpdotdev/warp/blob/02fe6e4e4d08f96fad8faa0a7e85d8df824c15ea/app/src/terminal/input/slash_commands/mixer.rs#L10-L47).
Callers reset the mixer, install, then run the query. `AtContextMenuSourceContext` carries the
long-lived handles the sources need but the state cannot own — currently just the
`CodeSymbolCache`.

This collapses duplication that already exists: `reset_mixer` and
`setup_data_sources_for_all_categories` each carried a full copy of the per-category match arms.
407 lines become 39.

### GUI migration

`AIContextMenu` gains a `core: AtContextMenuCoreState` field and keeps `SearchBar` /
`SearchBarState` for result selection and rendering untouched. `AIContextMenuState` shrinks to the
three genuinely GUI-only fields (scroll, uniform-list, hover states). `universal_developer_input.rs`
builds a throwaway core state for its "any categories available?" check, replacing a static call to
the removed `get_categories_for_mode`.

### Why this shape

The slash-command menu already solved this split, and the layering is copied from it:

- Generic menu primitives — `InlineMenuSelection`, `InputDrivenInlineMenuLifecycle` in
  `warp_search_core` — are plain structs, already shared.
- The shared domain state —
  [`SlashCommandDataSourceState` @ 02fe6e4](https://github.com/warpdotdev/warp/blob/02fe6e4e4d08f96fad8faa0a7e85d8df824c15ea/app/src/terminal/input/slash_commands/data_source/core.rs#L169-L190) —
  is a plain struct, not an entity.
- The per-surface wrappers (`GuiSlashCommandDataSource`, `TuiSlashCommandDataSource`,
  `SlashCommandModel`) are the models.
- There are two menu models, not one: `slash_command_model.rs` for the GUI and
  `crates/warp_tui/src/slash_commands.rs` for the TUI.

Note the placement convention: even the TUI's slash-command *data source* lives in `app/`, because
it needs app-crate internals. Only the menu model lives in `crates/warp_tui`. The `@` menu follows
that.

Result selection deliberately stays per-surface. The GUI's lives in `SearchBarState`, which holds
`Vec<QueryResultRenderer>` — GUI renderers with styles and position ids — so it is not shareable;
the TUI will use `TuiInlineMenuListState`.

### Considered alternatives

**Shared helpers only** — move the types plus free functions for category availability, source
wiring, and the `@` grammar, and let each front-end own its own navigation state machine. Smaller
diff, but the ~120-line state machine (main-menu/category/all-categories transitions, the
fall-through, the empty-results counter) would exist twice and drift. Rejected.

**One fully shared menu model** — a single entity owning mixer, navigation, query, and selection,
with both front-ends rendering a snapshot. The purest split, but it requires migrating the GUI off
`SearchBar` and re-plumbing `UniformList`, hover states, the details panel, and scroll-into-view
inside a 1711-line view. That is regression risk on a shipped surface for the benefit of an
unshipped one. Rejected.

**Trait with default methods over `core()` / `core_mut()`**, matching `SlashCommandDataSource`
exactly. That trait earns its keep because its default methods call back into surface-specific
policy (`self.availability(ctx)`, `self.command_passes_common_gates(...)`). Nothing in the `@` state
machine needs a surface hook — the gates are plain data — so the trait would be pure indirection
over `self.core()`. Rejected in favor of inherent methods on the state struct.

**`AtContextMenuCoreState` as a warpui model.** A model earns its keep when state changes from
sources its owner cannot see; that is exactly why `TuiSlashCommandDataSource` is one, with seven
external subscriptions feeding `UpdatedActiveCommands`. Every mutation of this state happens because
its owner called a method on it, so there is no event to emit that the caller is not already
positioned to emit. It would also cost an invented `Event` type, `as_ref` / `update` ceremony at
every access, an extra hop in the notify path, and an `App` fixture for tests that currently need
none. Rejected.

## Testing and validation

There are no behavior invariants to map, since the change is behavior-preserving. Correctness rests
on the diff being mechanical, plus a small test set over the extracted surface:

`app/src/search/ai_context_menu/core_tests.rs` — 8 unit tests, one per distinct behavior:

- the `@` trigger grammar and the close conditions, each as a table (start of buffer, after a space,
  after punctuation, suppressed mid-word; cursor behind the `@`, a newline, two consecutive spaces,
  a single space allowed)
- the text every action variant inserts, including the two the core declines
- the no-progress counter: closing after enough settled empty results, the category list being
  exempt because it filters locally, and loading or a rewritten query not counting
- category availability for the one branch no feature flag varies, the fall-through to
  all-categories, and category selection wrap-around

Five of the eight need no `App` fixture. Deliberately kept small: this is a no-op refactor, and the
code had no tests before.

Also required: `./script/format`, `cargo clippy -p warp --all-targets --tests -- -D warnings`, and
`cargo clippy --workspace --exclude warp_completer --all-targets --tests -- -D warnings` (the pass
that unifies `warp/tui`).

Manual validation, not yet done: open the `@` menu in `./script/run`, filter categories, enter one,
accept an item, and confirm the fall-through search when the filter matches no category name.

## Risks and mitigations

**Working-directory caching is the one non-mechanical change.** `available_categories` reads a
working directory cached on the core state, where `get_categories_for_mode` re-read
`app.windows().state().active_window` on every call. The cache is refreshed from the three
notifications the view already subscribed to in order to redraw — `DetectedRepositories`,
`WindowManager`, and the `ActiveSession` observe — plus on any gate change, and `set_session_state`
notifies whenever the working directory changes. Equivalent by argument rather than by
construction, so it is the part most worth reviewing closely.

**Two existing quirks are preserved on purpose** so the refactor stays a no-op, both worth fixing
separately:

1. The all-categories search installs no current-folder file source, so falling through outside a
   git repository returns no file results. Unifying on one `install_sources_for_category` would have
   silently fixed this, so the GUI call site filters `CurrentFolderFiles` out of the list it passes.
   The shared function handles the category correctly; the fix is deleting the call-site filter.
2. `is_at_menu_trigger` indexes with `chars().nth()` against what callers compute as a byte offset,
   so it misbehaves for non-ASCII text before the `@`.

**WASM cfg arms are unverified locally.** Presubmit has no WASM build step and does not run
`--all-features` on `warp`, and this touches cfg-gated code in all three files.

## Follow-ups

The rest of the stack, in order:

1. `harry/code-1910-explicit-working-dir` — give `FileSearchModel` and the file/skills data sources
   an explicit working directory instead of reading `active_window`. The TUI has no active window
   (`add_tui_window` never opens a platform window, so `set_active_window` is never reached) and
   hosts many sessions in one window, so window-keyed state cannot express a per-session working
   directory. `CodeSymbolCache` is out of scope, since Code symbols are excluded below and the GUI is
   its only consumer — which also avoids the one awkward part of this change, because
   `ensure_symbols_cached` runs inside a `spawner.spawn(move |cache, ctx| ...)` closure with no call
   site to thread a parameter through and would have needed stored state instead.
2. `harry/code-1910-context-menu` — the TUI menu model, in
   `crates/warp_tui/src/at_context_menu.rs`. Scope is category parity with the GUI minus what is
   technically blocked: files and folders, Drive objects, plans, rules, conversations, diff sets,
   and skills. Skills stay in despite the TUI having a `/` skills menu, because the GUI lists
   Skills in `@` alongside its own skills menu, and Conversations alongside its conversations menu.
   Diff sets are the one category that is not a buffer rewrite: `InsertDiffSet` inserts no text, and
   the GUI routes it through `Event::AttachDiffSetContext` into
   `TerminalView::handle_attach_diffset_context`, which inserts a `<change:…>` reference and then
   completes the attachment asynchronously via `LocalDiffStateModel::load_diff_data_for_mode`. The
   TUI needs its own handler; every input it requires already exists there (`current_repo_path`,
   `git_repo_status` branch metadata, the shared `create_attachment_reference_and_key`, the
   span-replacement path, and the same context model).

Excluded from the stack, both for technical reasons rather than product ones:

- **Code symbols.** `LaunchMode::Tui` reports
  [`supports_indexing() == false` @ 02fe6e4](https://github.com/warpdotdev/warp/blob/02fe6e4e4d08f96fad8faa0a7e85d8df824c15ea/app/src/lib.rs#L566-L583),
  so `RepoOutlines` is built with `new_with_indexing_enabled(false)`, never subscribes to
  `DetectedRepositories`, and `should_build_outlines` is permanently false. There are no outlines
  for the TUI to search regardless of working-directory plumbing. Enabling them is not a flag flip:
  `RepoOutlines` also feeds `BlocklistAIContextModel::pending_context` and
  `GetRelevantFilesController`, so turning it on changes what context the agent receives in the TUI.
- **Blocks.** The data source and `find_block_attachment_in_all_terminals` both iterate
  `views_of_type::<TerminalView>`, a GUI-only view type, so `<block:…>` would not resolve in the TUI
  even if the picker listed rows.

## Parallelization

Not proposed. The three branches are strictly sequential — the TUI menu needs the working-directory
seam, which needs the extracted core — and within each branch the work is a single-crate refactor
where the compiler is the feedback loop. Parallel agents on one `app/` checkout would serialize on
the cargo build lock and collide in `view.rs` and `mixer.rs`, so fan-out would cost wall-clock time
rather than save it.
