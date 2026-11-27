# Support setting a fallback font
## 1. Problem
Warp has a single terminal font setting backed by `FontSettings::monospace_font_name`. Rendering can fall back to platform fonts or existing app-provided fallback fonts, but users cannot specify an ordered fallback chain. The implementation needs to add a local-only ordered list of terminal fallback font family names, load those fonts without primary-font validation constraints, and consult that list before existing platform or bundled fallback behavior.
The work spans settings, appearance propagation, font loading, terminal rendering, text layout, and validation.
## 2. Relevant code
- `app/src/settings/font.rs:12` — defines `FontSettings`, including `monospace_font_name`, `monospace_font_size`, `monospace_font_weight`, and related text appearance settings.
- `app/src/appearance.rs (62-117)` — listens for `FontSettingsChangedEvent` values and updates the global `Appearance` model.
- `app/src/appearance.rs (327-424)` — loads user-selected font families and builds the initial `Appearance`.
- `crates/warp_core/src/ui/appearance.rs (18-32)` — stores the global monospace font family, font size, weight, line-height ratio, and font change events.
- `app/src/settings_view/appearance_page.rs (391-481)` — defines appearance-page actions for font family, font size, font weight, and related controls.
- `app/src/settings_view/appearance_page.rs (1864-2054)` — populates and updates the existing terminal and AI font dropdowns.
- `app/src/settings_view/appearance_page.rs (3876-4035)` — renders the "Terminal font" settings section.
- `app/src/settings/init.rs (56-105)` — registers all settings groups, including `FontSettings`.
- `app/src/settings/init.rs (125-167)` — validates public settings and reports settings-file errors on startup.
- `app/src/settings/init.rs (171-267)` — hot-reloads `settings.toml` and reloads all public settings.
- `crates/warpui_core/src/fonts.rs (126-237)` — defines `FamilyId`, `FontId`, `Properties`, `Cache`, and the current fallback cache state.
- `crates/warpui_core/src/fonts.rs (409-481)` — implements `glyph_for_char`, including app fallback then system fallback when `include_fallback_fonts` is true.
- `crates/warpui_core/src/fonts/external_fallback.rs (77-164)` — tracks app-provided external fallback fonts and lazy-load requests.
- `crates/warpui_core/src/platform/mod.rs (319-460)` — defines the platform-agnostic `FontDB` and `TextLayoutSystem` traits.
- `crates/warpui_core/src/text_layout.rs (144-269)` — caches laid-out lines/text frames and requests fallback font loads when glyphs are missing.
- `crates/warpui/src/windowing/winit/fonts.rs (525-542)` — loads fontconfig fallback fonts for a selected winit font.
- `crates/warpui/src/windowing/winit/fonts.rs (639-742)` — converts cosmic-text glyphs into Warp `Line` runs and records missing glyphs.
- `crates/warpui/src/windowing/winit/fonts.rs (699-765)` — builds cosmic-text attributes from `StyleAndFont`.
- `crates/warpui/src/windowing/winit/fonts.rs (1100-1168)` — selects a winit font and exposes fallback fonts.
- `crates/warpui/src/windowing/winit/fonts/linux.rs (49-103)` — loads Linux system font families and currently validates primary/system font families for English support.
- `crates/warpui/src/platform/mac/fonts.rs (49-83)` — loads macOS system font families and currently validates primary fonts with an `m` glyph.
- `crates/warpui/src/platform/mac/fonts.rs (287-350)` — builds macOS Core Text cascade fallback lists.
- `app/src/terminal/grid_renderer/cell_glyph_cache.rs (21-93)` — resolves glyphs for non-ligature terminal cell rendering and zero-width sequences.
- `app/src/terminal/grid_renderer.rs (1378-1473)` — renders terminal grid rows through the text layout path when ligature rendering is enabled.
- `app/src/terminal/grid_renderer.rs (1621-1749)` — renders individual grid cell glyphs when ligature rendering is disabled.
- `app/src/terminal/grid_size_util.rs (13-58)` — derives terminal cell dimensions from the primary monospace font and should remain primary-font based.
- `app/src/terminal/view.rs (3536-3563)` — refreshes terminal size on appearance font changes.
- `crates/integration/src/test/settings_file_errors.rs (65-89)` — settings-file invalid-value integration test pattern.
- `crates/integration/src/test/settings_private.rs (42-84)` — public setting persistence test pattern.
## 3. Current state
`FontSettings` has one terminal font family setting at `appearance.text.font_name`. `AppearanceManager` reacts to font setting changes by calling `get_or_load_font_family`, which validates a user-selected terminal font by requiring an `m` glyph before updating `Appearance::monospace_font_family`.
Terminal cell dimensions are computed from `Appearance::monospace_font_family`, font size, and line-height ratio. Non-ligature grid rendering calls `FontCache::glyph_for_char(..., include_fallback_fonts: true)`, which currently tries app-provided external fallbacks and then the platform fallback list. Ligature rendering and other text-layout paths call the platform `TextLayoutSystem`; on winit this delegates to cosmic-text, and on macOS it delegates to Core Text.
The existing native font loaders are designed for primary selectable fonts. macOS rejects system fonts that do not contain `m`; Linux's normal family loading validates English support. Those constraints are useful for primary terminal fonts but wrong for fallback fonts such as emoji, symbol, icon, and some CJK-only families.
`app/src/font_fallback.rs` defines app-provided external fallback mappings, but `app/src/lib.rs` only registers that function for wasm. Native builds primarily rely on platform fallback. Users cannot customize the order.
## 4. Proposed changes
### 4.1 Add a local-only fallback font setting
Add a new setting to `FontSettings` in `app/src/settings/font.rs`:
- Rust field: `monospace_fallback_font_names` or `fallback_font_names`
- Type: `Vec<String>`
- Default: `Vec::new()`
- `supported_platforms: SupportedPlatforms::ALL`
- `sync_to_cloud: SyncToCloud::Never`
- `private: false`
- Suggested `toml_path: "appearance.text.fallback_fonts"`
- Suggested `storage_key: "FallbackFontNames"` only if native preference migration or legacy storage needs a stable key
- Description: "Ordered fallback font families used when the terminal font cannot render a glyph."
Use the generated `FontSettingsChangedEvent` for this setting to refresh font fallback state. If `Vec<String>` does not already satisfy the settings/schema traits in this context, introduce a small transparent newtype such as `FallbackFontNames(Vec<String>)` deriving `Serialize`, `Deserialize`, `Clone`, `Default`, `PartialEq`, `JsonSchema`, and `SettingsValue`.
### 4.2 Add fallback-specific font loading
Do not reuse `get_or_load_font_family` as-is for fallback fonts because it intentionally rejects fonts without an `m` glyph.
Add a fallback-loading path that:
- accepts font families without `m`
- accepts fonts that do not advertise English support
- still rejects malformed names, unreadable files, unsupported font formats, and families with no loadable faces
- logs per-font failures and continues loading later configured fallback names
One possible implementation is:
- Add a load purpose enum in the platform layer, for example `SystemFontLoadPurpose::PrimaryText` and `SystemFontLoadPurpose::Fallback`.
- Extend `platform::FontDB` with a fallback-specific method, for example `load_fallback_from_system(&mut self, font_family: &str) -> Result<FamilyId>`, rather than changing primary font semantics.
- In `crates/warpui/src/platform/mac/fonts.rs`, keep the current `m` validation for primary font loading and bypass it for fallback loading.
- In `crates/warpui/src/windowing/winit/fonts/linux.rs`, keep `ValidateFontSupportsEn::Yes` for primary families and use `ValidateFontSupportsEn::No` for fallback families.
- In Windows/winit fallback loading, mirror the native platform's existing fallback-family lookup while avoiding primary-font-only validation.
Expose this through `warpui_core::fonts::Cache`, for example:
- `get_or_load_system_fallback_font(&mut self, font_family: &str) -> Result<FamilyId>`
- `set_configured_fallback_families(&mut self, families: Vec<FamilyId>)`
### 4.3 Track configured fallback families in the font cache
Extend `FontFallbackCache` or a sibling cache struct to store the currently configured ordered fallback family IDs. Keep this separate from app-provided external fallback fonts.
Update `Cache::glyph_for_char` so the `include_fallback_fonts` path checks fallbacks in this order:
1. configured user fallback families
2. existing app-provided external fallback family for that character, when loaded or requested
3. existing platform/system fallback fonts
For each configured fallback family:
- derive fallback `Properties` from the primary selected font's stored properties, as `app_font_fallback` already does for external fallback families
- call `select_font(fallback_family, properties)`
- call `glyph_for_char(fallback_font, ch, false)`
- return the first matching glyph/font pair
Preserve the current behavior for `include_fallback_fonts: false` so primary-font validation and direct glyph checks are not affected.
### 4.4 Propagate fallback settings through Appearance
Extend `Appearance` with:
- an ordered fallback family list, preferably `Arc<[FamilyId]>` or `Vec<FamilyId>`
- an accessor such as `monospace_fallback_font_families()`
- an event such as `AppearanceEvent::MonospaceFontFallbacksChanged`
- a setter that invalidates all views and emits the event
Update `build_appearance` to resolve the fallback setting at startup, then call both:
- `Appearance::new(..., fallback_families, ...)`
- `Cache::set_configured_fallback_families(fallback_families.clone())`
Update `AppearanceManager` to handle the generated fallback setting event by resolving configured names, updating `Appearance`, and updating the font cache. If resolving a primary font changes the primary family, keep the fallback list unchanged but re-render so fallback selection is recalculated against the new primary face.
Avoid making the fallback list affect `grid_cell_dimensions`; `app/src/terminal/grid_size_util.rs` should continue using the primary font family only.
### 4.5 Make text layout honor configured fallbacks
Non-ligature terminal rendering will use the `Cache::glyph_for_char` path after section 4.3. Ligature rendering and many inline terminal text surfaces go through `platform::TextLayoutSystem`, so they also need an explicit fallback-chain path.
Recommended approach:
- Extend `StyleAndFont` or a nearby text-layout input type with an optional ordered fallback family list, or add fallback chain access to `fonts::TextLayoutSystem`.
- Include a fallback-chain identity or generation in the text layout cache key, or clear text layout caches when the fallback chain changes. Without this, `LayoutCache` can return stale `Line` and `TextFrame` values because the existing cache key only contains the primary family and style runs.
- For winit/cosmic-text, make `TextLayoutSystem::build_attrs_list` or font selection preload configured fallback families into the same `cosmic_text::FontSystem` and prefer them before fontconfig fallback for spans using the terminal monospace family. If cosmic-text cannot accept an ordered family list directly, segment simple terminal text runs before shaping only when the primary font lacks a glyph and a configured fallback supports it; preserve full shaping for grapheme clusters and zero-width sequences.
- For macOS/Core Text, prepend configured fallback descriptors to the cascade list used for a selected terminal font before the default `cascade_list_for_languages` result.
- For direct glyph fallback APIs, make `platform::FontDB::fallback_fonts(character, font_id)` return configured fallback fonts first, then existing platform fallbacks.
The implementation should apply the configured chain only to terminal monospace text. UI text using the default UI font should not pick up terminal fallback settings unless a caller explicitly opts into the terminal monospace style.
### 4.6 Settings UI
Update `AppearanceSettingsPageView` to add a fallback font list editor near the existing "Terminal font" section:
- add actions for adding a fallback font, removing a fallback font, and moving entries up/down
- reuse the available system font loading path used by `update_font_dropdown`
- allow all fonts, not just monospace fonts, when selecting fallback fonts
- preserve configured-but-unavailable names in the selected list
- show the ordered list with enough visual affordance that users understand priority
If drag-and-drop list editing is too large for the first implementation, use add/remove plus up/down controls. This still satisfies ordered-list behavior.
### 4.7 Settings schema and migration behavior
The new setting is public and local-only:
- It should appear in the settings schema with a default of `[]`.
- It should not sync through cloud settings.
- It should not be migrated from issue/comment data or any authenticated GitHub identity.
- Existing users should not get a new native preference value unless they set the fallback list.
Settings-file validation should reject non-array values or non-string entries using the existing settings-file banner flow.
### 4.8 Cache invalidation
Changing fallback configuration must avoid stale glyphs and stale text layout:
- Clear or version `Cache::glyphs_by_char` entries affected by fallback changes. A conservative full clear is acceptable if scoped to glyph cache entries.
- Add `LayoutCache::clear()` or equivalent and invoke it for presenters when `AppearanceEvent::MonospaceFontFallbacksChanged` fires, or include fallback chain generation in `CacheKeyValue`.
- Terminal `CellGlyphCache` is reconstructed per paint in current block-list and alt-screen rendering, so it does not need persistent invalidation.
## 5. End-to-end flow
1. User sets `appearance.text.fallback_fonts = ["D2Coding", "Symbols Nerd Font", "Noto Color Emoji"]` or edits the list through Appearance settings.
2. `FontSettings` emits the generated fallback setting change event.
3. `AppearanceManager` resolves each configured font family with fallback-specific loading:
   - valid names become `FamilyId`s in configured order
   - blank, duplicate, missing, or unloadable names are skipped for rendering but preserved in settings
