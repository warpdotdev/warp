---
name: tui-ui-guidelines
description: Guidelines for writing Warp headless TUI (crates/warp_tui) UI code with the cell-grid TuiElement library. Read before any TUI UI work.
---

# tui-ui-guidelines

Guidelines for writing UI code in Warp's **headless TUI** front-end. This is the TUI counterpart to `gui-ui-guidelines` (which covers the pixel-based GUI desktop app). Read this once at the start of any TUI UI task, then keep it in mind while implementing.

The TUI is a distinct front-end from the GUI desktop app. Do **not** carry over GUI assumptions (pixels, mouse-pixel hit-testing, GPU/WGSL, `.app` bundles, design-system button *pixel* themes, launch modals). If a GUI guideline is about pixel layout or GPU rendering, it does not apply here.

## Where TUI UI code lives

- **Front-end views/screens:** `crates/warp_tui` — per-channel console binaries (e.g. `crates/warp_tui/src/bin`). Run/observe the TUI with `./script/run-tui`. There is no `.app` bundle, no GPU/WGSL, and no mouse-pixel model.
- **Element library:** `crates/warpui_core/src/elements/tui`, behind the `tui` cargo feature. This is a *parallel* cell-grid element vocabulary, separate from the GUI `Element`/`View` library.

**Shared with the GUI** (do reuse): the Entity/model core in `warp_core`/`warpui` — `App`/`Entity`/`AppContext`/`ViewContext`, the actions system, `Appearance`/theming, `FeatureFlag` runtime checks (`FeatureFlag::X.is_enabled()` works in both front-ends), telemetry, and logging.

**Different from the GUI** (do NOT use here): the GUI `Element`/`View` types, pixel geometry, and GPU/WGSL rendering or pixel-drawn button themes. The TUI has its own `crates/warp_tui/Cargo.toml`; the compile-time Cargo-feature bridge in `app/Cargo.toml` + `app/src/lib.rs enabled_features()` is GUI-app-specific and does not gate TUI code. (The TUI *does* have hover/click: `TuiHoverable` and `tui_collapsible` reuse the shared `MouseStateHandle`, so own that handle outside render just like the GUI — only pixel-based hit-testing is GUI-only.)

## The `TuiElement` trait

Defined in `crates/warpui_core/src/elements/tui/mod.rs`. An element measures itself, then paints into a sub-rectangle of a cell buffer:

