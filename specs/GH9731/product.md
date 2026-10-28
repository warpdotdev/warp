# Product Spec: File Tree icon themes

**Issue:** [warpdotdev/warp#9731](https://github.com/warpdotdev/warp/issues/9731)
**Figma:** none provided

## Summary

Let users pick a File Tree icon set under **Settings → Appearance**, ship two bundled themes (the existing default plus one Material/Seti-style alternative), and define a documented JSON theme format so the catalog can grow via community PRs in the future. V1 is read-only: themes are loaded from a fixed in-binary set; user-supplied theme files on disk are deferred to a follow-up.

This directly addresses warp/9731. It keeps Warp on the same community-contributable trajectory the terminal-theme catalog established, replaces today's closed `match extension { ... }` table in `app/src/code/icon.rs` with a data-driven registry, and ships zero visual regression for existing users (the `default` theme reproduces today's exact mapping).

## Problem

Warp's File Tree ships a fixed built-in icon set hard-coded in [`app/src/code/icon.rs`](https://github.com/warpdotdev/warp/blob/master/app/src/code/icon.rs). Users have no way to change it. The docs page on the [File Tree](https://docs.warp.dev/code/code-editor/file-tree) explicitly tells users to *file a GitHub issue* whenever an icon is missing. Two consequences (per the issue reporter):

1. Every missing or wrong icon becomes inbound triage work for the Warp team instead of a community PR.
2. Users coming from VS Code, Zed, or terminal file managers (`nvim-tree`, `yazi`, `lf`) cannot bring their preferred look across, even though visual differentiation of file types is how a tree gets scanned at a glance.

The terminal-theme catalog scales precisely *because* it is community-contributable; the file-tree icon set is on the opposite trajectory.

## Goals

- A user can switch the File Tree icon theme in **Settings → Appearance** without restarting Warp.
- The theme format is documented so a community PR adding a third theme is mechanical: drop a JSON file plus its referenced SVGs under the bundled assets directory and add an entry to the theme registry.
- A user picking the `default` theme sees the same icons they see today, byte-equivalent, on day one of this feature shipping. Zero visual regression for existing users.
- The theme system survives a missing or malformed theme file: a fallback to a generic file/folder icon, plus a settings notification, never a panic or an empty file tree.
- The lookup order is predictable and matches what every other file-tree theme system converges on: exact filename → extension → folder name (with separate open/closed states) → generic fallback.

## Non-goals (V1 — explicitly deferred to follow-ups)

- **User-supplied theme files loaded from disk.** The reporter explicitly suggests deferring this; the V1 catalog is the in-binary bundled set. A follow-up can add a `~/.warp/file_tree_icon_themes/<name>.json` discovery path.
- **In-app theme editor.** Theme files are read-only artifacts; users edit JSON in their editor of choice (only relevant once user-supplied themes land).
- **Per-file user overrides.** No "always use this icon for this specific file" affordance.
- **Theme-driven recoloring beyond what the theme file declares.** Themes map paths → icon assets; they do not modify icon tints, sizes, or other rendering parameters at runtime.
- **Language-ID-based resolution** (e.g. mapping a Tree-sitter language ID to an icon). The V1 lookup uses filename and extension only; language ID adds a second source of truth without a clear win for icon resolution. Tracked as a follow-up.
- **A generalized extension API.** This feature is the icon-theme slice only — no plugin system, no theme marketplace, no theme auto-fetch.
- **SVG-vs-NerdFont format choice as a user-facing toggle.** V1 is SVG-only (matching the current bundled assets). Nerd Font codepoint maps are a separate, additive feature.

## User experience

### Picking a theme

1. User opens **Settings → Appearance**. A new **File Tree icon theme** picker sits beneath the existing terminal theme controls.
2. The picker is a dropdown listing the bundled themes by display name (`Default`, `Material`). The currently selected theme is checked.
3. Selecting a theme re-renders the file tree immediately. No restart, no reload.
4. The picker has a small *"How to add a theme"* link beneath it that opens `docs/file-tree-icon-themes.md` (shipping in the same release, see the Open questions section). The link is informational; users do not need to follow it to use the feature.

