---
name: gui-settings-ui
description: GUI desktop app only. How to build a Settings page in the Warp client (app/src/settings_view) so its widgets and settings search behave correctly — picking a PageType, deciding whether a heading belongs in the page-title slot or inside a widget, scoping search_terms per widget, and spacing. Use when adding or editing a settings page, a SettingsWidget, or anything that affects settings search.
---

# gui-settings-ui

**Scope — GUI desktop app only.** This skill covers the settings modal in `app/src/settings_view/`, part of Warp's **GUI** desktop front-end. It does not apply to the headless TUI (`crates/warp_tui`). For general UI conventions see `gui-ui-guidelines`.

Settings pages look simple, so they get written by pattern-matching the nearest neighbor — and the nearest neighbor is often wrong. The same two mistakes have produced five Linear tickets (APP-5060, APP-5058, APP-4910, APP-4922, APP-5059, one of which turned out to be a false positive). Read this before writing a settings page so the sixth doesn't happen.

## The model

A settings page is a `PageType` (`app/src/settings_view/settings_page.rs`). It holds **a list of searchable widgets** plus **an optional page title**:

```rust
PageType::new_uncategorized(widgets, Some("Knowledge"))
PageType::new_categorized(categories, None)
PageType::new_monolith(widget, Some("Billing and Usage"), /* is_dual_scrollable */ true)
```

- **`Uncategorized`** — a flat list of widgets. The common shape.
- **`Categorized`** — widgets grouped under `Category`s, each with a subheader (rendered via `render_sub_header`) and an optional subtitle.
- **`Monolith`** — the whole page is a single widget because its content can't be split for search (Keybindings, Teams, About, Environments).

Each widget implements `SettingsWidget` (`settings_page.rs`):

- `search_terms(&self) -> &str` — the terms this widget matches on.
- `render(..)` — the widget's rows.
- `should_render(&self, app) -> bool` — defaults to `true`; used for feature-flag / auth / platform gating.
- `widget_id()` / `static_widget_id()` — default to `std::any::type_name::<Self>()`, used for scroll-to and deeplinks.

**How search filters.** `PageType::update_filter` keeps a widget when
`widget.should_render(app) && search_terms_match(widget.search_terms(), query)`.
`search_terms_match` requires **every whitespace-delimited word** of the query to appear (case-insensitively, as a substring) somewhere in the widget's terms; an empty query matches everything. So filtering happens **per widget** — a widget is the smallest unit that can survive or disappear.

**The page title is exempt from filtering.** `PageType::render_page` draws the title **once, before the loop** over the filtered widget list:

```rust
let mut page = Flex::column();
if let Some(title) = title {
    page.add_child(render_page_title(title, HEADER_FONT_SIZE, appearance));
}
for widget in widgets { /* … only the widgets that matched … */ }
```

That structural fact is the whole point of the title slot: a title passed through `PageType` **cannot** be filtered away; a title rendered inside a widget **can**.

## The decision that matters: where does the heading live?

For any heading on a settings page, ask **what does it name?**

- **It names the whole page** → it belongs in the `PageType` title slot.
- **It names one section among several** → it belongs inside the widget (or `Category`) that owns that section, so it disappears together with its own rows.

Getting this wrong in the first direction is bug class 1 below. Getting it wrong in the second direction leaves a section header stranded above unrelated rows.

### Page-shape taxonomy

Classify the page before you write it:

