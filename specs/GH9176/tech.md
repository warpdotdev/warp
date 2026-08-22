# Tech Spec: Tab Configs in Command Palette and keyboard shortcuts

**Issue:** [warpdotdev/warp#9176](https://github.com/warpdotdev/warp/issues/9176)

## Context

Warp's Command Palette infrastructure ([`app/src/search/command_palette/`](https://github.com/warpdotdev/warp/blob/master/app/src/search/command_palette/)) is composed of pluggable result sources, one per kind of selectable item (Launch Configurations, MCP servers, plus the static slash-command set). Each source supplies a data-source struct, a search-item type, and a renderer.

Adding Tab Configs is mechanical: clone the Launch Configuration source's three files (`data_source.rs`, `search_item.rs`, `renderer.rs`), point them at `app/src/tab_configs/` instead of `app/src/launch_configs/`, register the new source in the palette mixer.

### Relevant code

| Path | Role |
|---|---|
| `app/src/search/command_palette/launch_config/{mod,data_source,search_item,renderer}.rs` | Existing Launch Configuration palette source. The new `tab_config/` sibling will mirror this structure. |
| `app/src/search/command_palette/data_sources.rs` | Aggregator that registers all palette sources. The new source is added here. |
| `app/src/search/command_palette/mixer.rs` | Result ranking + de-duplication. No changes needed if the new source slots in cleanly. |
| `app/src/tab_configs/tab_config.rs` | `TabConfig` struct (existing). The palette source reads from the same on-disk files. |
| `app/src/tab_configs/mod.rs` | Tab Config module entry. The new source can call into existing helpers (loaders, parameter resolution) so the launch path is byte-equivalent to the `+`-button flow. |
| `app/src/server/telemetry/events.rs` | Telemetry. New `TabConfigPaletteSelected` event added. |
| `app/src/search/command_palette/launch_config/mod.rs` | The launch-path that fires when a Launch Configuration row is selected. The Tab Config equivalent reuses the existing Tab-Config-launch entry point. |

### Related closed PRs and issues

- The MCP-server palette source (added at some point in master) is the most recent example of adding a new result kind. Its commit history is the reference for how a new source plugs into the mixer.

## Crate boundaries

All new code lives in `app/`. No new crate, no cross-crate boundaries to manage. The Tab Config TOML loader already lives in the same crate; the palette source consumes it as an internal module dependency.

## Proposed changes

### 1. New palette source module

**Files:** new directory `app/src/search/command_palette/tab_config/`, mirroring the existing `launch_config/` siblings:

```
app/src/search/command_palette/tab_config/
├── mod.rs           — module entry, exports the data source
├── data_source.rs   — enumerates Tab Config files, exposes results to the palette
├── search_item.rs   — TabConfigSearchItem + matchable strings
└── renderer.rs      — row rendering (icon, title, subtitle)
```

The skeleton follows `launch_config/`'s structure 1:1. Implementation outline:

```rust
// data_source.rs
pub struct TabConfigDataSource;

impl TabConfigDataSource {
    /// Walk ~/.warp/tab_configs/, parse each .toml file, yield search items.
    /// Errors per file are surfaced as error-state items rather than silently
    /// dropped (invariant 5).
    pub fn results(ctx: &AppContext) -> Vec<TabConfigSearchItem> {
        let dir = warp_managed_paths::tab_configs_dir(); // existing helper
        let entries = std::fs::read_dir(&dir).ok().into_iter().flatten();
        entries
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().is_some_and(|x| x == "toml"))
            .map(|e| {
                let path = e.path();
                match TabConfig::load_from_path(&path) {
                    Ok(config) => TabConfigSearchItem::Loaded { config, path },
                    Err(err) => TabConfigSearchItem::Failed {
                        path,
                        reason: err.to_string(),
                    },
                }
            })
            .collect()
    }
}

// search_item.rs
pub enum TabConfigSearchItem {
    Loaded { config: TabConfig, path: PathBuf },
    Failed { path: PathBuf, reason: String },
}

impl SearchItem for TabConfigSearchItem {
    fn match_against(&self) -> &str {
        match self {
            Self::Loaded { config, .. } => config.name.as_str(),
            Self::Failed { path, .. } => {
                path.file_stem().and_then(|s| s.to_str()).unwrap_or("")
            }
        }
    }
    // …rest mirrors LaunchConfigSearchItem.
}
```

`TabConfig::load_from_path` is the existing loader (verify at implementation time; the function might be in `app/src/tab_configs/mod.rs`'s public surface).

### 2. Register the source in the mixer

**File:** `app/src/search/command_palette/data_sources.rs`.

The existing aggregator is a small list of constructor calls. Adding the Tab Config source is a one-line append. The mixer at `mixer.rs` doesn't need changes — the new source's items participate in the existing relevance ranking without any new code.

### 3. Selection dispatch

**File:** the existing palette dispatch logic — locate via `grep -rn "LaunchConfigSearchItem.*select\|on_palette_select" app/src/search/command_palette/`.

Two new arms:

```rust
// Healthy Tab Config: launch via the same entry point the + button uses.
SearchResult::TabConfig(TabConfigSearchItem::Loaded { config, .. }) => {
    crate::tab_configs::session_config_modal::launch_with_config(
        ctx, config,
    );
}
// Errored Tab Config: open the offending file in Warp's editor.
SearchResult::TabConfig(TabConfigSearchItem::Failed { path, .. }) => {
    crate::code::editor::open_file(ctx, path);
}
```

`launch_with_config` is the existing entry point used by the `+`-button path; using it byte-for-byte satisfies invariant 2 (palette launch behaves identically to button launch). `open_file` is the existing Warp editor entry point used by `/open-file`.

### 4. Iconography

**File:** `app/src/search/command_palette/tab_config/renderer.rs`.

Pick a row icon distinguishable from the Launch Configuration row icon. Candidates from the existing bundle (verify with `ls bundled/svg/`): `bundled/svg/tab.svg` or `bundled/svg/grid.svg`. The renderer is otherwise a copy of `launch_config/renderer.rs` with the icon path swapped.

### 5. Telemetry

**File:** `app/src/server/telemetry/events.rs`.

Add a new variant:

```rust
TabConfigPaletteSelected {
    config_name: String,
    /// True when the selected entry was an error-state item (the user
    /// clicked through to the editor rather than launching).
    was_error_state: bool,
},
```

Emit from the dispatch arm above. Keep the event distinct from `LaunchConfigPaletteSelected` so adoption metrics for the new surface are clean (open question 2 in product.md).

### 6. Reactive refresh

**File:** the new `data_source.rs`.

The Command Palette already invokes `results(ctx)` at every palette-open (no caching across sessions). Renaming or deleting a Tab Config TOML file is reflected on the next palette open without any explicit invalidation work. Invariant 6 holds without additional plumbing.

If profiling later reveals the directory walk is too slow for large catalogs (unlikely — Tab Configs typically number in single digits), a simple in-memory cache invalidated by a `~/.warp/tab_configs/` filesystem watcher is a follow-up. V1 does the filesystem walk every palette open — explicit, simple, and demonstrably correct.

## Testing and validation

Each invariant from `product.md` maps to a test at this layer:

| Invariant | Test layer | File |
|---|---|---|
| 1 (palette shows Tab Config) | unit | new `app/src/search/command_palette/tab_config/data_source_tests.rs` — write a fixture Tab Config TOML, call `TabConfigDataSource::results`, assert the returned vec contains the loaded item. |
| 2 (selection fires same launch path as `+` button) | unit | data_source_tests + a small integration assertion that the dispatch arm calls `launch_with_config`. Use a mock or capture closure. |
| 3 (mixed Tab Configs + Launch Configurations) | integration | extend the existing palette mixer integration test to register both sources and assert both kinds appear when their names match. |
| 4 (icon distinguishability) | unit | renderer_tests.rs — render a `TabConfigSearchItem`, assert the icon path is the new icon and not the Launch Configuration icon. |
| 5 (error-state row + editor open) | unit | data_source_tests — write a malformed TOML, assert the source returns a `Failed` variant with the parse error string. dispatch_tests — selecting a `Failed` item invokes `open_file`. |
| 6 (file rename/delete reflected) | unit | data_source_tests — call `results()`, rename a fixture file, call `results()` again, assert the new name appears and the old name doesn't. |
| 7 (Cmd+Ctrl+L unchanged) | integration | existing keybinding test for `Cmd+Ctrl+L` — assert it still fires the Launch Configuration handler, not a Tab Config one. |
| 8 (telemetry event distinct) | unit | dispatch_tests — assert `TabConfigPaletteSelected` is emitted with the expected payload, and `LaunchConfigPaletteSelected` is not. |

### Cross-platform constraints

- Tab Configs live under `~/.warp/tab_configs/` on every platform; the existing `warp_managed_paths::tab_configs_dir()` (verify name at implementation time) is the canonical accessor and handles platform differences.
- TOML parsing is byte-identical across platforms.
- No process spawning, no shell interaction, no platform-specific path quirks.

## End-to-end flow

```
User presses Cmd+P
  └─> Command Palette opens                                  (existing surface)
        └─> mixer queries every registered data source       (existing)
              ├─> LaunchConfigDataSource::results            (existing)
              ├─> TabConfigDataSource::results               (new)
              │     ├─> read_dir(~/.warp/tab_configs/)
              │     ├─> for each .toml: TabConfig::load_from_path
              │     │     ├─> Ok → TabConfigSearchItem::Loaded
              │     │     └─> Err → TabConfigSearchItem::Failed (carries reason)
              │     └─> return Vec<TabConfigSearchItem>
              └─> rank by relevance                          (existing mixer)

User types "tab config" / partial name
  └─> palette filters (existing)

User selects a Tab Config row
  └─> dispatch arm                                           (new)
        ├─> Loaded → launch_with_config(ctx, &config)        (existing tab_configs entry)
        │     └─> opens parameters modal (if any), spawns panes
        └─> Failed → open_file(ctx, path)                    (existing editor entry)

Telemetry
  └─> TabConfigPaletteSelected { config_name, was_error_state } (new)
```

## Risks

- **Visual ambiguity between Launch Configuration and Tab Config rows.** If the row icons aren't sufficiently distinguishable, users will mis-select. **Mitigation:** invariant 4's test asserts icon distinguishability; the icon choice is reviewed with maintainers (open question in product.md).
- **Parse-error surfacing UX.** A user who's actively editing a Tab Config TOML may have it transiently malformed. The error-state row is intentional (better than silent drop) but if the user opens the palette mid-edit they'll see an alarming "Failed to load" row. **Mitigation:** the row's "Open in editor" action is the natural recovery path; the row title still shows the file's stem so the user recognises it's their own work.
- **Result-list noise as Tab Configs proliferate.** A power user with 30+ Tab Configs would see all of them in the palette when typing a single character. **Mitigation:** the existing palette ranking algorithm handles dozens-to-hundreds of Launch Configurations today; Tab Configs ride the same ranking. If specific filtering becomes desirable (e.g. "tab config:" prefix to scope), that's a follow-up.
- **Telemetry duplicate events on rapid double-selection.** The existing palette debounces selections; the new dispatch arm reuses the same path so this is inherited.

## Follow-ups (out of this spec)

- Per-Tab-Config keyboard shortcut bindings (the issue's "ideally" ask).
- Menu-bar entry for Tab Configs.
- Folding Launch Configurations and Tab Configs into a single palette source once Launch Configurations are deprecated.
- A `tab config:` filter prefix for users with very large catalogs.
- A filesystem watcher on `~/.warp/tab_configs/` to avoid the per-open directory walk if profiling shows it as a hot path.
