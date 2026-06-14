# Support setting a fallback font
## 1. Summary
Warp should let users configure an ordered fallback font list for terminal text. When the selected terminal font does not contain a glyph, Warp should try the configured fallback fonts in order before falling back to its existing platform or bundled fallback behavior.
This solves mixed-script and symbol-heavy terminal workflows where users want a readable primary programming font plus separate fonts for CJK/Korean text, Nerd Font or Powerline symbols, brand icons, and emoji.
## 2. Problem
Warp currently exposes a single terminal font setting. Users who prefer fonts such as JetBrains Mono, Hack, or similar programming fonts cannot reliably render every glyph used by modern prompts, git tools, CJK-language output, emoji, or icon fonts unless they switch the entire terminal to a patched or broader-coverage font. That forces a tradeoff between readable Latin/programming text and correct rendering for missing glyphs.
Issue discussion calls out repeated cases where users want:
- a primary Latin/programming font for normal terminal text
- a CJK or Korean fallback such as D2Coding, Noto Sans Mono CJK KR, Source Han Mono, or Noto Sans KR
- a symbol/icon fallback such as Symbols Nerd Font or another Nerd Font
- an emoji fallback such as Apple Color Emoji or Noto Color Emoji
## 3. Goals
- Add a user-visible way to configure an ordered list of terminal fallback font families.
- Preserve the existing primary terminal font behavior and use fallback fonts only when the primary selected face cannot render the character or grapheme.
- Respect the configured order: the first configured fallback that can render the missing glyph wins.
- Apply fallback rendering to terminal text surfaces that use the terminal monospace font, including command output, alt-screen output, prompts, input text, and block terminal labels.
- Support fallback fonts that are not monospace and may not contain Latin glyphs, such as emoji, symbol, icon, or CJK-only fonts.
- Allow settings-file users to configure the fallback list directly with a stable TOML key.
- Update terminal rendering live when the setting changes, without requiring an app restart.
- Keep terminal grid sizing based on the primary terminal font so adding fallback fonts does not resize sessions or alter PTY column/row calculation.
## 4. Non-goals
- This spec does not require codepoint-range mapping, per-script mapping, or conditional rules beyond an ordered fallback list.
- This spec does not require users to install bundled fallback fonts from Warp.
- This spec does not require changing the default terminal font or forcing a default Nerd Font.
- This spec does not require native drawing for box-drawing, Powerline, or Nerd Font symbols beyond existing native glyph handling already present in Warp.
- This spec does not require fallback configuration for arbitrary app UI fonts, AI content fonts, notebook fonts, or editor fonts unless those surfaces are already rendered through the terminal monospace font path.
- This spec does not require cloud-syncing the fallback list because installed font availability is machine-specific.
## 5. Figma / design references
Figma: none provided.
The issue includes a screenshot showing missing or incorrectly rendered glyphs, but no product mock for the settings UI.
## 6. User experience
### Default behavior
- Existing users who do not configure fallback fonts should see no intentional behavior change.
- The primary terminal font remains the source for metrics such as cell width, cell height, line height, and normal glyph rendering.
- Existing system fallback behavior and Warp's existing built-in/web fallback behavior remain available after the configured list is exhausted.
### Settings file behavior
- Users can set an ordered fallback list in `settings.toml` under the terminal text appearance settings.
- The intended shape is:
  - `[appearance.text]`
  - `fallback_fonts = ["D2Coding", "Symbols Nerd Font", "Noto Color Emoji"]`