4. `Appearance` stores the resolved ordered list and emits `MonospaceFontFallbacksChanged`.
5. `fonts::Cache` stores the resolved ordered list and clears or versions fallback-sensitive caches.
6. Terminal views invalidate and re-render.
7. When rendering a character:
   - primary terminal font is checked first
   - configured fallback families are checked in order
   - existing app/platform fallback behavior runs if no configured fallback matches
8. Terminal cell metrics, PTY dimensions, cursor positions, and selection coordinates continue to use the primary terminal font.
## 6. Risks and mitigations
- Risk: fallback fonts without `m` or English support continue to be rejected.
  - Mitigation: add fallback-specific font loading and tests with symbol/emoji-only fonts.
- Risk: text layout and direct glyph rendering choose different fallback fonts.
  - Mitigation: centralize fallback ordering in `Cache`/platform font APIs and add tests covering both ligature-enabled and ligature-disabled terminal rendering paths.
- Risk: layout cache returns stale glyph runs after changing fallback order.
  - Mitigation: include fallback generation in cache keys or clear text layout caches on fallback setting changes.
- Risk: fallback glyph metrics perturb terminal sizing.
  - Mitigation: keep `grid_cell_dimensions` and PTY sizing primary-font-only; do not measure fallback fonts for cell dimensions.