- **Single-topic page** — one heading names everything on it. Knowledge, Third party CLI agents, Editor and Code Review, Codebase Indexing, Account, Scripting. → **Title in the `PageType` slot.**
- **Multi-section page** — several independent sections, each with its own heading. Warp Agent, Agent profiles, Appearance, Features. → **Per-section headings live in widgets/categories** and correctly disappear with their rows. (For `Categorized`, `get_filtered` drops categories whose widgets all filtered out, so their subheaders vanish automatically — that's the behavior you want.)
- **Monolith page** — Keybindings, Teams, About, Environments. The page filters all-or-nothing, so an in-widget title can never be stranded. Not this bug class. Using the title slot is still tidier (Billing and Usage and Referrals do).

The two are not mutually exclusive: a page can name itself in the title slot **and** have per-section subheaders inside its widgets. Privacy does exactly that — `PageType::new_uncategorized(widgets, Some("Privacy"))` plus `render_sub_header` calls inside individual widgets. The rule is per heading, not per page.

Worked positive example: **Scripting** (`scripting_page.rs`) is a small page done right — two focused widgets (`WarpControlCliInstallWidget`, `LocalControlModeWidget`) with their own `search_terms`, and `PageType::new_uncategorized(widgets, Some("Scripting"))`. A ticket was once filed claiming Scripting needed splitting; it was canceled because the premise was wrong. A small page is not automatically a mega-widget.

## Bug class 1 — the vanishing title

**Symptom:** on a single-topic page, typing a search term that matches one row makes the page heading disappear along with the non-matching rows, leaving an unlabeled orphan setting.

**Cause:** the only heading was rendered inside a widget — via `build_sub_header` / `render_page_title` in that widget's `render`, or via a header-only widget that exists just to draw a title. Filtering removes the widget, and the heading goes with it.

Canonical fix — commit `ddadcee` ([APP-5060], #14519), Knowledge. Before, `AIFactWidget::render` opened with:

```rust
let header = build_sub_header(appearance, "Knowledge", …).finish();
let mut column = Flex::column().with_child(header) /* … all the Knowledge rows … */;
```

After, the heading moved to page chrome and the rows became focused widgets:

```rust
let title = match subpage {
    Some(AISubpage::Knowledge) => Some("Knowledge"),
    Some(AISubpage::ThirdPartyCLIAgents) => Some("Third party CLI agents"),
    None | Some(AISubpage::WarpAgent) | Some(AISubpage::Profiles) => None,
};
PageType::new_uncategorized(widgets, title)
```

(The `ThirdPartyCLIAgents` arm and the exhaustive `match` came from #14524; note the deliberate absence of a `_` arm, so a new subpage forces this decision.)

The same commit (#14524) deleted `CodeSubpageHeaderWidget` from `code_page.rs` — a widget whose entire job was `build_sub_header(appearance, self.title, None)` — and replaced it with `PageType::new_uncategorized(widgets, Some(subpage.title()))`. **A header-only widget is always this bug.** If a widget renders nothing but a title, delete it and use the title slot.

## Bug class 2 — the unfilterable mega-widget

**Symptom:** searching a term that should isolate one row shows every row on the page, and the sidebar match count is 1 no matter how specific the query is.

**Cause:** one widget renders many unrelated settings and declares one mega `search_terms()` blob covering all of them. The widget is the filter unit, so it's all-or-nothing.

Before (`CLIAgentWidget`, pre-#14524) — one widget, one blob, seven settings:

```rust
fn search_terms(&self) -> &str {
    "third party cli coding agent claude codex gemini toolbar footer layout chip chips \
     rearrange re-arrange bar command regex auto show rich input dismiss ctrl enter submit newline"
}
```

After — one widget per setting, each with terms scoped to just that setting:

```rust
fn cli_agent_widgets() -> Vec<Box<dyn SettingsWidget<View = AISettingsPageView>>> {
    vec![
        Box::new(CLIAgentWidget::default()),
        Box::new(CLIAgentAutoToggleRichInputWidget::default()),
        Box::new(CLIAgentAutoOpenRichInputWidget::default()),
        Box::new(CLIAgentAutoDismissRichInputWidget::default()),
        Box::new(CLIAgentSubmitRichInputWidget::default()),
        Box::new(CLIAgentCommandsWidget),
        Box::new(CLIAgentToolbarLayoutWidget),
    ]
}
```

Rules of thumb when splitting:

- **One widget ≈ one setting row** (or one tightly-coupled group that always shows and hides together).
- **Scope `search_terms` to that widget only.** Keep enough shared context that a page-level query still matches (each CLI-agent widget above keeps `"third party cli coding agent"`), then add the terms unique to the row.
- **Move conditional rendering into `should_render`.** A row that used to be behind an `if is_footer_enabled { … }` inside a mega-render becomes its own widget with `fn should_render(&self, app) -> bool`, so search and rendering agree. Share the predicate through a small free function (`should_render_cli_agent_rich_input(app)`) rather than duplicating the condition.
- **Move per-row state with the row.** Each `SwitchStateHandle` / `MouseStateHandle` moves to the widget that owns its control. Never create one inline while rendering (see `gui-ui-guidelines` and the AGENTS.md note on `MouseStateHandle`).
- **Watch the widget ids.** `widget_id()` is `std::any::type_name::<Self>()`, so splitting a widget changes ids. `settings_widget_deeplink_target` in `app/src/settings_view/mod.rs` maps stable public slugs (`warp://settings?widget=<slug>`) onto them. The CLI-agent split deliberately kept `CLIAgentWidget` as the first widget so `cli_agent_settings_widget_id()` — the target of the `cli_agents` deeplink — stayed valid. If you rename or remove a widget that backs a deeplink, re-point the accessor.

## Spacing: `PAGE_TITLE_MARGIN_BOTTOM` is the only title gap

`render_page_title` already applies `PAGE_TITLE_MARGIN_BOTTOM` (`settings_page.rs`), and `render_page` already wraps the whole page in uniform `PAGE_PADDING`. **A page must not add its own top padding/margin on top of that** — you get a double gap, and every page that tries invents a different constant.

#14524 removed five such offenders, each with a different number:

- Account — `Container::new(account_info).with_margin_top(VERTICAL_MARGIN)` (24px), `main_page.rs`.
- Billing and Usage — `.with_margin_top(HEADER_PADDING)` (15px), `billing_and_usage_dispatch.rs`.
- Referrals — `.with_padding_top(PAGE_PADDING)` (28px), `referrals_page.rs`.
- Shared blocks — a hardcoded `.with_margin_bottom(24.)` under the page title, `show_blocks_view.rs`.
- Privacy — `.with_padding_top(PAGE_PADDING)` (28px), `privacy_page.rs`.

If the gap looks wrong, change `PAGE_TITLE_MARGIN_BOTTOM` (which is deliberately defined as `HEADER_PADDING`, so title spacing tracks section spacing) — don't patch it per page.

## Subpages rebuild their `PageType` — reapply the filter

AI and Code subpages rebuild their `PageType` when the active subpage changes (`AISettingsPageView::build_page`, `CodeSettingsPageView::set_active_subpage`). A fresh `PageType` starts with **every** widget in its filter, so a live search query is silently dropped unless it's reapplied. `SettingsView::reapply_search_filter_to_active_subpage` in `app/src/settings_view/mod.rs` exists for exactly this ([APP-4922], #14116). If you add a code path that rebuilds a subpage's page while search may be active, call it.

## Known anti-examples still in the tree

Useful to read, not to copy:

- **Warpify** (`warpify_page.rs`) — `PageType::new_categorized(categories, None)` where the first category is `Category::new("", vec![Box::new(TitleWidget::default())])` and `TitleWidget::render` calls `render_page_title("Warpify", …)`. Single-topic page, title inside a widget: bug class 1.
- **Warp Drive** (`warp_drive_page.rs`) — `PageType::new_uncategorized([WarpDriveHeaderWidget, WarpDriveToggleWidget], None)` with no title slot at all. `WarpDriveHeaderWidget` is a conditional sign-up banner, so a signed-in user sees only a bare toggle row and no page heading.

## How to verify a settings-page change

Verification is cheap here; do both.

1. **Unit-test the filter.** `app/src/settings_view/mod_tests.rs` has the harness — `StubWidget`, `stub_widgets_page`, and `visible_widget_count` (which reads `PageType::get_filtered`). Assert that a term unique to one widget yields exactly one visible widget, and that clearing the query restores all of them. Follow the `${filename}_tests.rs` convention from AGENTS.md; run with `cargo nextest run -p warp settings_view`.
2. **Exercise the real search.** Open Settings, go to the page, and check three things:
   - a term matching exactly one row leaves **only** that row,
   - the **page title is still visible** while that filter is active,
   - clearing the search restores the full page, with the title spacing unchanged.

## Anti-patterns

```rust
// A widget whose whole job is to draw the page heading. Delete it and pass the
// title through PageType instead.
struct FooSubpageHeaderWidget { title: &'static str }

// A single-topic page with no page title, drawing its heading inside a widget.
// Any non-matching search term erases the heading.
PageType::new_uncategorized(widgets, None)   // and build_sub_header("Foo", …) in a widget

// One widget, one mega search_terms blob, many unrelated rows: nothing can be
// filtered or attributed individually.
fn search_terms(&self) -> &str { "foo bar baz qux quux corge grault" }

// Conditional rows hidden inside render() while search_terms still advertises
// them — the page matches a query and then renders nothing for it.
fn render(&self, …) { if flag_enabled { /* the only rows */ } }
// Use a separate widget with should_render() instead.

// Per-page top spacing stacked on top of PAGE_TITLE_MARGIN_BOTTOM + PAGE_PADDING.
Container::new(content).with_margin_top(24.).finish()

// A non-exhaustive match over subpages when deciding the title — a new subpage
// silently inherits `None` instead of forcing the single-topic/multi-section call.
let title = match subpage { Some(Subpage::Knowledge) => Some("Knowledge"), _ => None };
```
