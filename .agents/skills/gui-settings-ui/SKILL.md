---
name: gui-settings-ui
description: GUI desktop app only. How to build a Settings page in the Warp client (app/src/settings_view) so its widgets and settings search behave correctly — picking a PageType, deciding whether a heading belongs in the page-title slot or inside a widget, gating a widget, and scoping search_terms per widget. Use when adding or editing a settings page, a SettingsWidget, or anything that affects settings search.
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
- `should_render(&self, app) -> bool` — defaults to `true`. This is one of **two** ways to conditionally show a setting, and usually not the right one — see "Two ways to gate a widget" below.
- `widget_id()` / `static_widget_id()` — default to `std::any::type_name::<Self>()`, used for scroll-to and deeplinks.

**How search filters.** `PageType::update_filter` keeps a widget when
`widget.should_render(app) && search_terms_match(widget.search_terms(), query)`.
`search_terms_match` requires **every whitespace-delimited word** of the query to appear (case-insensitively, as a substring) somewhere in the widget's terms; an empty query matches everything. So filtering happens **per widget** — a widget is the smallest unit that can survive or disappear.

**Whether the page title survives a search depends on the variant.** `PageType::render_page` treats the title differently for widget-list pages than for a monolith, and that difference is the easiest thing here to get wrong.

For **`Uncategorized` and `Categorized`**, the title is drawn **once, before the loop** over the filtered widget list, unconditionally:

```rust
let mut page = Flex::column();
if let Some(title) = title {
    page.add_child(render_page_title(title, HEADER_FONT_SIZE, appearance));
}
for widget in widgets { /* … only the widgets that matched … */ }
```

The filter cannot reach it. That is what makes the title slot a fix for bug class 1: a title passed through `PageType` survives filtering on these two variants, while a title rendered inside a widget does not.

For **`Monolith`**, the title is **gated on the sole widget surviving the filter**:

```rust
FilteredPageType::Monolith { widget, title, .. } => {
    let mut page = Empty::new().finish();
    if let Some(widget) = widget          // None when the widget didn't match
        && widget.should_render(app)
    {
        if let Some(title) = title { /* … title, then the widget … */ }
    }
    page                                  // otherwise nothing renders at all
}
```

`get_filtered` sets that `widget` to `filter.then_some(..)`, so a non-matching query makes it `None` and the page renders empty — **title included**. On a monolith the title slot buys no search protection; the page is all-or-nothing, title and all.

One more thing worth knowing before you go looking for a title stranded over zero rows: you won't find one. `SettingsView::filtered_pages` (`app/src/settings_view/mod.rs`) drops any page whose `MatchData` is falsy from the sidebar and auto-selects the first page that still matches, so a fully non-matching page is never displayed. The state the title slot actually protects is the **partial** match — some widgets survive, some don't — and only widget-list pages can be in it.

## Two ways to gate a widget

There are two mechanisms for conditionally showing a setting, and they are not interchangeable.

**1. An `if` at page-build time — never create the widget.** This is how most gating in `AISettingsPageView::build_page` (`ai_page.rs`) is written:

```rust
if FeatureFlag::AIRules.is_enabled() {
    widgets.extend(Self::knowledge_widgets());
}
if cfg!(feature = "voice_input")
    && ai_settings.voice_input_enabled_internal.is_supported_on_current_platform()
{
    widgets.push(Box::new(VoiceWidget::default()));
}
```

**2. `should_render(&self, app) -> bool` — create the widget and let it opt out per pass.**

**Which one: can the value change while the app is running?**

- **Fixed for the process** — a feature flag, a `cfg!` feature, a platform-support check. → **Use the build-time `if`.** This is the preferred default: the widget never exists, so there is nothing to filter, render, or reason about.
- **Can change at runtime** — a setting the user toggles, auth state, an availability check that can flip mid-session. → **Use `should_render`.**

The reason is *when each is evaluated*. The build-time `if` runs once, when the page is constructed, and its result is frozen until something rebuilds the page (for AI/Code subpages, only switching subpages does — see below). `should_render` is re-evaluated on every filter and render pass, so it tracks a value that changes while the settings page is open. Gate a runtime-changing value with a build-time `if` and the page goes stale; gate a static flag with `should_render` and you carry a widget around for nothing.