- An empty list or omitted setting means "no user-configured fallback fonts."
- Duplicate names should be ignored after the first occurrence.
- Blank names should be ignored.
- Font names are interpreted as system font family names exactly as the existing terminal font selector displays them.
### Settings UI behavior
- The Appearance settings page should show a "Fallback fonts" control near "Terminal font."
- The control should make it clear that fallbacks are checked in order after the terminal font.
- Users should be able to add a font family, remove a font family, and reorder selected fallback families.
- The font picker should support the same "view all available system fonts" concept as the existing terminal font picker because valid fallbacks may be non-monospace.
- The selected list should visibly preserve order.
- If a configured font is not currently installed or cannot be loaded, Warp should keep the entry visible so users can fix the name or install the font instead of silently deleting it.
### Rendering behavior
- For a character rendered by the primary terminal font, Warp uses the primary terminal font even if a configured fallback also contains that character.
- For a character not rendered by the primary terminal font, Warp checks configured fallback fonts in list order and uses the first one that contains the glyph.
- If no configured fallback font can render the glyph, Warp continues through its existing built-in and platform fallback paths.
- If no fallback path can render the glyph, Warp's current missing-glyph behavior applies.
- Fallback rendering should preserve terminal text styling where possible:
  - bold text should prefer the bold face of the fallback family when available
  - italic text should prefer the italic face when available
  - if a fallback family lacks a matching face, Warp should use the closest available face rather than failing to render the glyph
- Fallback glyphs should render inside the existing terminal grid cells. The fallback list should not change PTY size, wrapping, cursor movement, selection coordinates, or grid-width calculation.
- Wide-character behavior should continue to follow the terminal model's existing width handling. The fallback font choice should not redefine whether a character occupies one or two cells.
### Error and edge-case behavior
- A missing, misspelled, unsupported, or unloadable fallback font should not crash Warp and should not prevent other configured fallbacks from being used.
- Settings-file parse errors or type errors for `fallback_fonts` should use the existing settings-file error banner behavior.
- Runtime font-loading failures for individual names should be non-blocking and should not rewrite the user's settings.
- Changing the fallback list should update existing terminal sessions and restored sessions after the settings model reloads.
- Fallback fonts must work for symbol and emoji fonts that do not contain the `m` glyph and may not identify as monospace.
## 7. Success criteria
- A user can configure `fallback_fonts = ["Symbols Nerd Font"]` while keeping `font_name = "JetBrains Mono"` or `font_name = "Hack"`, and Nerd Font private-use prompt icons render from the fallback when the primary font lacks them.
- A user can configure a CJK/Korean fallback after a Latin programming font, and mixed English/Korean terminal output renders Latin text from the primary font and Korean text from the configured fallback.
- A user can configure an emoji fallback such as Noto Color Emoji or Apple Color Emoji, and emoji render when the primary font lacks those glyphs.
- Reordering the fallback list changes which fallback font renders a glyph when multiple configured fallbacks support the same character.
- Removing all configured fallbacks returns Warp to its prior behavior.
- Invalid or unavailable fallback font names do not crash Warp and do not prevent later valid fallback fonts from being used.
- Terminal rows, columns, wrapping, cursor placement, text selection, and scrollback behavior remain based on the primary font and current terminal model.
- The setting is available in the settings schema and can be changed by hot-reloading `settings.toml`.
## 8. Validation
- Unit test fallback-chain selection with a primary font missing a glyph and multiple configured fallbacks where only a later fallback contains the glyph.
- Unit test that primary-font glyphs are not overridden by fallback fonts.
- Unit test that fallback font loading allows fonts without an `m` glyph or English language support when used only as fallbacks.
- Settings schema tests should validate the default value for `appearance.text.fallback_fonts`.
- Settings-file integration tests should verify a valid list is accepted and an invalid type triggers the existing settings error banner.
- Manual validation should use a terminal line that mixes Latin, Korean/CJK, Nerd Font symbols, Powerline separators, and emoji while changing the fallback order.
- Manual validation should verify that changing the setting live updates both normal block output and alt-screen output without restarting Warp.
## 9. Open questions
- Should the first UI version include drag-and-drop reordering, up/down buttons, or both?
- Should Warp surface unavailable fallback names inline in the settings UI, or is preserving the entry plus logging sufficient for the first implementation?
- Should Warp provide recommended fallback presets for common cases such as CJK + Nerd Font + emoji after the basic ordered list ships?
- Should future work add per-codepoint or per-script mappings for users who need more deterministic control than an ordered list?