### File tree rendering with a theme active

1. The user opens a workspace. The Project Explorer renders each row's icon by resolving the row's path against the active theme.
2. The lookup order, deterministic, first match wins:
   1. **Exact filename.** `.gitignore`, `Dockerfile`, `package.json`, `Cargo.toml`, `README.md`. Theme entries listed by exact name take precedence over their extension.
   2. **Extension.** `foo.rs` → the `rs` icon if the theme defines one. The leading dot is stripped before lookup.
   3. **Folder name** (folders only). `node_modules`, `.git`, `src`, `dist`. The theme can declare separate icons for open and closed folder states; if it omits one, that state falls back to the generic folder icon for that theme.
   4. **Generic fallback.** Theme-declared `file` icon for files, `folder` icon (or `folderExpanded`) for folders. If even those are missing — pathological — the built-in `Icon::File`/`Icon::Folder` glyphs surface, matching today's behavior for unmapped extensions.
3. Switching themes does not require any cached state to be invalidated: the resolution function is a pure mapping from path to bundled SVG path, called per row at render time.

### Misconfiguration scenarios

1. **Setting points at a non-existent theme** (e.g. user hand-edits TOML to `icon_theme = "ridiculous"`): Warp surfaces a settings-error notification *"File tree icon theme `ridiculous` is not bundled. Falling back to `default`."* with a button that opens settings, and renders with `default`. The error does not block other settings from loading.
2. **A bundled theme JSON fails to parse** (catastrophic — an internal bug, not a user-facing scenario): Warp logs the parse error and treats the theme as if it weren't bundled. Falls back to `default`. CI's `cargo test` catches this before release because each bundled theme is parsed in unit tests.
3. **A theme references an SVG asset path that doesn't exist in the bundle.** Resolution treats that lookup as "no entry"; the next step in the lookup order runs (extension → folder → fallback). Logged at `warn!` level so the discrepancy surfaces in Warp logs without spamming the UI.

## Configuration shape

The setting lives under the existing **Appearance** group. The Rust setting carries a string and the registry validates membership at read time:

```toml
[appearance]
file_tree_icon_theme = "default"
```

Values: any theme `id` registered in the bundled catalog. V1 catalog: `"default"`, `"material"`. Defaulting to `"default"` preserves today's behavior unconditionally.

The theme JSON format (read-only artifact bundled at `bundled/file_tree_icon_themes/<id>.json`):

```json
{
  "id": "material",
  "displayName": "Material",
  "iconDefinitions": {
    "rs":         { "iconPath": "bundled/svg/file_type/material/rust.svg" },
    "json":       { "iconPath": "bundled/svg/file_type/material/json.svg" },
    "ts":         { "iconPath": "bundled/svg/file_type/material/typescript.svg" },
    "py":         { "iconPath": "bundled/svg/file_type/material/python.svg" },
    "default_file":   { "iconPath": "bundled/svg/file_type/material/_file.svg" },
    "default_folder": { "iconPath": "bundled/svg/file_type/material/_folder.svg" },
    "default_folder_expanded": { "iconPath": "bundled/svg/file_type/material/_folder_open.svg" },
    "git":            { "iconPath": "bundled/svg/file_type/material/_folder_git.svg" },
    "node_modules":   { "iconPath": "bundled/svg/file_type/material/_folder_node.svg" }
  },
  "fileExtensions": {
    "rs": "rs",
    "ts": "ts",
    "tsx": "ts",
    "js": "ts",
    "jsx": "ts",
    "py": "py",
    "json": "json"
  },
  "fileNames": {
    ".gitignore":   "git",
    "Dockerfile":   "docker",
    "Cargo.toml":   "rs",
    "package.json": "json"
  },
  "folderNames": {
    ".git":         "git",
    "node_modules": "node_modules"
  },
  "folderNamesExpanded": {
    ".git":         "git",
    "node_modules": "node_modules"
  },
  "file":            "default_file",
  "folder":          "default_folder",
  "folderExpanded":  "default_folder_expanded"
}
```

