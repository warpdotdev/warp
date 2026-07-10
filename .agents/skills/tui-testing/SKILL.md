---
name: tui-testing
description: Write and run unit tests for Warp TUI (crates/warp_tui) elements/screens by rendering to text lines. Use for TUI test work.
---

# tui-testing

How to write and run unit tests for Warp's **headless TUI** front-end (`crates/warp_tui` and the element library at `crates/warpui_core/src/elements/tui`). This complements `rust-unit-tests` (general Rust test conventions) and parallels `gui-integration-test` for the TUI.

TUI tests are plain, fast unit tests: they render an element tree to a fixed cell grid and assert on the resulting text lines. They do **not** use the GUI real-display / integration / computer-use framework (that's `gui-integration-test` / `gui-integration-test-video`, which are GUI-only).

## Two test locations, two harnesses

TUI tests live in two crates, and which render helper you use depends on where the test is:

### Element-library tests (in `warpui_core`)

Tests for the shared cell-grid elements live in `crates/warpui_core/src/elements/tui/*_tests.rs` and use the crate-internal `test_support` helpers from `crates/warpui_core/src/elements/tui/mod.rs`:

- `test_support::render_to_lines(element: &dyn TuiElement, size: TuiSize) -> Vec<String>` — one-call harness: builds `area = TuiRect::new(0, 0, size.width, size.height)` and `TuiBuffer::empty(area)`, calls `element.render(area, &mut buffer, ctx)` inside a paint context, and returns `buffer.to_lines()`. **It only calls `render`, not `layout`** — fine for a leaf like `TuiText`, but composite elements (e.g. `TuiFlex`) populate child sizes during `layout`, so lay the element out first (see the `layout_at` helper in `flex_tests.rs`) or it renders empty/stale.
- `test_support::with_paint_context(|ctx| ...)` — runs a closure with a `TuiPaintContext` over a fresh, empty view map. Use it when you need the `TuiBuffer` afterward to assert on individual `Cell`s.

These helpers are `pub(crate)` to `warpui_core`, so they are only callable from that crate's own tests. Simplest leaf assertion (see `text_tests.rs`, `flex_tests.rs`):

```rust path=null start=null
assert_eq!(
    render_to_lines(&TuiText::new("hello"), TuiSize::new(10, 1)),
    vec!["hello     "],
);
```

### View/screen tests (in `warp_tui`)

`warp_tui` tests (`crates/warp_tui/src/*_tests.rs`) can NOT use `test_support` — render directly instead, under an `App::test` read/update so an `AppContext` is available. `layout` must run before `render` so child sizes are populated. This local helper mirrors `render_element` in `transcript_view_tests.rs` and `render_lines` in `editor_element_tests.rs`:

```rust path=null start=null
fn render_lines(app_ctx: &AppContext, mut element: impl TuiElement, w: u16, h: u16) -> Vec<String> {
    let mut rendered_views = EntityIdMap::default();
    let mut lctx = TuiLayoutContext { rendered_views: &mut rendered_views };
    let size = element.layout(TuiConstraint::loose(TuiSize::new(w, h)), &mut lctx, app_ctx);
    let area = TuiRect::new(0, 0, size.width, size.height);
    let mut buffer = TuiBuffer::empty(area);
    let mut paint_ctx = TuiPaintContext::new(&mut rendered_views);
    element.render(area, &mut buffer, &mut paint_ctx);
    buffer.to_lines()
}
```

Views that resolve theme styles need an `Appearance` singleton (`ctx.add_singleton_model(|_| Appearance::mock())`). To exercise a whole view through the real draw path, drive the presenter: `TuiPresenter::new()`, `presenter.invalidate(&invalidation, ctx, window_id)`, then `presenter.present(ctx, &view, area)` and assert on `frame.buffer.to_lines()` (see `transcript_view_tests.rs`).

## Asserting on styles, cursor, and events

- **Styles/colors:** paint into a buffer yourself and index cells. `to_lines()` only carries glyphs, so style assertions read `Cell` fields:

```rust path=null start=null
let mut buffer = TuiBuffer::empty(TuiRect::new(0, 0, 1, 1));
with_paint_context(|ctx| text.render(TuiRect::new(0, 0, 1, 1), &mut buffer, ctx));
let cell = &buffer[(0, 0)];
assert_eq!(cell.symbol(), "a");
assert_eq!(cell.fg, Color::Red);
assert!(cell.modifier.contains(Modifier::BOLD));
```

(`with_paint_context` is the `warpui_core`-only helper; in a `warp_tui` test construct the paint context directly with `TuiPaintContext::new(&mut rendered_views)` as in the `render_lines` helper above.)

- **Cursor:** call `element.cursor_position(area, ctx)` and assert on the returned `Option<(u16, u16)>`.
- **Events:** build a `TuiEvent` (e.g. `TuiEvent::KeyDown { keystroke, chars, details, is_composing }` or `TuiEvent::ScrollWheel { .. }`), then call `element.dispatch_event(&event, area, &mut event_ctx, &mut layout_ctx, app_ctx)` and assert on the returned `bool` and on the re-rendered lines/cursor. Layout must run first. See `render_element` / `dispatch_event` / `dispatch_scroll` helpers in `crates/warp_tui/src/transcript_view_tests.rs`.

Keep test areas at a stable, small width/height so golden line vectors stay readable and deterministic; trailing padding is spaces (e.g. `"hello     "`).

## Where tests live

Follow the repo convention: put tests in a sibling `*_tests.rs` file included at the end of the source module:

```rust path=null start=null
#[cfg(test)]
#[path = "foo_tests.rs"]
mod tests;
```

Real examples to model:
- Element library: `crates/warpui_core/src/elements/tui/text_tests.rs`, `flex_tests.rs`, `container_tests.rs`, `constrained_box_tests.rs`, `buffer_tests.rs`.
- Views/screens: `crates/warp_tui/src/transcript_view_tests.rs`, `crates/warp_tui/src/input/view_tests.rs`.

## `Appearance` in view tests

Views that resolve theme styles (via `TuiUiBuilder::from_app`) need an `Appearance` singleton. Install the mock in the test with `app.add_singleton_model(|_| Appearance::mock());` (as in `transcript_view_tests.rs`). `Appearance::mock()` comes from `warp_core`'s `test-util` feature, wired as a dev-dependency of the TUI crates.

## Process-level tests (no integration harness)

The TUI has **no** GUI-style integration harness: the real-display, synthetic-event framework in `crates/integration` (see `gui-integration-test`) is GUI-only and does not drive the TUI. Besides render-to-lines unit tests, binary-level behavior is covered by a process-level test that spawns the built binary and asserts on its output/exit — see `crates/warp_tui/tests/worker_dispatch.rs` (it runs `CARGO_BIN_EXE_warp-tui-oss` and checks that a worker invocation dispatches without launching the TUI frontend). Use that pattern for process/CLI-level behavior, and render-to-lines unit tests for element/screen rendering. There is no separate TUI integration-test skill because there is no such framework today.

## Running

- Whole crates: `cargo nextest run -p warp_tui` and `cargo nextest run -p warpui_core`.
- The TUI element library is behind the `tui` feature; if a test needs it explicitly, add `--features tui`.
- A single test by substring: `cargo nextest run -p warp_tui -E 'test(<substring>)'`.
- Before opening a PR, run `./script/format` and `cargo clippy` per repo conventions.
