# Accent-title active pane indicator — Engineering Spec

GitHub issue: https://github.com/warpdotdev/warp/issues/15668

Code references inspected at commit: `066ec71b736fc3755e29f58f733deadbdac3d1af`

## Summary

Add a synced `appearance.panes.active_pane_indicator_style` setting with `corner_marker` as the default and `accent_title` as the alternative. Move active-indicator state from terminal-session state to the pane focus model. The pane-header framework will color standard titles. Custom headers must opt in and apply the color through a render-context contract. An unsupported custom header will keep the corner marker.

This document is the implementation contract. The product mock remains on issue #15668.

## Current system

- `PaneConfiguration.show_active_pane_indicator` stores a Boolean indicator state. `TerminalView::refresh_pane_header` is its only producer and sets it from terminal `is_active_session`, not pane focus (`app/src/pane_group/pane/mod.rs:694-751`, `app/src/terminal/view/pane_impl.rs:107-119`).
- `PaneHeader::render` draws `bundled/svg/upper-left-triangle.svg` after either standard or custom content when that Boolean is true (`app/src/pane_group/pane/view/header/mod.rs:755-837`, `app/src/pane_group/pane/view/header/mod.rs:1004-1024`).
- `PaneHeader` already owns a `PaneFocusHandle` and refreshes for pane focus, split, and maximization changes (`app/src/pane_group/pane/view/header/mod.rs:182-216`).
- `PaneFocusHandle::is_focused` is true for a focused split pane, a maximized pane, and the sole pane in an unsplit group (`app/src/pane_group/focus_state.rs:158-184`, `app/src/pane_group/mod.rs:1042-1080`).
- `HeaderContent` separates framework-owned `Standard` headers from view-owned `Custom` headers. Custom headers currently declare only drag ownership (`app/src/pane_group/pane/view/header_content.rs:14-150`).
- Standard title color is `theme.sub_text_color(theme.background())` inside `PaneHeader::render_standard_header` (`app/src/pane_group/pane/view/header/mod.rs:582-724`).
- The shared custom-header title helper uses the same sub-text color (`app/src/pane_group/pane/view/header/components.rs:110-126`).
- The existing custom headers are Terminal, AI document/plan, Markdown/Jupyter file, and Code headers (`app/src/terminal/view/pane_impl.rs:733-751`, `app/src/ai/ai_document_view.rs:769-831`, `app/src/ai/ai_document_view.rs:1379-1388`, `app/src/notebooks/file/mod.rs:1222-1317`, `app/src/code/view.rs:1593-1664`, `app/src/code/view.rs:1758-1953`, `app/src/code/view.rs:2482-2500`).
- Pane settings already use globally synced settings that respect the user's sync preference. The Appearance page has a Panes category and settings-search widgets (`app/src/settings/pane.rs:1-25`, `app/src/settings_view/appearance_page.rs:1498-1505`, `app/src/settings_view/appearance_page.rs:3951-4033`).
- Warp's contrast utilities can shift a foreground toward black or white until it reaches the text threshold of 4.5:1. `ContrastingColor` applies this logic to solid and gradient `Fill` values (`crates/warp_core/src/ui/color/contrast.rs:26-80`, `crates/warp_core/src/ui/color/contrast.rs:128-184`, `crates/warp_core/src/ui/theme/mod.rs:384-411`).

## Technical design

### 1. Add the synced enum setting

In `app/src/settings/pane.rs`, add:

```rust
#[derive(
    Clone, Copy, Debug, Default, Eq, PartialEq, Deserialize, Serialize,
    Sequence, schemars::JsonSchema, settings_value::SettingsValue,
)]
#[schemars(
    description = "How the focused pane is identified in its header.",
    rename_all = "snake_case"
)]
pub enum ActivePaneIndicatorStyle {
    #[default]
    CornerMarker,
    AccentTitle,
}
```

Register `PaneSettings.active_pane_indicator_style` as `ActivePaneIndicatorStyleSetting` with:

