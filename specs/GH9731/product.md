# PRODUCT.md — File Tree Icon Themes

**GitHub Issue:** [warpdotdev/warp#9731](https://github.com/warpdotdev/warp/issues/9731)
**Figma:** none provided
**Design reference:** issue comment mock image provided in GitHub discussion, not a Figma source of truth

## Summary

Warp's Project Explorer should support user-selectable file tree icon themes instead of relying on a fixed built-in icon set. The first version adds a File Tree icon theme picker in Settings → Appearance, ships two bundled themes, and defines a documented read-only icon theme format based on Nerd Font codepoints behind an `iconDefinitions` abstraction that can later grow SVG support without breaking existing themes.

The bundled themes are:

1. **Warp Default** — preserves today's Project Explorer icon behavior and fallback experience.
2. **Seti-style** — a community-style Nerd Font icon theme that demonstrates the contributed-theme shape.

## Problem

The Project Explorer currently exposes no theming hook for file and folder icons. Users who prefer editor-style visual language from tools such as VS Code, Zed, Neovim file trees, Yazi, or lf cannot carry that scanning experience into Warp. Every missing or incorrect file-type icon also becomes a Warp-maintained product request rather than a contribution to a theme catalog.

As Warp becomes an agentic development environment, the Project Explorer is part of the primary navigation surface. File-type and folder icons should be configurable enough for users to scan repositories quickly while keeping the first implementation bounded.

## Goals

1. Add a user-facing File Tree icon theme picker under Settings → Appearance.
2. Ship two bundled themes: Warp Default and a Seti-style Nerd Font theme.
3. Make Warp Default the default selection and preserve the current Project Explorer experience for users who do not change the setting.
4. Define a documented theme format with `iconDefinitions` and mapping sections for exact file names, extensions, language IDs, folder names, folder-open names, and fallbacks.
5. Resolve icons in this order: exact filename, extension, language ID, folder name with open/closed state, fallback.
6. Allow special folders such as `.git`, `node_modules`, `src`, `dist`, `target`, `public`, and `test` to be expressed by theme data rather than hard-coded UI logic.
7. Make bundled theme additions reviewable through ordinary repository pull requests.
8. Keep v1 read-only: users can select shipped themes, but there is no in-app theme editor and no local user-defined theme folder support yet.

## Non-goals

- No generalized extension API.
- No in-app editor or visual theme builder.
- No per-file user-level overrides.
- No local user-defined icon theme folder loading in v1.
- No SVG theme file support in v1, beyond preserving the current Warp Default rendering where needed.
- No automatic import of VS Code, Zed, Material Icon Theme, vscode-icons, nvim-tree, Yazi, or lf theme files.
- No icon recoloring beyond the color specified by the selected theme and the existing row selected/hovered treatment needed for readability.
- No changes to Project Explorer tree behavior such as sorting, expansion, drag/drop, context menus, or file-opening semantics.

## User Experience

### Settings

1. Users see a new **File Tree icon theme** setting under Settings → Appearance.
2. The setting is presented as a picker or dropdown with at least:
   - Warp Default
   - Seti-style
3. Warp Default is selected for new and existing users unless they choose another value.
4. The setting should be searchable by terms such as `file tree`, `project explorer`, `icons`, `icon theme`, `seti`, and `nerd font`.
5. Selecting a theme updates Project Explorer icons without requiring app restart.
6. The selected theme persists across app restarts.
7. The selected theme syncs through the same settings mechanism used for globally syncable appearance settings, unless the implementation discovers a repository policy that prevents syncing this setting.
8. If the selected theme is unavailable due to a corrupted bundled definition or unknown stored setting value, Warp falls back to Warp Default and does not break the Project Explorer.

### Project Explorer icon behavior

1. File rows render the selected theme's file icon when a theme rule matches.
2. Directory rows render the selected theme's folder icon. Themes may distinguish closed and open folders.
3. Existing selection, hover, ignored-file styling, indentation, chevrons, drag previews, and row click behavior remain unchanged.
4. Icons stay within the same visual footprint as current file tree icons so row height and alignment do not change materially.
5. If a file has no matching rule, Warp renders the selected theme's file fallback.
6. If a folder has no matching rule, Warp renders the selected theme's folder fallback for the correct open/closed state.
7. Dotfiles and extensionless files can be matched by exact file name before fallback handling.
8. Case handling is deterministic:
   - Extension matching is case-insensitive for normal file extensions.
   - Exact filename and folder-name matching use the normalized names defined by the theme format, with dot-prefixed names supported.
9. Special folder mappings are data-driven. For example, a theme can assign distinct icons for `.git`, `node_modules`, `src`, `dist`, and `target`.
10. If a Nerd Font glyph is not available in the user's configured font stack, Warp should still keep the row usable. It may show the platform font fallback glyph, but Warp Default remains available as the no-surprise option.

### Theme format behavior

The documented v1 format is read-only from the product perspective: users and contributors can inspect it and propose bundled theme changes through PRs, but Warp does not load arbitrary user files in v1.

Required behavior:

1. Each theme has a stable ID, display name, and `iconDefinitions` section.
2. An icon definition can specify a Nerd Font codepoint or glyph string and an optional color.
3. Mapping sections can reference icon definition IDs.
4. Supported mapping sections include:
   - exact file names
   - file extensions
   - language IDs
   - folder names
   - folder names when open
   - file fallback
   - folder fallback
   - folder-open fallback
5. Mapping references to missing icon definition IDs are invalid theme data. Bundled themes with invalid data should fail validation during development and fall back safely at runtime.
6. The format should reserve a compatible path for future SVG support, such as allowing an icon definition to gain an optional SVG field later while keeping existing glyph-only themes valid.

### Global file search behavior

Global file search currently reuses the code file icon lookup path. If implementation keeps that shared path, search results should use the selected file icon theme for file results. If implementation narrows the first release to the Project Explorer only, the product behavior must be called out in the implementation PR and not left ambiguous.

### Empty, loading, and error states

1. Empty Project Explorer states are unchanged.
2. Remote-session and disabled Project Explorer messaging are unchanged.
3. Loading or lazily loaded directories use the same folder icon rules once rows are rendered.
4. Missing or invalid icon theme data never prevents the file tree from rendering file and folder names.

## Success Criteria

1. Settings → Appearance contains a File Tree icon theme picker with Warp Default and Seti-style options.
2. Existing users who do nothing see the current Project Explorer icon behavior preserved by Warp Default.
3. Selecting Seti-style changes file and folder icons for common files and folders in an open workspace without requiring restart.
4. File icon resolution follows exact filename → extension → language ID → fallback.
5. Folder icon resolution supports folder name and open/closed state before folder fallback.
6. Dotfiles such as `.gitignore` and special folders such as `.git` and `node_modules` can receive theme-specific icons.
7. Unknown extensions and unknown folder names always render a fallback icon instead of a blank space.
8. Ignored file/folder styling, selected row styling, hover styling, keyboard navigation, click handling, context menus, drag/drop, and open-in-pane/open-in-tab behavior do not regress.
9. The selected icon theme persists across app restarts and survives unknown/corrupted stored values by falling back to Warp Default.
10. The bundled theme format is documented enough that a contributor can add or adjust a bundled icon mapping in a PR without modifying matching code.

## Validation

1. Manual validation: open a workspace containing Rust, TypeScript, JavaScript, Python, JSON, Markdown, shell, Go, C/C++, Terraform, unknown extension, extensionless file, `.gitignore`, `.git`, `node_modules`, `src`, `dist`, and `target`; verify Warp Default and Seti-style render expected icons and fallbacks.
2. Manual validation: switch themes from Settings → Appearance while the Project Explorer is visible; verify visible rows update and interactions still work.
3. Manual validation: restart Warp after choosing Seti-style; verify the setting is preserved.
4. Manual validation: use keyboard navigation, click-to-open, context menu, and drag/drop in the Project Explorer after switching themes.
5. Product screenshot or integration artifact: capture the Project Explorer with both bundled themes selected for the same sample workspace.
6. Automated validation should cover theme resolution precedence, invalid theme references, fallback behavior, and at least one file tree render path using each bundled theme.

## Open Questions

1. Should Global file search intentionally share the selected File Tree icon theme in v1, or should the setting apply only to the Project Explorer until search-specific product review happens?
2. Should the File Tree icon theme setting live under Settings → Appearance only, or also be discoverable from Settings → Code because Project Explorer enablement currently lives there?
3. Should the Seti-style bundled theme require users to configure a Nerd Font explicitly, or should Warp choose a bundled/fallback font for glyph icons if available?
4. What exact license/source attribution is required for the Seti-style glyph mapping and color palette before it ships?