The real `should_render` users are all the runtime kind: `SettingsSyncWidget` (`main_page.rs`) on auth state, `WarpDriveToggleWidget` (`warp_drive_page.rs`) on `WarpDriveSettings::is_warp_drive_available`, and the CLI-agent rich-input widgets (`ai_page.rs`) on the user-toggleable footer setting, via `should_render_cli_agent_rich_input`.

Either mechanism keeps search honest: an uncreated widget isn't in the list, and `update_filter` already skips a widget whose `should_render` is false. What you must never do is hide rows inside `render` while `search_terms` still advertises them — that makes the page match a query and then show nothing for it.

## The decision that matters: where does the heading live?

For any heading on a settings page, ask **what does it name?**

- **It names the whole page** → it belongs in the `PageType` title slot. On `Uncategorized` / `Categorized` that is also what keeps it on screen while the page is filtered; on a `Monolith` it is a structural choice only (see above).
- **It names one section among several** → it belongs inside the widget (or `Category`) that owns that section, so it disappears together with its own rows.

Getting this wrong in the first direction is bug class 1 below. Getting it wrong in the second direction leaves a section header stranded above unrelated rows.

### Page-shape taxonomy

Classify the page before you write it:

- **Single-topic page** — one heading names everything on it, but the content is still made of separately-matchable widgets. Knowledge, Third party CLI agents, Editor and Code Review, Account, Scripting. → **Title in the `PageType` slot.**
- **Multi-section page** — several independent sections, each with its own heading. Warp Agent, Agent profiles, Appearance, Features. → **Per-section headings live in widgets/categories** and correctly disappear with their rows. (For `Categorized`, `get_filtered` drops categories whose widgets all filtered out, so their subheaders vanish automatically — that's the behavior you want.)
- **Monolith page** — Keybindings, Teams, About, Environments, Codebase Indexing. There is no partial-match state: the sole widget either matches, and the whole page renders, or it doesn't, and the whole page renders empty and drops out of the sidebar. So a monolith can never strand an orphaned setting under a missing heading — it is **not** affected by bug class 1, but for that reason, *not* because its title is protected. Passing the title through the slot is still the tidier structure (Billing and Usage and Referrals do), just don't expect it to keep the title on screen during a non-matching search; on a monolith it will not. Codebase Indexing looks single-topic but is really one unfilterable widget covering the whole page (`CodePageWidget`, wrapped by `CodeIndexingPageWidget`) — building it as `Uncategorized` instead of `Monolith` made the sidebar show a permanent, misleading "(1)" on any match ([APP-5530]).

The two are not mutually exclusive: a page can name itself in the title slot **and** have per-section subheaders inside its widgets. Privacy does exactly that — `PageType::new_uncategorized(widgets, Some("Privacy"))` plus `render_sub_header` calls inside individual widgets. The rule is per heading, not per page.

Worked positive example: **Scripting** (`scripting_page.rs`) is a small page done right — two focused widgets (`WarpControlCliInstallWidget`, `LocalControlModeWidget`) with their own `search_terms`, and `PageType::new_uncategorized(widgets, Some("Scripting"))`. A ticket was once filed claiming Scripting needed splitting; it was canceled because the premise was wrong. A small page is not automatically a mega-widget.

## Bug class 1 — the vanishing title

**Symptom:** on a single-topic `Uncategorized` or `Categorized` page, typing a search term that matches one row makes the page heading disappear along with the non-matching rows, leaving an unlabeled orphan setting. (A monolith can't reach this state — see the taxonomy above.)

**Cause:** the only heading was rendered inside a widget — via `build_sub_header` / `render_page_title` in that widget's `render`, or via a header-only widget that exists just to draw a title. Filtering removes the widget, and the heading goes with it.

Canonical fix — commit `ddadcee` ([APP-5060], #14519), Knowledge. Before, `AIFactWidget::render` opened with:

```rust
let header = build_sub_header(appearance, "Knowledge", …).finish();
let mut column = Flex::column().with_child(header) /* … all the Knowledge rows … */;
```

After, the heading moved to page chrome and the rows became focused widgets:

```rust
let title = match subpage {
    AISubpage::Knowledge => Some("Knowledge"),
    AISubpage::ThirdPartyCLIAgents => Some("Third party CLI agents"),
    AISubpage::WarpAgent | AISubpage::Profiles => None,
};
PageType::new_uncategorized(widgets, title)
```

(The `ThirdPartyCLIAgents` arm and the exhaustive `match` came from #14524; note the deliberate absence of a `_` arm, so a new subpage forces this decision.)

The same commit (#14524) deleted `CodeSubpageHeaderWidget` from the then-combined Code page — a widget whose entire job was `build_sub_header(appearance, self.title, None)` — and replaced it with a title passed through `PageType`. Those two halves are now separate pages, `code_indexing_page.rs` and `code_editor_review_page.rs`, each passing its own `PAGE_TITLE` through the slot. **A header-only widget is always this bug.** If a widget renders nothing but a title, delete it and use the title slot.

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
- **Give each row the right gating mechanism** (see "Two ways to gate a widget"). A row that used to sit behind an `if … { … }` inside a mega-`render` becomes its own widget: if its condition is a static feature flag or platform check, gate it with an `if` in the page's build function and never create it; if it can change at runtime, give it `should_render`. The CLI-agent rich-input rows are the runtime case — they hang off the user-toggleable footer setting — so they use `should_render` and share the predicate through a small free function (`should_render_cli_agent_rich_input(app)`) rather than duplicating the condition.
- **Move per-row state with the row.** Each `SwitchStateHandle` / `MouseStateHandle` moves to the widget that owns its control. Never create one inline while rendering (see `gui-ui-guidelines` and the AGENTS.md note on `MouseStateHandle`).
- **Watch the widget ids.** `widget_id()` is `std::any::type_name::<Self>()`, so splitting a widget changes ids. `settings_widget_deeplink_target` in `app/src/settings_view/mod.rs` maps stable public slugs (`warp://settings?widget=<slug>`) onto them. The CLI-agent split deliberately kept `CLIAgentWidget` as the first widget so `cli_agent_settings_widget_id()` — the target of the `cli_agents` deeplink — stayed valid. If you rename or remove a widget that backs a deeplink, re-point the accessor.

## Subpages rebuild their `PageType` — reapply the filter

AI subpages rebuild their `PageType` when the active subpage changes (`AISettingsPageView::set_active_subpage` / `build_page`). A fresh `PageType` starts with **every** widget in its filter, so a live search query is silently dropped unless it's reapplied. `SettingsView::reapply_search_filter_to_active_subpage` in `app/src/settings_view/mod.rs` exists for exactly this ([APP-4922], #14116). If you add a code path that rebuilds a subpage's page while search may be active, call it.
The Code umbrella no longer works this way: `CodeIndexing` and `EditorAndCodeReview` are separate pages that each own their widgets outright, so nothing rebuilds and there is no filter to reapply. Prefer that shape for new umbrella children — a subpage that owns its own page needs none of this machinery.

## Known anti-examples still in the tree

Useful to read, not to copy:

- **Warpify** (`warpify_page.rs`) — `PageType::new_categorized(categories, None)` where the first category is `Category::new("", vec![Box::new(TitleWidget::default())])` and `TitleWidget::render` calls `render_page_title("Warpify", …)`. Single-topic page, title inside a widget: bug class 1.
- **Warp Drive** (`warp_drive_page.rs`) — `PageType::new_uncategorized([WarpDriveHeaderWidget, WarpDriveToggleWidget], None)` with no title slot at all. `WarpDriveHeaderWidget` is a conditional sign-up banner, so a signed-in user sees only a bare toggle row and no page heading.

## How to verify a settings-page change

Verification is cheap here; do both.

1. **Unit-test the filter.** `app/src/settings_view/mod_tests.rs` has the harness — `StubWidget`, `stub_widgets_page`, and `visible_widget_count` (which reads `PageType::get_filtered`). Assert that a term unique to one widget yields exactly one visible widget, and that clearing the query restores all of them. Follow the `${filename}_tests.rs` convention from AGENTS.md; run with `cargo nextest run -p warp settings_view`.
2. **Exercise the real search.** Open Settings, go to the page, and check three things:
   - a term matching exactly one row leaves **only** that row,
   - the **page title is still visible** while that filter is active — expect this on an `Uncategorized` / `Categorized` page; a monolith correctly disappears whole instead,
   - clearing the search restores the full page.

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
// Gate the widget instead: an `if` at page-build time for a static flag, or
// should_render() when the condition can change at runtime.
```