- type: `ActivePaneIndicatorStyle`
- default: `CornerMarker`
- platforms: `SupportedPlatforms::ALL`
- sync: `SyncToCloud::Globally(RespectUserSyncSetting::Yes)`
- surface: `settings::SettingSurfaces::GUI`
- private: `false`
- TOML path: `appearance.panes.active_pane_indicator_style`
- serialized values: `corner_marker` and `accent_title`
- description: `How the focused pane is identified in its header.`

In `app/src/settings_view/appearance_page.rs`:

- Add `AppearancePageAction::SetActivePaneIndicatorStyle(ActivePaneIndicatorStyle)`.
- Add one `Dropdown<AppearancePageAction>` to `AppearanceSettingsPageView`.
- Add an `ActivePaneIndicatorStyleWidget` as the first item in Appearance → Panes.
- Label the row `Active pane indicator`.
- Label the options `Corner marker` and `Accent title`.
- Use `ActivePaneIndicatorStyleSetting::storage_key()` and `sync_to_cloud()` for the local-only icon contract.
- Use the search terms `active pane indicator focused focus corner marker triangle accent title pane header`.
- Save through `PaneSettings::handle(ctx)` and `set_value`.

`PaneHeader` must subscribe to `PaneSettingsChangedEvent::ActivePaneIndicatorStyle`. A setting update must notify every rendered pane header without recreating a tab, pane, or workspace.

### 2. Derive activity from pane focus

Remove these members after all reads are migrated:

- `PaneConfiguration.show_active_pane_indicator`
- `PaneConfiguration::set_show_active_pane_indicator`
- `PaneConfigurationEvent::ShowActivePaneIndicatorUpdated`
- the `set_show_active_pane_indicator(is_active_session, ...)` call in `TerminalView::refresh_pane_header`

Do not replace them with another mutable Boolean. In `PaneHeader::render`, compute:

```rust
let is_active_pane = self
    .focus_handle
    .as_ref()
    .is_some_and(|handle| handle.is_focused(app))
    && !self.pane_configuration.as_ref(app).dim_even_if_focused();
```

This is an intentional behavior expansion. With either style selected, Notebook, file, Code, AI document, and future pane types use the same focus source as Terminal. `active_session_id` remains terminal input/session state and no longer controls pane-header presentation.

The selected pane in the active top-level tab updates through the existing focus-state subscription. Split focus changes update the old and new pane headers. A maximized pane remains active because `PaneFocusHandle::is_focused` includes `PaneState::Maximized`.

The invariant applies when a pane header is rendered. Do not force a normally hidden header to become visible only to show an indicator.

### 3. Make indicator selection one framework decision

Add these declarative types in `app/src/pane_group/pane/view/header_content.rs`:

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CustomHeaderAccentTitleSupport {
    Unsupported,
    UsesHeaderRenderContext,
}

pub struct HeaderRenderContext<'a> {
    // Existing fields remain unchanged.
    pub active_title_color: Option<Fill>,
}
```
Add `accent_title_support: CustomHeaderAccentTitleSupport` to the `HeaderContent::Custom` variant. Every `HeaderContent::Custom` constructor must set it. There is no implicit supported default.

- `Unsupported` means that the custom header does not apply `active_title_color` to one semantic title.
- `UsesHeaderRenderContext` means that the custom header applies `active_title_color`, when present, only to its semantic active title.
- `HeaderContent::Standard` is always supported because the framework owns its primary title.

`PaneHeader::render` will resolve a single presentation:

1. If `is_active_pane` is false, set `active_title_color` to `None` and draw no corner marker.
2. If the style is Corner marker, set `active_title_color` to `None` and draw the corner marker.
3. If the style is Accent title and the content is Standard, color only the primary title and draw no corner marker.
4. If the style is Accent title and a Custom header declares `UsesHeaderRenderContext`, let it color its title and draw no corner marker.
5. If the style is Accent title and a Custom header declares `Unsupported`, set `active_title_color` to `None` and draw the corner marker.

Implement the branch through this small pure presentation type and resolver:

```rust
enum ActivePaneIndicatorPresentation {
    None,
    CornerMarker,
    AccentTitle(Fill),
}

