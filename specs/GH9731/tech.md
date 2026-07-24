# Tech Spec: File Tree icon themes

**Issue:** [warpdotdev/warp#9731](https://github.com/warpdotdev/warp/issues/9731)

## Context

Today's File Tree icon mapping lives in [`app/src/code/icon.rs`](https://github.com/warpdotdev/warp/blob/master/app/src/code/icon.rs) as a closed `match extension { ... }` table. Roughly 22 file extensions are hardcoded; everything else returns `None` and the caller falls back to `Icon::File`. There is no folder-level mapping (the file tree always renders `Icon::Folder` for any folder).

The function's only call site is `app/src/code/file_tree/view/render.rs`, which renders one row per visible file/folder in the Project Explorer. Because each render call invokes `icon_from_file_path` per row, the per-call work must stay O(1) — a parsed theme cached at startup and indexed by `HashMap` keeps the hot path fast.

### Relevant code

| Path | Role |
|---|---|
| `app/src/code/icon.rs` | The closed `match` to be replaced by the theme registry. The function signature `icon_from_file_path(path: &str, appearance: &Appearance) -> Option<Box<dyn Element>>` is preserved to keep the call site untouched. |
| `app/src/code/file_tree/view/render.rs` | Sole call site. Calls `icon_from_file_path` per row; falls back to `Icon::File`/`Icon::Folder` on `None`. No changes required to this file. |
| `app/src/settings/code.rs` (or `appearance.rs` if it exists) | `define_settings_group!` macro for settings. The new `FileTreeIconTheme` setting plugs in here. |
| `app/src/code/mod.rs` | The `code` module root. The new `file_tree_icon_themes` registry module is added here. |
| `bundled/svg/file_type/` | Existing bundled SVGs the `default` theme will reference verbatim. |
| `bundled/file_tree_icon_themes/` (new) | Bundled JSON theme files. `default.json` and `material.json` for V1. |
| `bundled/svg/file_type/material/` (new) | The `material` theme's SVGs. |

### Related closed PRs and issues

- #9731 — the feature request itself, with reporter-supplied scoping that this spec follows.
- (None of my open PRs interact with this surface.) The icon-theme work is orthogonal to LSP, CLI agents, bootstrap scripts, and worktree-marker stripping.

## Crate boundaries

The theme registry is leaf-level UI code that depends only on serde + the existing `warpui` element types. It lives in `app/src/code/file_tree_icon_themes.rs` (a new module under the existing `code` namespace). No new crate is needed; no existing crate's dependency direction changes.

## Proposed changes

### 1. New theme model types

**File:** new `app/src/code/file_tree_icon_themes/model.rs`.

```rust
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IconThemeFile {
    pub id: String,
    pub display_name: String,
    pub icon_definitions: HashMap<String, IconDefinition>,
    #[serde(default)]
    pub file_extensions: HashMap<String, String>,
    #[serde(default)]
    pub file_names: HashMap<String, String>,
    #[serde(default)]
    pub folder_names: HashMap<String, String>,
    #[serde(default)]
    pub folder_names_expanded: HashMap<String, String>,
    #[serde(default)]
    pub file: Option<String>,
    #[serde(default)]
    pub folder: Option<String>,
    #[serde(default)]
    pub folder_expanded: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IconDefinition {
    pub icon_path: String,
}
```

`#[serde(rename_all = "camelCase")]` matches the on-disk JSON format documented in `product.md`.

### 2. Theme registry

**File:** new `app/src/code/file_tree_icon_themes/registry.rs`.

```rust
pub struct IconTheme {
    file: IconThemeFile,
}

impl IconTheme {
    /// Resolve a path to a bundled SVG path, applying the documented lookup order.
    /// Returns `None` if no entry resolves; the caller falls back to the generic Icon.
    pub fn resolve(&self, path: &Path, is_folder: bool, is_expanded: bool) -> Option<&str> {
        let name = path.file_name()?.to_str()?;

        // 1. Folder name (folders only)
        if is_folder {
            let folder_map = if is_expanded
                && !self.file.folder_names_expanded.is_empty()
            {
                &self.file.folder_names_expanded
            } else {
                &self.file.folder_names
            };
            if let Some(key) = folder_map.get(name) {
                if let Some(def) = self.file.icon_definitions.get(key) {
                    return Some(def.icon_path.as_str());
                }
            }
            // Folder default
            let default_key = if is_expanded {
                self.file.folder_expanded.as_ref().or(self.file.folder.as_ref())
            } else {
                self.file.folder.as_ref()
            };
            return default_key
                .and_then(|k| self.file.icon_definitions.get(k))
                .map(|def| def.icon_path.as_str());
        }

        // 2. Exact filename
        if let Some(key) = self.file.file_names.get(name) {
            if let Some(def) = self.file.icon_definitions.get(key) {
                return Some(def.icon_path.as_str());
            }
        }

        // 3. Extension (leading dot stripped by Path::extension)
        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            if let Some(key) = self.file.file_extensions.get(ext) {
                if let Some(def) = self.file.icon_definitions.get(key) {
                    return Some(def.icon_path.as_str());
                }
            }
        }

        // 4. File default
        self.file
            .file
            .as_ref()
            .and_then(|k| self.file.icon_definitions.get(k))
            .map(|def| def.icon_path.as_str())
    }
}

pub struct IconThemeRegistry {
    themes: HashMap<String, IconTheme>,
}

impl IconThemeRegistry {
    /// Load all bundled themes at startup. Themes that fail to parse are
    /// logged at `error!` and skipped — they don't block other themes.
    pub fn from_bundled() -> Self { ... }

    pub fn get(&self, id: &str) -> Option<&IconTheme> {
        self.themes.get(id)
    }

    pub fn list(&self) -> impl Iterator<Item = (&str, &str)> {
        // (id, display_name) pairs for the Settings UI dropdown
        self.themes.values().map(|t| (t.file.id.as_str(), t.file.display_name.as_str()))
    }
}
```

The registry is constructed once at app startup (e.g. inside the same `LazyLock` site that hosts other appearance-related caches) and stored as a singleton. Every per-row call to `icon_from_file_path` performs at most four `HashMap::get` calls.

### 3. Replace `icon_from_file_path`

**File:** `app/src/code/icon.rs`.

The signature is preserved to keep the existing call site untouched. The body becomes:

```rust
pub fn icon_from_file_path(path: &str, appearance: &Appearance) -> Option<Box<dyn Element>> {
    let theme_id = AppearanceSettings::handle().file_tree_icon_theme();
    let theme = ICON_THEME_REGISTRY.get(&theme_id)
        .or_else(|| ICON_THEME_REGISTRY.get(DEFAULT_THEME_ID))?;

    let parsed_path = Path::new(path);
    // The render call site only invokes this for files; folder rendering takes
    // a separate path (see render.rs:53). For V1, is_folder=false here.
    // Folder support is added in a sibling change to render.rs that passes
    // is_folder/is_expanded explicitly — see Section 4.
    let asset_path = theme.resolve(parsed_path, false, false)?;

    let theme_colors = appearance.theme();
    let element: Box<dyn Element> = if asset_path.ends_with(".svg") && needs_tinting(asset_path) {
        Icon::new(
            asset_path,
            theme_colors.main_text_color(theme_colors.background()).into_solid(),
        )
        .finish()
    } else {
        Image::new(
            AssetSource::Bundled { path: asset_path },
            CacheOption::BySize,
        )
        .finish()
    };
    Some(element)
}
```

Today's `icon.rs` mixes `Image::new` (untinted, full-color SVGs) and `Icon::new` (tinted, single-color glyphs — currently used for `md` and `sh` only). The theme JSON does not yet distinguish these; for V1, `needs_tinting` is a small static set listing the keys whose SVGs are single-color glyphs intended for theme-color tinting. Folding this into the theme JSON as a per-icon `tint: true` flag is a follow-up.

### 4. Folder rendering pass

**File:** `app/src/code/file_tree/view/render.rs`.

Today, folders are rendered with a hardcoded `Icon::Folder` at line 53. To honor `folderNames` and `folder`/`folderExpanded` from the theme, the folder branch picks up a sibling helper:

```rust
fn folder_icon(&self, is_expanded: bool) -> ImageOrIcon {
    let theme_id = AppearanceSettings::handle().file_tree_icon_theme();
    let theme = match ICON_THEME_REGISTRY.get(&theme_id) {
        Some(t) => t,
        None => return ImageOrIcon::Icon(Icon::Folder),
    };
    match theme.resolve(Path::new(&self.path), true, is_expanded) {
        Some(svg) => ImageOrIcon::Image(Image::new(
            AssetSource::Bundled { path: svg },
            CacheOption::BySize,
        ).finish()),
        None => ImageOrIcon::Icon(Icon::Folder),
    }
}
```

The folder render branch swaps `ImageOrIcon::Icon(Icon::Folder)` for `self.folder_icon(is_expanded)`. The `is_expanded` value comes from the existing tree-state model (find via `grep -rn "expanded" app/src/code/file_tree/`).

### 5. Settings entry

**File:** the existing **Appearance** settings group (verify exact file at implementation time — likely `app/src/settings/appearance.rs` or grouped under `code.rs`). Add a new setting via the existing `define_settings_group!` macro:

```rust
file_tree_icon_theme: FileTreeIconTheme {
    type: String,
    default: "default".to_owned(),
    supported_platforms: SupportedPlatforms::ALL,
    sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
    private: false,
    toml_path: "appearance.file_tree_icon_theme",
    description: "File Tree icon theme. Bundled options: default, material.",
},
```

A small validator runs at read time: if the stored value is not in `ICON_THEME_REGISTRY.list()`, it surfaces a settings-error notification (existing pattern in `app/src/settings/initializer.rs`) and returns `"default"`. This satisfies invariant 7.

### 6. Settings UI dropdown

**File:** the existing **Settings → Appearance** view (locate via `grep -rn "appearance" app/src/settings/view/` or equivalent).

Render a labeled dropdown sourced from `ICON_THEME_REGISTRY.list()`. Selection writes the chosen theme's `id` to the setting; the file tree re-renders on the next frame because the render path reads the setting directly each call (the registry handles change propagation through the existing settings observer pattern — no new infrastructure).

### 7. Bundled theme catalog

**Files:** new `bundled/file_tree_icon_themes/default.json` and `bundled/file_tree_icon_themes/material.json`.

`default.json` reproduces today's mapping byte-equivalent. The keys map directly from the existing `match extension` arms:

```json
{
  "id": "default",
  "displayName": "Default",
  "iconDefinitions": {
    "rs":         { "iconPath": "bundled/svg/file_type/rust.svg" },
    "json":       { "iconPath": "bundled/svg/file_type/json.svg" },
    "ts":         { "iconPath": "bundled/svg/file_type/typescript.svg" },
    "py":         { "iconPath": "bundled/svg/file_type/python.svg" },
    "cpp":        { "iconPath": "bundled/svg/file_type/cpp.svg" },
    "go":         { "iconPath": "bundled/svg/file_type/go.svg" },
    "md":         { "iconPath": "bundled/svg/file_type/markdown.svg" },
    "sh":         { "iconPath": "bundled/svg/terminal.svg" },
    "kotlin":     { "iconPath": "bundled/svg/file_type/kotlin.svg" },
    "php":        { "iconPath": "bundled/svg/file_type/php.svg" },
    "perl":       { "iconPath": "bundled/svg/file_type/perl.svg" },
    "c":          { "iconPath": "bundled/svg/file_type/c.svg" },
    "cython":     { "iconPath": "bundled/svg/file_type/cython.svg" },
    "flash":      { "iconPath": "bundled/svg/file_type/flash.svg" },
    "wasm":       { "iconPath": "bundled/svg/file_type/wasm.svg" },
    "zig":        { "iconPath": "bundled/svg/file_type/zig.svg" },
    "sql":        { "iconPath": "bundled/svg/file_type/sql.svg" },
    "angular":    { "iconPath": "bundled/svg/file_type/angular.svg" },
    "terraform":  { "iconPath": "bundled/svg/file_type/terraform.svg" }
  },
  "fileExtensions": {
    "rs": "rs",
    "json": "json",
    "ts": "ts", "tsx": "ts",
    "js": "ts", "jsx": "ts",
    "py": "py",
    "cpp": "cpp", "hpp": "cpp",
    "go": "go",
    "md": "md",
    "sh": "sh",
    "kt": "kotlin", "kts": "kotlin",
    "php": "php",
    "pl": "perl", "pm": "perl",
    "c": "c", "h": "c",
    "pyx": "cython", "pxd": "cython",
    "swf": "flash",
    "wasm": "wasm",
    "zig": "zig",
    "sql": "sql",
    "ng": "angular", "ngml": "angular",
    "tf": "terraform", "hcl": "terraform", "tfvars": "terraform"
  }
}
```

Wait — note the `js`/`jsx`/`ts`/`tsx` collision. Today's code maps both `ts/tsx` AND `js/jsx` to the TypeScript icon (existing behavior, possibly a typo in the original or intentional — the mapping in `icon.rs:38` literally says `Some("js") | Some("jsx") => ...javascript.svg...`). Re-read `icon.rs` at implementation time and reproduce the *actual* current mapping byte-equivalent — this spec's example is illustrative.

`material.json` provides a parallel set of icons under `bundled/svg/file_type/material/` with the same keys, plus folder entries and a default file/folder.

### 8. Documentation

A short `docs/file-tree-icon-themes.md` describes the theme JSON format and the bundled catalog. Out of scope for the spec's core feature gate per the open question; recommend shipping it alongside the implementation PR.

## Testing and validation

Each invariant from `product.md` maps to a test at this layer:

| Invariant | Test layer | File |
|---|---|---|
| 1 (default theme byte-equivalent) | unit | `app/src/code/icon_tests.rs` (extending today's tests) — for each extension in today's `match`, assert the asset path the new pipeline returns matches the path today's match returns. Brittle by design: any drift in `default.json` fails the test. |
| 2 (material theme differs / falls back when absent) | unit | `app/src/code/file_tree_icon_themes/registry_tests.rs` (new) — load `material.json`, resolve a known-mapped path, assert the returned asset path is under `bundled/svg/file_type/material/`. Resolve an unmapped path, assert the `default_file` entry is returned. |
| 3 (theme switch re-renders within one frame) | integration | UI integration test under `crates/integration/`. Assert no observable flash/jump after switching. |
| 4 (filename wins over extension) | unit | registry_tests — `Dockerfile` resolves to its filename entry, not to a `Dockerfile`-extension entry. |
| 5 (extension fallback → file default → built-in) | unit | registry_tests — three test cases at each fallback step. |
| 6 (folder collapsed-state used when expanded missing) | unit | registry_tests — theme with `folderNames` only, no `folderNamesExpanded`. |
| 7 (invalid theme id falls back + notifies) | unit | settings test — set TOML `file_tree_icon_theme = "ridiculous"`, assert read returns `"default"` and an error event was dispatched. |
| 8 (missing `iconPath` falls through, no panic) | unit | registry_tests — theme with a definition pointing at a non-existent asset, assert resolution falls through to the next step and the call doesn't panic. Capture `warn!` log line via tracing capture. |
| 9 (per-row lookup is O(1)) | bench (informal) | manual benchmark with 1,000 rows; not gated in CI but documented in PR description. |
| 10 (catalog parse + asset existence at test time) | unit | new `bundled_themes_test.rs` — load every JSON under `bundled/file_tree_icon_themes/`, parse, walk every `iconPath`, assert each path resolves to an actual file in the bundle. |

### Cross-platform constraints

- Theme JSON paths use forward slashes; the bundled-asset layer already normalizes for Windows.
- `Path::file_name()` and `Path::extension()` handle Windows paths correctly via `std::path` — no extra work.
- SVG rendering through `Image::new(AssetSource::Bundled { ... })` is the existing infrastructure; no platform-specific work.

## End-to-end flow

```
User opens Settings → Appearance
  └─> [appearance_view::file_tree_icon_picker]               (settings UI, new dropdown)
        ├─> source list from ICON_THEME_REGISTRY.list()
        └─> on selection → AppearanceSettings.set_file_tree_icon_theme(id)

User opens a workspace; file tree renders
  └─> [file_tree::view::render::row]                         (existing render path)
        └─> for each row:
              ├─> if folder → folder_icon(is_expanded)        (new helper)
              │     └─> ICON_THEME_REGISTRY.get(theme_id)
              │           └─> theme.resolve(path, true, is_expanded)
              │                 ├─> folder_names_expanded if expanded → iconDefinitions[key]
              │                 ├─> folder_names → iconDefinitions[key]
              │                 ├─> folderExpanded default → iconDefinitions[key]
              │                 ├─> folder default → iconDefinitions[key]
              │                 └─> None → caller renders Icon::Folder
              └─> if file → icon_from_file_path(path, appearance)  (existing call site)
                    └─> ICON_THEME_REGISTRY.get(theme_id)
                          └─> theme.resolve(path, false, false)
                                ├─> file_names → iconDefinitions[key]
                                ├─> file_extensions → iconDefinitions[key]
                                ├─> file default → iconDefinitions[key]
                                └─> None → caller renders Icon::File

User edits TOML to set file_tree_icon_theme = "ridiculous"
  └─> [AppearanceSettings::reload]
        ├─> validator: "ridiculous" not in ICON_THEME_REGISTRY → fall back to "default"
        ├─> emit settings-error notification
        └─> file tree re-renders with default theme
```

## Risks

- **Hidden visual regression for the `default` theme.** Today's `match` is a hand-written closed enum; the `default.json` is a JSON transcription. A typo in the transcription would silently change icons for existing users. **Mitigation:** invariant 1's test asserts byte-equivalence for every existing arm. CI catches transcription bugs before release.
- **Tinting behavior split.** Today's code uses `Icon::new` (tinted) for `md` and `sh` and `Image::new` (untinted) for everything else. The theme JSON has no tint flag in V1; the V1 implementation hard-codes the tint set. If a contributor adds a new tinted asset to a theme, they have to also add the icon key to `needs_tinting`. **Mitigation:** document this in `docs/file-tree-icon-themes.md` and track adding a per-icon `tint: true` flag as a follow-up.
- **Theme JSON drift between bundled themes.** If `default.json` and `material.json` cover different extension sets, a user switching themes sees more or fewer icons. **Mitigation:** invariant 10's test plus a pre-commit lint that diffs the two themes' extension keys and warns (not errors) on mismatches. A small theme can intentionally cover fewer keys; the warning surfaces the choice.
- **Material theme licensing.** Bundling Material Icon Theme SVGs requires legal review. **Mitigation:** the `material` theme can ship as Warp-original art if licensing is blocked. The theme system itself is unaffected.
- **Initial theme load latency.** Parsing two JSON files at startup is negligible (microseconds), but if the bundled catalog grows, parsing all themes upfront becomes wasteful. **Mitigation:** lazy parse — the registry holds raw JSON bytes per theme `id` and parses on first `get()`. V1 catalog is small enough that eager parsing is fine; this is a forward-looking note.

## Follow-ups (out of this spec)

- User-supplied theme files loaded from `~/.warp/file_tree_icon_themes/<id>.json` (the issue's primary "community-contributable" angle, deferred per reporter scoping).
- Per-icon `tint: true` flag in the theme JSON, removing the V1 hard-coded tint set.
- Language-ID resolution as a third lookup step (Tree-sitter language ID → icon).
- Nerd Font codepoint support as an alternative theme format (the issue's "lower-lift" alternative for V1).
- In-app theme editor / preview pane.
- Theme marketplace / GitHub-hosted theme catalog auto-discovery.
- A documentation page for the SVG asset constraints (sizes, viewBox conventions, tinting expectations).