- Risk: non-monospace fallback glyphs can visually overflow cells.
  - Mitigation: treat this as expected terminal-emulator behavior for arbitrary fallbacks; preserve grid model correctness and document that fallback fonts render within primary-font cells.
- Risk: loading many fallback fonts increases startup or settings-change cost.
  - Mitigation: load only configured names, keep the default list empty, reuse font cache entries, and avoid preloading all system fonts on Linux for dropdown rendering beyond existing behavior.
- Risk: platform-specific fallback APIs differ substantially.
  - Mitigation: add trait-level tests for platform test doubles and targeted native tests for macOS/winit code paths where feasible.
## 7. Testing and validation
- Add unit tests for the new setting default and schema shape, following `app/src/settings/schema_validation_tests.rs`.
- Add settings-file integration tests based on `crates/integration/src/test/settings_file_errors.rs`:
  - valid `fallback_fonts = ["A", "B"]` loads without a settings error
  - invalid `fallback_fonts = "A"` triggers the settings error banner
- Add font cache tests with test fonts or platform test doubles:
  - primary glyph wins over fallback glyph
  - first configured fallback with the glyph wins
  - later fallback is used when earlier fallback lacks the glyph
  - configured fallbacks are skipped when the font fails to load
- Add platform-loader tests where practical:
  - fallback loading allows a family that lacks `m`
  - Linux fallback loading does not require English support
- Add terminal rendering validation for both ligature states:
  - `use_ligature_rendering = false` exercises `CellGlyphCache::glyph_for_char`
  - `use_ligature_rendering = true` exercises the text layout path in `grid_renderer.rs`
- Add manual validation on at least one winit/Linux environment and one macOS environment:
  - primary programming font + CJK fallback
  - primary programming font + Nerd Font fallback
  - primary programming font + emoji fallback
  - fallback order reversal visibly changes glyph source when two fallbacks contain the same glyph
## 8. Follow-ups
- Add presets or recommendations for common fallback chains.
- Add codepoint-range or script-specific mappings for users who need deterministic per-script font selection.
- Consider native drawing for additional box-drawing, Powerline, and Nerd Font symbols beyond existing native glyph handling.
- Consider exposing fallback configuration to non-terminal monospace surfaces if users request it after terminal support ships.