fn resolve_active_pane_indicator(
    is_active_pane: bool,
    style: ActivePaneIndicatorStyle,
    header_support: CustomHeaderAccentTitleSupport,
    accent_title_color: Fill,
) -> ActivePaneIndicatorPresentation;
```

Represent Standard headers as supported when calling the resolver. Rendering code must not independently decide whether to draw the corner marker. This provides the exactly-one guarantee for every rendered focused header.

### 4. Framework-color Standard titles

Change `PaneHeader::render_standard_header` to accept `title_color: Option<Fill>`.

- Apply the accent override only to `StandardHeader.title`.
- Keep `title_secondary`, `left_of_title`, `right_of_title`, badges, icons, close and overflow controls on their current colors.
- Keep `title_style`, `title_clip_config`, `title_max_width`, shrink behavior, hover behavior, and the three-column layout unchanged.

Views that return `HeaderContent::simple` or `HeaderContent::Standard` require no per-view color logic. This includes Notebook and every other standard pane header.

### 5. Resolve a readable local accent foreground

Add one pane-header color helper near `render_pane_header_title_text`. Resolve the title fill from the theme without mutating the theme:

```rust
fn resolved_accent_title_color(appearance: &Appearance) -> Fill {
    appearance
        .theme()
        .accent()
        .on_background(
            appearance.theme().background(),
            MinimumAllowedContrast::Text,
        )
}
```

This uses `foreground_color_with_minimum_contrast` through the existing `ContrastingColor` implementation:

- If the raw accent already reaches 4.5:1 against the header surface, return it unchanged.
- Otherwise, minimally shift each solid color or gradient stop toward the higher-contrast black/white foreground until it reaches 4.5:1.
- Use `theme.background()` because pane headers currently render and resolve text against that surface.
- Keep the result local to pane-header title rendering. Do not write it to `WarpTheme`, `ThemeSettings`, or any setting.
- Always apply the text threshold. Do not branch on `FontSettings.enforce_minimum_contrast`; that setting controls terminal text, while pane-header text already uses Warp's UI text-contrast path.

The current contrast helper is total for valid theme fills. Contrast failure therefore does not normally cause a corner fallback. If a future header surface cannot produce a valid resolved fill, treat that custom header as `Unsupported` and use the corner marker. Do not render an unreadable raw accent as title text.

### 6. Update supported custom headers

Extend the shared helper to this signature and preserve its font and clipping defaults:

```rust
pub fn render_pane_header_title_text(
    title: impl Into<Cow<'static, str>>,
    appearance: &Appearance,
    clip_config: ClipConfig,
    title_color: Option<Fill>,
) -> Box<dyn Element>;
```

When `title_color` is `None`, use the current `theme.sub_text_color(theme.background())`.

The v1 support matrix is:

- Terminal rich header: supported. Apply the override only to the terminal or active AI-conversation title in `TerminalView::render_header_title`.
- AI document/plan: supported. Apply it only to the plan title in `AIDocumentView::render_plan_header`.
- Notebook: supported through the Standard header path.
- Markdown and rendered Jupyter file views: supported. Apply it only to the centered file title in `FileNotebook::render_header_content`.
- Code with zero or one file: supported. Apply it only to the centered file title.
- Code with multiple files: supported. Pass the optional accent title fill into `render_tab_internal`. Use it only when both the pane is active and `is_active` is true.

For multi-file Code panes, split the existing `text_color` responsibility:

- Keep the current active/inactive tab text color as the base color.
- Use the resolved accent only for the selected file-name `Text`.
- Keep inactive file-name text on the existing sub-text color.
- Keep the unsaved-dot color on the existing base color.
- Keep language icons, close buttons, tab borders, hover states, preview italics, tooltips, and drag elements unchanged.

Each listed Custom header must declare `UsesHeaderRenderContext`. Any future or unconverted Custom header must declare `Unsupported` and receives the corner fallback while focused in Accent title mode.

### 7. Preserve interaction behavior

- **Side-panel focus:** `dim_even_if_focused == true` means pane focus is not the current interaction target. Hide both title accent and corner marker. Restore the last-focused pane's selected indicator as soon as pane focus returns. Ensure `DimEvenIfFocusedUpdated` notifies the header as well as the pane body.
- **Maximized pane:** continue to show the selected indicator. Maximization changes layout, not focus.
- **Inactive-pane dimming:** evaluate activity before the inactive overlay. Inactive panes have no indicator. A side-panel-focused pane uses normal title color and receives the existing dim overlay.
- **Accent border:** leave `show_accent_border` unchanged and independent (`app/src/pane_group/pane/view/mod.rs:419-430`). It may coexist with either indicator style because it is not the active-pane indicator.
- **Themes:** recompute the title fill on every render from the active `Appearance`. Theme, system-theme, custom-theme, transparency, and minimum-contrast changes therefore update the title without changing this setting.
- **Header mechanics:** do not change element hierarchy, hit targets, overlay order, drag ownership, clipping, tooltips, badges, controls, hover logic, or dimensions except for passing the title color.

## Decisions and alternatives

### Pane focus, not terminal active-session state

- Chosen: use `PaneFocusHandle::is_focused` for both styles.
- Advantage: one source works for every pane type and follows focus immediately.
- Cost: Corner marker expands from active terminal sessions to every focused pane type.
- Rejected: retain terminal gating for Corner marker. This would make the two styles disagree about which pane is active and would leave non-terminal panes without a Corner marker fallback.

### Capability on `HeaderContent::Custom`

- Chosen: require an explicit `CustomHeaderAccentTitleSupport` field and pass the resolved color through `HeaderRenderContext`.
- Advantage: capability is declared at the render boundary where the framework chooses fallback, and the custom view retains ownership of its title element.
- Rejected: add a `BackingView` trait method. One backing view can return different header shapes at runtime, so a view-level capability can be less precise than the returned content.
- Rejected: infer support from pane type. This is not extensible and can silently omit the fallback.

### Framework-owned Standard title color

- Chosen: `PaneHeader` colors Standard primary titles.
- Advantage: all current and future Standard headers inherit correct behavior without per-view changes.
- Rejected: require each Standard producer to pass a colored title. This duplicates focus, setting, and contrast logic.

### Contrast-adjusted accent

- Chosen: minimally shift the local accent foreground to 4.5:1.
- Advantage: preserves accent identity when possible and keeps Accent title mode available across themes.
- Cost: the displayed title can differ from the raw configured accent on low-contrast surfaces.
- Rejected: fall back whenever the raw accent is below 4.5:1. This would make the selected style change unpredictably between themes.
- Rejected: use the raw accent. This fails the text-contrast requirement.

### Side-panel and maximized behavior

- Chosen: hide the indicator while `dim_even_if_focused` is true, because no pane is the current interaction target.
- Chosen: keep the indicator visible when maximized, because the pane remains focused.
- Reviewer awareness: these were open product questions. They are explicit defaults and may be changed during spec review.

## Assumptions

- The pane header surface remains `theme.background()`. If a header later introduces a different surface, it must pass that surface into the resolver.
- `HeaderContent::Standard` has one semantic primary title. `title_secondary` is supporting text and does not receive the accent.
- The exactly-one invariant applies to a focused pane with a rendered header. This feature does not make hidden headers visible.
- A Custom header that declares `UsesHeaderRenderContext` is responsible for applying the supplied color exactly once to its semantic title. Focused renderer tests enforce the contract.
- No new feature flag or telemetry event is required.

## Out of scope

- New theme-schema tokens or changes to the configured accent.
- User-selected indicator color, marker location, marker size, border style, or additional indicator styles.
- Accent treatment for top-level tab-bar titles or vertical-tab titles.
- Accent treatment for secondary titles, badges, unsaved dots, icons, buttons, borders, or pane content.
- Changes to when pane headers are shown.
- Removal or redesign of `show_accent_border`.

## Validation criteria

### Unit and component tests

1. Add setting tests for `ActivePaneIndicatorStyle`:
   - default and missing value resolve to `CornerMarker`;
   - `corner_marker` and `accent_title` round-trip through `SettingsValue`;
   - the generated schema accepts the default;
   - sync metadata is `Globally(RespectUserSyncSetting::Yes)`.
   - Run: `cargo nextest run -p warp settings::schema_validation_tests`.

2. Add table-driven tests for the pure presentation resolver in `app/src/pane_group/pane/view/header/mod_tests.rs`:
   - inactive pane → none for both settings;
   - focused + Corner marker → corner for Standard and both Custom capabilities;
   - focused + Accent title + Standard → accent title only;
   - focused + Accent title + supported Custom → accent title only;
   - focused + Accent title + unsupported Custom → corner only;
   - `dim_even_if_focused` → none;
   - maximized focus → the selected presentation.
   - Each case asserts that `CornerMarker` and `AccentTitle` are mutually exclusive.
   - Run: `cargo nextest run -p warp pane_group::pane::view::header`.

3. Add contrast tests next to the resolver:
   - a compliant solid accent is unchanged;
   - a low-contrast solid accent reaches `MinimumAllowedContrast::Text`;
   - both gradient stops reach the threshold;
   - dark and light header surfaces select the correct shift direction;
   - the source `WarpTheme::accent()` remains unchanged.
   - Run: `cargo nextest run -p warp pane_group::pane::view::header`.

4. Add focused renderer tests for the supported custom headers:
   - Terminal, AI document/plan, Markdown/Jupyter, Code single-file, and Code multi-file declare `UsesHeaderRenderContext`;
   - a focused multi-file Code pane accents only the selected file-name title;
   - inactive file titles and unsaved dots retain their previous colors;
   - changing the active Code file moves the accent without changing the pane;
   - unsupported Custom test content uses the corner fallback.
   - Run the relevant module tests with `cargo nextest run -p warp terminal::view`, `cargo nextest run -p warp ai::ai_document_view`, `cargo nextest run -p warp notebooks::file`, and `cargo nextest run -p warp code::view`.

5. Extend pane-group focus tests to cover Terminal → Notebook → Code focus transfer, top-level tab switching, side-panel dim state, and maximize/unmaximize. Assert that the presentation follows `focused_pane_id`, not `active_session_id`.
   - Run: `cargo nextest run -p warp pane_group::tests`.

### Settings and GUI integration

6. Add a GUI integration test that:
   - finds `Active pane indicator` through Settings search using `accent title` and `active pane`;
   - verifies Corner marker is selected by default;
   - selects Accent title and observes the focused header update without reopening the pane;
   - switches focus among Terminal, Notebook, Markdown/Jupyter, AI document/plan, and Code panes;
   - verifies unsupported custom test content falls back to one corner marker;
   - restarts settings state and verifies persistence;
   - applies a synced settings update and verifies the rendered style updates immediately.

7. Run the focused GUI integration test through the repository's `crates/integration` registration and command documented by its test module.

### Visual proof

8. Attach a computer-use video to the implementation PR. The video must show the real running app:
   - open Settings → Appearance → Panes and search for the setting;
   - change Corner marker to Accent title and show the immediate update;
   - move focus across split Terminal, Notebook, Markdown/Jupyter, AI plan, Code single-file, and Code multi-file panes;
   - in multi-file Code, switch files and show only the selected title moving to the accent while an unsaved dot remains unchanged;
   - maximize and restore a pane;
   - focus a side panel and return to the pane;
   - switch between representative dark, light, transparent, and custom low-contrast themes.

9. Attach screenshots that hold the dark, light, transparent, and deliberately low-contrast custom-theme states long enough to inspect title legibility and the multi-file Code selected/inactive/unsaved details. Store media as PR artifacts, not repository files.

### Repository checks

10. Run:

```bash
./script/format --check
./script/presubmit
```

The implementation is complete only when the focused tests, GUI integration test, repository checks, and required visual proof pass. If the UI cannot be exercised, record the blocker on the implementation PR and leave visual validation outstanding.