- `layout(&mut self, constraint: TuiConstraint, ctx: &mut TuiLayoutContext, app: &AppContext) -> TuiSize` — measure against a constraint and return a size within it. `app` gives shared read access to the core (mirrors the GUI's `Element::layout`).
- `render(&self, area: TuiRect, buffer: &mut TuiBuffer, ctx: &mut TuiPaintContext)` — paint into `area` of `buffer`. `area`'s size is what `layout` returned, clamped to what was available.
- `cursor_position(&self, area, ctx) -> Option<(u16, u16)>` — where the terminal cursor should sit within `area`, if this element owns it (default: `None`).
- `present(&mut self, ctx)` — participate in the child-view recursion so the presenter records parent/child view relationships (default: nothing; only container/child-view elements override it).
- `dispatch_event(&mut self, event, area, event_ctx, ctx, app) -> bool` — offer an event to this element, returning whether it was handled (default: `false`).
- `.finish()` — boxing convenience that returns `Box<dyn TuiElement>`, mirroring the GUI `Element::finish`. **Always terminate an element with `.finish()`; never hand-wrap an element in `Box::new`.** It's what the child-taking APIs (`TuiFlex::child`/`with_child`, `TuiChildView`, etc.) expect, and it keeps element trees consistent and readable.

## Composition vocabulary

Re-exported from `crates/warpui_core/src/elements/tui/mod.rs`:

- **Layout containers:** `TuiFlex` (`TuiFlex::row()` / `TuiFlex::column()`, with `.child(...)`, `.flex_child(...)`, `.with_cross_axis_alignment(...)`), `TuiContainer`, and `TuiConstrainedBox` (e.g. `.with_max_cols(N)`).
- **Content:** `TuiText` (`.with_style(style)`, `.truncate()`, `TuiText::from_spans([...])`).
- **View/embedding:** `TuiChildView` for embedding another view's rendered element; `TuiEventHandler` (e.g. `.on_key("x", |_, _, _| ...)`) to attach handlers to a subtree.
- **Multi-child trait:** `TuiParentElement` provides `with_child` / `with_children` / `add_child` / `add_children`.
- **Geometry (integer cells):** `TuiSize`, `TuiRect`, `TuiConstraint` (`TuiConstraint::loose(size)` / `TuiConstraint::tight(size)`; `TuiConstraint::clamp`). Also `TuiPoint`.

## Styling

Styles are `TuiStyle` values (`Color`, `Modifier` — e.g. `Modifier::BOLD`, `Modifier::DIM`) painted into a `TuiBuffer` of `Cell`s. Terminal cells have no alpha, so styles are solid.

**Prefer the semantic style helpers on `TuiUiBuilder`** (`crates/warp_tui/src/tui_builder.rs`) over hardcoding colors — this mirrors the GUI guideline about reusing themes. Construct it per render with `TuiUiBuilder::from_app(app)`, then ask for semantic styles: `primary_text_style()`, `muted_text_style()`, `dim_text_style()`, `error_text_style()`, `success_glyph_style()`, `accent_border_style()`, `input_text_style()`, etc. The builder owns the theme→style recipes so views ask for "primary text" / "muted text" instead of deriving colors from the theme by hand. Do not reach for raw ANSI slots (e.g. `Color::White`) directly — those are tuned for dark backgrounds and wash out on light themes.

## Events and keybindings

Crossterm input is converted (in `crate::runtime`) to `TuiEvent` and dispatched through the element tree via `dispatch_event`; text-cursor placement flows through `cursor_position`.

Keybindings follow the GUI convention: each TUI view module exposes a top-level `init(app)` that registers its bindings, aggregated in `crates/warp_tui/src/keybindings.rs` and called once at TUI startup. Fixed/reserved bindings (e.g. ctrl-c) are tagged with the `tui` group (`TUI_BINDING_GROUP`); editable, user-remappable bindings are named with a `tui:` prefix. GUI bindings never fire in the TUI — predicate-scoped bindings never match TUI keymap contexts, and predicate-less ones dispatch action types no TUI view handles — and debug-time validators (`register_binding_validators`) enforce that any keystroke binding matching a TUI view's context is TUI-owned.

## Example: composing a small element tree

A `TuiFlex::column()` of styled `TuiText` children, wrapped in a width cap (illustrative):

```rust path=null start=null
let builder = TuiUiBuilder::from_app(app);
let title_style = builder.accent_border_style().add_modifier(Modifier::BOLD);
let muted = builder.muted_text_style();

let column = TuiFlex::column()
    .child(
        TuiText::new("Warp Agent")
            .with_style(title_style)
            .truncate()
            .finish(),
    )
    .child(TuiText::new(version).with_style(muted).truncate().finish());

TuiConstrainedBox::new(column.finish())
    .with_max_cols(48)
    .finish()
```

Verify API names against the element library (`crates/warpui_core/src/elements/tui/mod.rs`) and `TuiUiBuilder` (`crates/warp_tui/src/tui_builder.rs`); don't invent methods. Don't treat existing `crates/warp_tui` view code as canonical examples — much of it is early prototyping and isn't the pattern to copy going forward.

## Reference

- Run/observe the TUI with `./script/run-tui`.
- Verify a TUI change by building and running it (`./script/run-tui`) and observing the output in an interactive terminal; the `tui-verify-change` skill covers this end to end.
- Write and run TUI tests with the `tui-testing` skill.