Field reference:

| Field | Required | Notes |
|---|---|---|
| `id` | yes | Stable identifier referenced from settings TOML. Unique across the bundled catalog. |
| `displayName` | yes | Shown in the Settings dropdown. |
| `iconDefinitions` | yes | Map of icon-key → bundled SVG path. Keys are arbitrary strings used by the maps below. |
| `fileExtensions` | no | `extension (no leading dot) → iconDefinitions key`. |
| `fileNames` | no | `exact filename → iconDefinitions key`. Wins over `fileExtensions`. |
| `folderNames` | no | `folder name → iconDefinitions key` for collapsed folders. |
| `folderNamesExpanded` | no | `folder name → iconDefinitions key` for expanded folders. Falls back to `folderNames` if absent. |
| `file` | no | Default icon for files with no extension/name match. Defaults to a built-in generic if missing. |
| `folder` | no | Default folder icon. |
| `folderExpanded` | no | Default expanded-folder icon. Falls back to `folder` if absent. |

## Testable behavior invariants

Numbered list — each maps to a verification path in the tech spec:

1. With `appearance.file_tree_icon_theme = "default"`, the icon shown for every file currently mapped in `app/src/code/icon.rs` is byte-equivalent to today's icon. (Zero visual regression on the default theme.)
2. With `appearance.file_tree_icon_theme = "material"`, the icon shown for every file *with a `material`-theme entry* differs from the `default` theme; for files *without* an entry, the theme's `file`/`folder` fallback is used.
3. Switching the theme via Settings UI re-renders the file tree within 1 frame (no observable flash, no scroll-position jump).
4. For a path `foo/bar/Dockerfile`, lookup order resolves to the `Dockerfile` filename entry, not the (non-existent) `Dockerfile` extension, regardless of which theme is active.
5. For a path `foo/bar/script.sh` where the theme has no `sh` extension entry but does have a `default_file`, the `default_file` icon renders. If the theme has neither, the built-in `Icon::File` renders.
6. For a folder `node_modules` that the theme declares in `folderNames` but not `folderNamesExpanded`, the closed-state icon renders both when collapsed and expanded.
7. Setting `file_tree_icon_theme` to a string that is not in the bundled catalog falls back to `default` at read time and surfaces a settings-error notification with a button to open Settings → Appearance.
8. A bundled theme whose JSON references an `iconPath` that does not exist in the bundle falls through to the next lookup step (extension → folder → fallback) without panicking and logs a `warn!`.
9. Rendering the file tree on a workspace with 1,000 files completes within the same frame budget as today (the per-row lookup is O(1) on the theme's HashMaps, not O(n) on the theme's entry count).
10. The bundled theme catalog is exercised at `cargo test` time: every bundled theme JSON parses and every `iconPath` it references resolves to an actual asset under `bundled/`.

## Open questions

- **Should `docs/file-tree-icon-themes.md` ship in the same PR or as a follow-up?** Recommend same release, separate PR. Documentation is in scope for the same release as the feature gate but does not need to block the spec.
- **Material theme licensing.** The Material Icon Theme is MIT-licensed (PKief/vscode-material-icon-theme). Confirm the Warp legal stance on bundling the SVG set with attribution. If bundling is blocked, the V1 alternative theme can be a Warp-original Seti-inspired set instead — same shape, different art.
- **Should the `material` theme cover every extension `default` covers?** Recommend yes, even when the icon would be visually similar — it gives community contributors a clean template (one entry per extension) and avoids surprise fall-throughs.
- **TOML key location.** `appearance.file_tree_icon_theme` is the most natural placement (sits next to terminal theme controls). Confirm with maintainers whether the appropriate Rust setting group is `AppearanceSettings` or a new `FileTreeSettings` group.
