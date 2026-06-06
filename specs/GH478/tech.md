# TECH.md — Per-tab theme overrides driven by directory and launch configurations

Issue: https://github.com/warpdotdev/warp/issues/478
Related: https://github.com/warpdotdev/warp/issues/2618
Product spec: `specs/GH478/product.md`

## Context

The Warp theme is currently a single global value derived from
`ThemeSettings`. The renderer reads it through the global `Appearance`
singleton, which today owns exactly one resolved `WarpTheme`. Per-tab
overrides therefore require changes in four layers — the launch
configuration schema, a new directory-pattern settings group, the
persisted tab snapshot (with override-source semantics), and the
renderer's theme lookup (with a theme catalog so non-active themes can be
borrowed by reference). None of this requires changing the theme storage
format or adding new theme types.

All file references are at HEAD on `master`.

### Launch configuration schema

`app/src/launch_configs/launch_config.rs` defines the YAML data model:

- `LaunchConfig` (lines 14–20) — `name`, `active_window_index`, `windows`.
- `WindowTemplate` (lines 36–41) — `active_tab_index`, `tabs`. **No theme
  field today.**
- `TabTemplate` (lines 180–187) — `title`, `layout`, `color:
  Option<AnsiColorIdentifier>`. The `color` field (line 186) is the
  closest precedent for a per-tab theme field.
- The file already imports `crate::themes::theme::AnsiColorIdentifier`
  (line 7).

### Theme model and global theme settings

- `app/src/themes/theme.rs:42–100` — `ThemeKind` enum, the canonical
  theme identifier. Built-in variants plus `Custom(CustomTheme)` and
  `CustomBase16(CustomTheme)`. Derives `Serialize`, `Deserialize`,
  `JsonSchema`, `settings_value::SettingsValue`.
- `app/src/themes/theme.rs:112+` — `Display for ThemeKind` returns the
  canonical user-facing string (`"Dark City"`, `"Solarized Dark"`, etc.).
- `app/src/settings/theme.rs:14–48` — `ThemeSettings` group; `theme_kind`
  field with `toml_path: "appearance.themes.theme"` (line 23).
- `app/src/settings/theme.rs:69–78` — `derived_theme_kind`, applies
  system-light/dark logic. Wrapped by per-tab resolution.
- `app/src/settings/theme.rs:81–83` — `active_theme_kind`, current entry
  point for "what theme should we show?".

### Renderer entry point

- `app/src/appearance.rs:34` re-exports `Appearance` from
  `warp_core::ui::appearance`.
- `app/src/appearance.rs:40–47` — `AppearanceManager`; subscribes to
  `ThemeSettings` (line 51) and pushes changes into `Appearance`.
- `Appearance` today owns one `WarpTheme`. Per-tab overrides require
  it to own a *catalog* (see *Proposed changes* §5).

### Tab and window state

- `app/src/app_state.rs:43–59` — `WindowSnapshot`. **No theme field.**
- `app/src/app_state.rs:61–69` — `TabSnapshot`. Carries
  `default_directory_color` and `selected_color: SelectedTabColor` (the
  per-tab indicator color).
- `app/src/app_state.rs:71–75` — `TabSnapshot::color()`, the resolution
  helper for the indicator.
- `app/src/tab.rs:134–143` — `TabData` (runtime), holds `selected_color`.

### Existing precedent — per-tab color

The path that loads `TabTemplate.color` into a runtime tab is the
closest analog. Today:

1. `TabTemplate.color: Option<AnsiColorIdentifier>` parsed from YAML
   (`launch_config.rs:186`).
2. On launch-config open, `app/src/workspace/view.rs:3517–3519` writes
   `tab_template.color` into `self.tabs[…].selected_color` via
   `SelectedTabColor::Color`.
3. `TabSnapshot.selected_color` persists this through session save/restore.

The new override follows the same path but with richer types (an
override-source enum, see §3) and an apply-time resolver (§2) instead of
direct serde (which is what Oz's first-pass review correctly flagged).

### Existing pane cwd tracking

The active pane's cwd is already tracked for tab title generation and
breadcrumbs. The directory-pattern feature (§4) hooks into the existing
"focused pane cwd changed" event rather than introducing new shell-side
plumbing.

## Proposed changes

Six steps in dependency order. Each compiles standalone.

### 1. Schema: `theme: Option<String>` on `TabTemplate` and `WindowTemplate`

File: `app/src/launch_configs/launch_config.rs`.

Add a new field to both structs:

```rust
#[serde(skip_serializing_if = "Option::is_none", default)]
pub theme: Option<String>,
```

The field is `Option<String>` rather than `Option<ThemeKind>` for two
reasons that directly address Oz's first-pass concerns:

- **Serde-compat (Oz concern 1).** `ThemeKind` derives `Deserialize`,
  but its accepted string form (variant names / snake-case) is not
  guaranteed to match the human-readable display strings the product
  spec promises (`"Dark City"`, `"Solarized Dark"`). Going through
  `String` decouples the YAML surface from the enum's internal
  representation.
- **Field-level fallback (Oz concern 2).** A direct `ThemeKind`
  deserialization on an unknown string would fail the entire YAML load
  and reject every tab in the file. With `Option<String>` the load
  always succeeds; resolution and fallback happen at apply time per the
  product spec's behavior #11.

The `From<WindowSnapshot> for WindowTemplate` (lines 43–70) and
`TryFrom<TabSnapshot> for TabTemplate` (lines 189–200) impls implement
the save rule from product behavior #10. The "preserved override" for a
tab is computed as:

```rust
fn preserved_override(state: &TabThemeState) -> Option<&ThemeKind> {
    // menu_pin and launch_config_pin both round-trip through saved
    // launch configs as tab-level `theme:`. menu_pin wins on
    // resolution (layer #1 > #2), so we save the higher-priority slot
    // when both are set; on reopen the field reseeds launch_config_pin
    // and the user's prior menu_pin (if any) is restored from session
    // state independently.
    state.menu_pin.as_ref()
        .or(state.launch_config_pin.as_ref())
        .or(state.window_default.as_ref())
}
```

(Directory-matched themes are deliberately not part of
`preserved_override` — they round-trip through `directory_overrides`,
not through the saved launch configuration.)

`From<WindowSnapshot> for WindowTemplate` then:

1. Computes the preserved override for every tab.
2. If every tab's preserved override is the same `Some(X)` *and* no tab
   has a manual pin different from the others, emits the saved
   `WindowTemplate` with `theme: Some(X.to_string())` and clears the
   per-tab `theme:` on every `TabTemplate`. This is the common case —
   a window opened from a launch configuration with a window-level
   `theme:` and no per-tab pinning round-trips losslessly.
3. Otherwise, emits no window-level `theme:` and writes each tab's
   preserved override (if any) into the corresponding `TabTemplate.theme`.
4. Tabs whose preserved override is `None` (all sources empty, or only
   `cwd_resolved` populated) emit no `theme:` field. Their effective
   theme on reopen will be either a fresh directory match or the
   global theme.

This is what Oz's v3 review correctly flagged: a window opened with a
window-level `theme:` would otherwise drop the theme on save, because
v2's "only emit manual" rule treated `window_default` as not preserved.
The `preserved_override` helper makes the logic symmetric with
resolution and adds an integration test (in §6) that exercises the
full round-trip.

Round-trip and serde tests live in
`launch_configs/launch_config_tests.rs` (referenced by the `#[path]`
attribute on line 11) and gain coverage in §6.

### 2. Theme-name resolver

A new helper, alongside `ThemeKind` in `app/src/themes/theme.rs`:

```rust
/// Resolve a free-form theme reference (from a launch configuration or
/// from `directory_overrides`) to a `ThemeKind`.
///
/// Accepts:
///   * Display strings: "Dark City", "Solarized Dark", "Dracula"
///   * Snake-case strings: "dark_city", "solarized_dark", "dracula"
///   * Custom theme names registered via the theme loader
///
/// Matching is case-insensitive on whitespace-stripped input.
/// Returns `None` for unknown names; callers log a warning and fall
/// through to the next resolution layer.
pub fn resolve_theme_ref(raw: &str) -> Option<ThemeKind> { ... }
```

Implementation:

1. Trim and case-fold the input.
2. Walk built-in `ThemeKind` variants; return the first whose `Display`
   string or snake-case form matches.
3. Consult the custom-theme registry the existing loader maintains
   (the same registry that powers the theme picker).
4. Return `None` on no match.

This satisfies Oz concerns 1 and 2 by making YAML-format coupling
explicit and keeping unknown-theme handling field-level.

### 3. `TabThemeState` on `TabSnapshot` and `TabData`

File: `app/src/app_state.rs` and `app/src/tab.rs`.

A tab can have up to four independent theme inputs at once — a menu
pin, a launch-config manual pin, a cwd-pattern match, and a
launch-config window-level default — and the product spec defines a
strict priority order between them (menu > launch-config-manual > cwd >
window default > global). A single `Option<enum>` cannot represent the
state "menu pin cleared, launch-config-manual still applies, cwd also
exists as a deeper fallback" because the sources are not mutually
exclusive at storage time, only at render time. Replace the single
field with a struct holding all four slots, plus a resolver:

```rust
/// Per-tab theme state. Each slot is independent storage for one
/// resolution layer; the resolver in `effective()` enforces the
/// priority order from `product.md` §Resolution order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TabThemeState {
    /// Layer #1. Set by the right-click "Pin theme" menu; cleared by
    /// the right-click "Reset theme" menu.
    /// **Persisted across sessions.**
    pub menu_pin: Option<ThemeKind>,

    /// Layer #2. Set by a tab-level `theme:` in the launch
    /// configuration that opened (or restored) the tab. Cleared only
    /// by the right-click "Forget launch config theme" menu, never by
    /// "Reset theme" — see Zach's v4 review.
    /// **Persisted across sessions.**
    pub launch_config_pin: Option<ThemeKind>,

    /// Layer #3. Computed by directory-pattern matching against the
    /// focused pane's cwd. Recomputed on tab creation, on focused-pane
    /// cwd change, and on `directory_overrides` settings change.
    /// **Not persisted** — recomputed on startup from current settings
    /// + restored cwd.
    pub cwd_resolved: Option<ThemeKind>,

    /// Layer #4. Set at open time when the tab was created from a
    /// launch configuration whose window had a window-level `theme:`.
    /// **Persisted across sessions** (the launch config that opened
    /// the tab is not necessarily reopened on restore).
    pub window_default: Option<ThemeKind>,
}

impl TabThemeState {
    /// Resolution order: menu_pin > launch_config_pin > cwd_resolved
    /// > window_default > caller's global fallback. Mirrors
    /// product.md §"Resolution order" exactly.
    pub fn effective<'a>(&'a self, global: &'a ThemeKind) -> &'a ThemeKind {
        self.menu_pin.as_ref()
            .or(self.launch_config_pin.as_ref())
            .or(self.cwd_resolved.as_ref())
            .or(self.window_default.as_ref())
            .unwrap_or(global)
    }

    pub fn has_any_override(&self) -> bool {
        self.menu_pin.is_some()
            || self.launch_config_pin.is_some()
            || self.cwd_resolved.is_some()
            || self.window_default.is_some()
    }
}
```

Add `pub theme_state: TabThemeState` to:

- `TabSnapshot` (`app_state.rs:61–69`), placed next to `selected_color`.
- `TabData` (`tab.rs:134`), same.

Session serialization writes `menu_pin`, `launch_config_pin`, and
`window_default`; `cwd_resolved` is recomputed on startup. The
serializer omits the field entirely when all four slots are `None`, so
existing sessions with no overrides round-trip with no schema cost.

`WindowSnapshot` does not gain a theme field; the window-level
launch-config theme lives only on the *tabs* it opened (in their
`window_default` slot). The dedicated slot is what makes
`effective()`'s priority order honor the product spec without
collapsing into an enum that could mis-rank window defaults — the
finding Oz raised in the v2 review.

Menu actions (§6) target individual slots:

- **Reset theme** clears `menu_pin` only. `launch_config_pin`,
  `cwd_resolved`, and `window_default` are unaffected — the tab falls
  through to the next non-empty layer per `effective()`.
- **Forget launch config theme** clears `launch_config_pin` only.
- "Pin theme..." sets `menu_pin`.

A tab can have a menu pin AND a launch-config pin simultaneously: the
menu pin wins for rendering (layer #1 > layer #2), and clearing the
menu pin reveals the launch-config pin underneath. This is exactly the
behavior Zach's v4 review asked for.

### 4. Directory-pattern overrides settings group

A new settings group, defined in a new file
`app/src/settings/directory_overrides.rs` and exported from
`app/src/settings/mod.rs`:

```rust
define_settings_group!(DirectoryThemeOverrides, settings: [
    overrides: Map {
        // Stored as TOML table: keys = directory paths, values = theme refs.
        type: BTreeMap<String, String>,
        default: BTreeMap::new(),
        supported_platforms: SupportedPlatforms::ALL,
        // **Local-only.** Directory paths can encode employer, customer,
        // and project names (e.g. `~/Work/<client>/<engagement>/...`).
        // Cloud-syncing the keys would leak that organizational context
        // off-machine; cloud-syncing the values without the keys is
        // useless. The setting is therefore not synced. Per-tab themes
        // remain user-controllable through the right-click "Pin theme"
        // menu, which writes to the (non-synced) tab snapshot, not to
        // this map. See *Privacy model* below.
        // (Per Zach's v4 review: `Never` is the correct variant here.
        // The settings system uses `Globally`, `Never`, `PerPlatform`;
        // there is no `Locally` variant.)
        sync_to_cloud: SyncToCloud::Never,
        private: true,
        toml_path: "appearance.themes.directory_overrides",
        max_table_depth: 1,
        description: "Local map of directory paths to theme names. The \
                      active pane's cwd is matched against keys (longest \
                      prefix wins); the matched theme overrides the global \
                      theme for that tab. Stored locally; never synced to \
                      Warp's cloud because path keys can leak employer or \
                      project names.",
    },
]);
```

#### Privacy model (responding to Oz's [SECURITY] finding on v2)

Directory paths are not just configuration — they encode information
about the user's employer, clients, and projects. A key like
`~/Work/AcmeCorp/redesign-2026` reveals all three. The settings system
already distinguishes synced from local settings via `sync_to_cloud`
and `private`; the design rule applied here is:

- **Keys are locally-stored, never synced.** `sync_to_cloud: Never`
  and `private: true`. The map is written only to the user's local
  `settings.toml` and is not transmitted off-machine by the settings
  sync path.
- **No telemetry on the contents.** The match function emits at most a
  count metric ("a directory match applied to N tabs this minute"); it
  never logs path keys or theme names to remote telemetry pipelines.
- **Local logs are also redacted.** Diagnostic output is routinely
  shared in bug reports and support sessions, so even the local
  Warp log must not contain raw `directory_overrides` keys. The
  redaction rule and helper are specced in *Diagnostic redaction* below.
- **No round-tripping into shareable artifacts.** A saved launch
  configuration does not emit `directory_overrides` entries (product
  spec #10). Launch configurations are explicitly designed to be
  shared between machines and users; the directory map is not.
- **Opt-in cloud sync is a follow-up.** A future `cloud_sync_directory_overrides`
  setting could let users with enterprise sync needs share the map
  *to themselves* across machines. That requires a separate spec
  covering opt-in UI, encryption-at-rest of keys, and admin-policy
  controls; it is deliberately not pre-paid for here.

#### Diagnostic redaction

Helper, alongside the settings group:

Per Oz's v5 [SECURITY] review: a stable unsalted 24-bit FxHash over
the raw key is dictionary-guessable — an adversary with a shared log
can hash a list of plausible candidate paths and recover the key by
collision. The fix is a **per-installation keyed hash**: a 32-byte
random salt is generated on first launch, stored in a local-only,
non-synced file (`~/.warp/redaction_salt`, mode `0600`), and never
leaves the machine.

```rust
/// Stable, **non-reversible** short identifier for a
/// `directory_overrides` key. Suitable for inclusion in user-facing
/// warnings and locally-shared diagnostic logs.
///
/// Implementation: HMAC-SHA256(installation_salt, raw_key), truncated
/// to 6 hex chars (24 bits). The installation salt is read from
/// `~/.warp/redaction_salt` (generated on first launch, mode 0600,
/// never synced). Because the salt is per-installation and never
/// shared, an identifier leaked in a log file cannot be reversed by
/// a dictionary attack against candidate paths — the attacker would
/// need the salt, which only ever exists on the originating machine.
///
/// Truncation to 24 bits keeps the identifier short enough for
/// readable log lines; with O(10) entries per typical user the
/// collision probability inside a single installation is negligible,
/// and cross-installation collisions are irrelevant because each
/// installation has its own salt.
pub fn redacted_key_id(raw_key: &str, salt: &InstallationSalt) -> String {
    let tag = hmac_sha256::HMAC::mac(raw_key.as_bytes(), salt.as_bytes());
    let truncated = u32::from_be_bytes([tag[0], tag[1], tag[2], tag[3]])
        & 0x00ff_ffff;
    format!("{truncated:06x}")
}
```

`InstallationSalt` is a thin newtype around `[u8; 32]` whose
constructor reads the salt file, generating it (with `rand::OsRng`,
mode `0600` write) if missing. It is loaded once into the settings
subsystem at startup and passed into `redacted_key_id` as a borrow;
no global static is required.

If the salt file is missing or unreadable at runtime — for example
the user deleted it, or Warp is running in a container without write
access — the redaction helper emits `directory_overrides[unsalted]:
...` with no derived identifier from the path at all, which is the
safest fallback and surfaces the configuration error to the user.

**Even-stronger alternative considered and rejected**: storing an
opaque local ID alongside each entry (e.g. a sidecar file mapping
`key → uuid`). That removes the hash entirely but requires every
settings edit to round-trip through Warp's runtime to allocate IDs
for new keys; users editing `settings.toml` directly in a text
editor would not get an ID until next launch, so the diagnostic for
a freshly-typed bad theme value would be
`directory_overrides[unidentified]`. The salted-HMAC scheme is
deterministic from the key alone and preserves "edit settings.toml,
get an immediate identifying warning" — the property that makes the
diagnostics actually useful for the user.

Usage rule (enforced by code review and by the tests in *Privacy
invariant tests*): every log / warn / error line emitted by the
`directory_overrides` matcher, settings subscriber, or value validator
must reference offending entries by `redacted_key_id` only.

Warnings include the offending **value** (the theme name) verbatim
because theme names are not sensitive and are needed for the user to
locate the entry in their `settings.toml`. Example warning text:

```
directory_overrides[id=8a3f9c]: unknown theme "Drakula" — matching this entry will be skipped until the value is corrected
```

The identifier is stable for a given key on a given machine, so the
same warning recurs with the same id across Warp restarts and helps
the user correlate multiple diagnostics about the same entry. The
identifier does **not** match between machines (different salts), so
correlation across users in a shared bug-report channel is impossible
— which is the point.

Resolution helper, alongside the group:

```rust
/// Returns the theme to use for `cwd`, walking the directory-overrides
/// map by longest-prefix match. Returns `None` when no key matches.
/// Tilde expansion and trailing-slash normalization happen here.
pub fn directory_theme_for(
    overrides: &BTreeMap<String, String>,
    cwd: &Path,
) -> Option<ThemeKind> { ... }
```

Match semantics (per product spec #2, #3):

- Keys are tilde-expanded once at evaluation time. On Linux/macOS tilde
  expands to `$HOME`; on Windows to `%USERPROFILE%`.
- Both `/` and `\` are accepted as separators in keys; normalization
  rewrites them to the platform's canonical form via
  `Path::components()` before comparison.
- On Windows, drive-letter prefixes are normalized to uppercase
  (`c:\…` → `C:\…`).
- A key matches a cwd if it is a prefix at a path-component boundary.
  Implementation: walk both paths' `Path::components()` after
  normalization and require the key's components to be an exact prefix
  of the cwd's. Using `str::starts_with` would let `~/Work/medone`
  spuriously match `~/Work/medone-archive` and is forbidden.
- Case sensitivity follows the platform's filesystem default:
  case-sensitive on Linux, case-insensitive on macOS, case-insensitive
  on Windows. Implementation uses
  `unicase::eq(key_component, cwd_component)` on Windows and macOS,
  `key_component == cwd_component` on Linux. Tests cover each platform.
- The longest matching key wins (most components after normalization,
  not most bytes — `~/Work` is shorter than `~/Work/medone` regardless
  of how the user typed them).
- Each value is resolved via `resolve_theme_ref`; unresolved values are
  skipped and a warning is logged once at settings-change time, not
  per-match. **Warnings refer to the offending entry by a short hash of
  its key, never by the key itself, per *Diagnostic redaction* below.**

Matching is invoked from **three** places (responding to Oz's v2
finding that settings edits were not handled):

- **Tab creation** — when a tab is added (from a launch configuration
  or from "new tab"), the focused pane's cwd is matched and
  `tab.theme_state.cwd_resolved` is set to the result (`None` on no
  match). The `launch_config_pin` slot is set independently from the
  launch config (`tab_template.theme` resolved through
  `resolve_theme_ref`); the `window_default` slot is set similarly
  from the window-level `theme:`. None of these is gated on
  `cwd_resolved` being absent — every applicable slot is filled, and
  `effective()` chooses the winner.
- **Focused-pane cwd-change events** — the existing pane-cwd tracker
  emits a "focused pane cwd changed" event used today for tab title
  updates. A new subscriber re-runs `directory_theme_for` for the
  affected tab when the cwd differs from the last-matched key. The
  runtime tab retains the last-matched key so re-matching is `O(1)`
  in the no-change case. Result is written to `cwd_resolved`; if
  the effective theme (per `TabThemeState::effective`) changes, the
  tab re-renders.
- **Settings-change events on `DirectoryThemeOverrides`** — the
  `define_settings_group!` macro emits change events the same way
  `ThemeSettings` does (`app/src/appearance.rs:51` is the existing
  pattern). A new subscription on `DirectoryThemeOverrides::handle(ctx)`
  walks every open tab in every window, recomputes
  `cwd_resolved` from the new map and the tab's current cwd, and
  re-renders each tab whose effective theme changed. Implements
  product behavior #6.

The same subscription handles validation: any value in the new map
that fails `resolve_theme_ref` is logged once via the
`redacted_key_id` helper (the raw key is **never** included in the
warning, per *Diagnostic redaction* below) and treated as absent for
matching purposes. Subsequent edits that fix
the value re-trigger this validation.

### 5. Renderer: `Appearance` gains a theme catalog

This addresses Oz concern 3 (`Appearance::theme_for` cannot return
`&WarpTheme` for arbitrary overrides because `Appearance` only owns the
global `WarpTheme` today).

`Appearance` is extended in `warp_core::ui::appearance`:

```rust
pub struct Appearance {
    // existing fields...

    /// Currently-active global theme. Always present in the catalog
    /// below, but tracked separately so `theme()` is O(1).
    global_theme: Arc<WarpTheme>,

    /// Lazy cache of all themes referenced by per-tab overrides.
    /// Keyed by `ThemeKind` so custom themes get their own entries.
    /// Wrapped in `RwLock` because lookup is on the hot rendering path
    /// while population happens on the (much rarer) settings-change path.
    theme_cache: parking_lot::RwLock<HashMap<ThemeKind, Arc<WarpTheme>>>,
}

impl Appearance {
    /// Active global theme. Unchanged contract.
    pub fn theme(&self) -> Arc<WarpTheme> {
        self.global_theme.clone()
    }

    /// Per-tab theme lookup. The caller passes
    /// `tab.theme_state.effective(global_kind)` (or `None` for the
    /// global theme).
    /// Returns the global theme when `override` is `None`.
    pub fn theme_for(&self, override_kind: Option<&ThemeKind>) -> Arc<WarpTheme> {
        match override_kind {
            None => self.global_theme.clone(),
            Some(kind) if *kind == self.global_theme.kind() => {
                self.global_theme.clone()
            }
            Some(kind) => {
                if let Some(t) = self.theme_cache.read().get(kind).cloned() {
                    return t;
                }
                self.load_and_cache(kind)
            }
        }
    }

    fn load_and_cache(&self, kind: &ThemeKind) -> Arc<WarpTheme> {
        // Load via the same path the global theme loader uses.
        // On failure, log warning and return self.global_theme.clone()
        // (callers see a fall-through to global theme; per behavior #11).
    }
}
```

Cache-invalidation rules:

- `AppearanceManager` already subscribes to `ThemeSettings` change events
  (`app/src/appearance.rs:51`). On every change it (a) updates
  `global_theme` and (b) calls a new `theme_cache.clear()` so any
  previously-cached entries are reloaded next time they are looked up.
  The hit-rate cost is bounded by the number of distinct overridden
  themes open at once (typically ≤ 5).
- Custom-theme file changes (which already invalidate the global theme
  via the existing watcher) trigger the same cache clear.

#### Renderer call-site migration

`Appearance::as_ref(ctx).theme()` is consumed in 862 call sites across
the workspace as of HEAD. An exhaustive line-by-line enumeration in this
spec would be unreviewable and would also rot the moment any caller
moves; the design instead defines a categorical rule plus a custom lint
that mechanically enforces it.

**Categorical rule** (matches product spec #15):

A call to `Appearance::*::theme()` becomes
`Appearance::as_ref(ctx).theme_for(Some(tab.theme_state.effective(&global_kind)))`
**if and only if** the call is reached during the rendering of content
that lives inside a single tab. All other calls keep the existing
global `theme()` form. Concretely:

| Area | Per-tab? | Notes |
| --- | --- | --- |
| `app/src/terminal/view/**` | per-tab | terminal grid, blocks, in-tab modals over the grid |
| `app/src/terminal/input/**` | per-tab | input bar, inline menus, slash commands inside the tab |
| `app/src/terminal/{view.rs, block_filter.rs, rich_history.rs, terminal_manager.rs, universal_developer_input.rs, ssh/**, warpify/**, shared_session/**}` | per-tab | terminal-pane content |
| `app/src/terminal/{share_block_modal.rs, profile_model_selector.rs}` | per-tab | scoped to a specific tab's content |
| `app/src/{tab.rs, root_view.rs, voltron.rs, modal.rs, menu.rs, search_bar.rs, input_suggestions.rs}` | global | window chrome, command palette, top-level modals |
| `app/src/settings/**` | global | settings UI |
| `app/src/reward_view.rs`, `wasm_nux_dialog.rs`, `word_block_editor.rs` | global | full-window modals/views |
| `crates/onboarding/**` | global | onboarding flow runs outside a tab |
| `crates/ui_components/**` | global | generic widgets — they do not know about tabs |
| `crates/editor/**` | global by default; per-tab when invoked from `terminal/view/**` (the call site, not the crate, decides) | editor is reusable |
| `crates/integration/**` | tests; updated to assert both forms |

**Migration enforcement — concrete tooling**

Per Oz's v5 suggestion, the spec commits to a specific mechanism
rather than leaving it open between tooling families. The
implementation adds the lint as a **`dylint`** library named
`appearance_theme_in_tab_path` under a new top-level `tools/lints/`
directory. `dylint` is the standard for project-specific Rust lints
(used by `rust-fuzz`, `solana`, and others), runs against a normal
`cargo` build, and does not require forking clippy or modifying the
toolchain. The lint:

- Flags any call to `Appearance::*::theme()` (the global accessor)
  whose enclosing module is under a *per-tab* path per the table
  above. Path classification is hard-coded in the lint source.
- Fails CI when triggered.
- Is silenced at a call site only by replacing the call with
  `theme_for(...)`, or — for a deliberate exception — by
  `#[allow(appearance_theme_in_tab_path = "...")]` with a written
  rationale (e.g. "this surface intentionally renders in window
  chrome even though it lives under terminal/view/").

CI hook: `./script/presubmit` gains
`cargo dylint --lib appearance_theme_in_tab_path -- -D warnings`
after the existing `cargo clippy` step. The lint runs in parallel
with clippy and adds well under a minute to presubmit time.

**Fallback** if the Warp team would rather not introduce `dylint` as
a new dependency: replace the lint library with a `ripgrep` check in
`./script/presubmit` that matches `Appearance::*::theme()` calls
under per-tab paths and exits non-zero on hit. This gives 90% of the
guarantee with no new tooling; the trade-off is no per-call-site
`#[allow]` syntax (deliberate exceptions need a known-marker comment
that the script greps and skips). The implementation PR opens with
`dylint`; if review prefers ripgrep, that switch is one file change
and one CI-script change.

The lint is the deliverable that gives this spec a feasibility
guarantee: the migration is correct *by construction* across all 862
sites, and any future call site added in a per-tab path is caught by
CI before merge. The PR opens with the lint enabled and zero
warnings; subsequent reviewers can read the diff knowing every flagged
site was either migrated or explicitly justified.

Window-chrome consumers (per the table) keep
`Appearance::as_ref(ctx).theme()` unchanged. The custom lint is the
single source of truth for which sites are which; reviewers verify the
table by reading the lint config, not by re-walking 862 call sites.

### 6. Right-click menu: Pin theme / Reset theme / Forget launch config theme

The existing tab context menu gains three entries:

- **Pin theme...** — opens a submenu listing built-in themes plus any
  loaded custom themes (the same list the theme picker shows). On
  click, sets `tab.theme_state.menu_pin = Some(chosen)`. Always
  visible (when the feature flag is on, see §7).
- **Reset theme** — sets `tab.theme_state.menu_pin = None`. The other
  three slots (`launch_config_pin`, `cwd_resolved`, `window_default`)
  are unaffected; if any still has a value the tab immediately
  re-renders with the next-priority theme per `effective()`. Visible
  only when `theme_state.menu_pin.is_some()`.
- **Forget launch config theme** — sets `tab.theme_state.launch_config_pin
  = None`. Visible only when `theme_state.launch_config_pin.is_some()`.
  This entry exists per Zach's v4 review: a user pinning a different
  theme via the menu should not also discard what their launch
  configuration originally set.

All three entries trigger the existing theme-changed redraw path used
today when the global theme changes (no new render-invalidation work).

Telemetry: one counter per click for each of the three entries (using
existing tab-menu telemetry conventions). No new event schema.

### 7. Feature flag wiring

Per Zach's v4 review, the entire feature ships behind a feature flag
`appearance.themes.per_tab_overrides`. The flag is added through the
existing feature-flag system (the same mechanism that gates other
in-development features like `OpenWarpNewSettingsModes`).

**Default values:**

- `dev` and `preview` channels: flag defaults to **on**.
- `stable` channel: flag defaults to **off** in the initial release.
  A follow-up changelog entry flips it on once preview telemetry
  confirms no regressions.

**Surfaces gated by the flag (when off):**

| Surface | Behavior with flag off |
| --- | --- |
| `directory_overrides` matching | Skipped entirely. The settings group is still parsed (so users who set up the map under preview do not lose data on stable) but `directory_theme_for` is never invoked. |
| Launch-config `theme:` fields | Deserialized as today (`Option<String>`) but ignored — `launch_config_pin` and `window_default` are not populated when applying templates. |
| Right-click menu entries | "Pin theme...", "Reset theme", and "Forget launch config theme" are hidden. |
| Theme catalog (§5) | `theme_for(None)` always returns the global theme. `theme_for(Some(_))` is reachable only if a render path bypasses the flag check — see *Flag check at the resolver* below — and falls back to the global theme via the catalog's existing missing-theme path. The catalog cache stays empty in normal flag-off operation. |
| Settings-change subscription on `DirectoryThemeOverrides` | Subscribed but the handler short-circuits when the flag is off. |
| `appearance_theme_in_tab_path` lint (§5) | Still enforced. Migrated call sites pass not `theme_state.effective(&global)` directly but the wrapper described below, so the flag toggle is the single place that controls runtime behavior. |

**Flag check at the resolver (single source of truth)**

Per Oz's v5 [IMPORTANT] finding: the table above implied populated
slots could not exist when the flag is off, but the persistence
section preserves them so flipping the flag back on is non-lossy.
Both must be true. The resolution is to put the flag check inside the
resolver itself, so populated slots **on disk** are independent from
populated slots **at render time**:

```rust
/// The single function the renderer should call. When the flag is
/// off, `effective()`'s slot walk is skipped entirely and the global
/// theme is returned. When on, `effective()` runs normally.
///
/// Putting the check here (not at the call site) means:
///   * Flipping the flag has no per-frame cost: still one branch.
///   * Persisted slot data is never consulted at render time when
///     off, so "did the migration miss a call site?" defects can't
///     leak themed pixels into a flag-off install.
///   * The 862 migrated call sites all look identical and don't
///     each need their own flag check.
pub fn theme_state_effective<'a>(
    state: &'a TabThemeState,
    global: &'a ThemeKind,
    ctx: &AppContext,
) -> &'a ThemeKind {
    if !per_tab_overrides_enabled(ctx) {
        return global;
    }
    state.effective(global)
}
```

The renderer-call-site migration in §5 calls `theme_state_effective`,
not `state.effective` directly. The lint flags both the bare
`Appearance::*::theme()` form and any direct `state.effective()` call
inside a per-tab module — both are migration mistakes.

**Persisted state with flag off:**

The `theme_state` field is still serialized and deserialized through
session restore. A user who pinned themes under preview, then opens
stable with the flag off, has their pins preserved on disk; flipping
the flag back on (or upgrading to a stable release that turns it on)
restores the visible behavior exactly as they left it. This avoids
any "I lost my themes when I downgraded" regression.

The settings-system feature-flag check is one call:

```rust
fn per_tab_overrides_enabled(ctx: &AppContext) -> bool {
    FeatureFlags::as_ref(ctx).per_tab_overrides.value()
}
```

Each gated surface above (matching, launch-config application, menu
visibility, settings-change subscription) guards entry with this
single call. The implementation PR introduces a constant for the flag
name to avoid typos.

## Testing and validation

### Unit tests

`app/src/themes/theme.rs` (or sibling test module):

- `resolve_theme_ref("Dark City")` → `Some(ThemeKind::DarkCity)`.
- `resolve_theme_ref("dark_city")` → `Some(ThemeKind::DarkCity)`.
- `resolve_theme_ref("  DARK CITY  ")` → `Some(ThemeKind::DarkCity)`
  (case-insensitive, whitespace-tolerant).
- `resolve_theme_ref("My Custom Theme")` → `Some(ThemeKind::Custom(...))`
  when registered; `None` otherwise.
- `resolve_theme_ref("Definitely Not A Theme")` → `None`.

`app/src/launch_configs/launch_config_tests.rs`:

- YAML round-trip of `TabTemplate { theme: Some("Solarized Dark") }`.
- A YAML file with one valid and one unknown `theme:` value loads
  successfully; resolution applied at apply time, only the unknown tab
  falls through, the file is **not** rejected.
- `WindowTemplate.theme` set with all tabs un-themed: round-trip
  preserves the window-level value.
- Saving a snapshot whose tabs have only `Cwd(_)` overrides emits no
  `theme:` fields in the resulting YAML.

`app/src/settings/directory_overrides.rs` (new):

- `directory_theme_for(map, "~/Work/medone/apps/admin-api")` returns
  the theme mapped to `"~/Work/medone"`.
- `directory_theme_for(map, "~/Work/medone-archive")` returns `None`
  when only `"~/Work/medone"` is mapped (component-boundary match).
- Two overlapping keys `"~/Work"` and `"~/Work/medone"`: longer wins
  for paths under `medone`, shorter wins elsewhere under `~/Work`.
- Unresolved theme value: that entry is skipped, `directory_theme_for`
  on a path that would have matched it returns `None` (or the next
  match), and one warning is logged.

`app/src/app_state.rs` (or test module):

- Each persisted slot round-trips through session-restore
  serialization: `menu_pin`, `launch_config_pin`, `window_default`.
- `cwd_resolved` does **not** persist; on deserialization the slot is
  `None` regardless of what was written. (Pins product behavior #13.)
- `TabThemeState::effective()` walks the priority order exactly:
  - `menu=A launch=B cwd=C window=D` → `A`.
  - `menu=None launch=B cwd=C window=D` → `B`.
  - `menu=None launch=None cwd=C window=D` → `C`.
  - `menu=None launch=None cwd=None window=D` → `D`.
  - All `None` → global fallback.
  Pins the 5-layer resolution order from product.md and addresses
  the CRITICAL finding from Oz's v2 review (window defaults must
  not outrank cwd) plus Zach's v4 split (menu pin must outrank
  launch-config pin).
- "Reset theme" semantics: a tab with `menu=A launch=B` resolves to
  `A`. After clearing `menu_pin`, it resolves to `B`. A tab with
  `menu=A` only resolves to `A`; after clear, falls through to
  global. Pins product behavior #18.
- "Forget launch config theme" semantics: a tab with `menu=A launch=B`
  resolves to `A`. After clearing `launch_config_pin`, it still
  resolves to `A` (menu wins). After also clearing `menu_pin`, it
  falls through to global.
- `TabSnapshot::color()` returns the same value with and without
  `theme_state` populated (color and theme are independent — #19).

`app/src/appearance.rs` (or `warp_core` tests):

- `Appearance::theme_for(None)` returns the same `Arc` as
  `Appearance::theme()`.
- `Appearance::theme_for(Some(&ThemeKind::Light))` returns a `WarpTheme`
  whose kind is `Light` regardless of the active global theme.
- `Appearance::theme_for(Some(&ThemeKind::Custom(missing)))` falls back
  to the global theme and increments the existing missing-custom-theme
  warning counter.
- After `ThemeSettings` changes, the cache is cleared: a subsequent
  `theme_for(Some(&kind))` reloads from disk rather than returning
  stale custom-theme content.

### Integration tests

`crates/integration/`:

- Open a launch configuration with three tabs — `theme: "Dracula"`,
  `theme: "Solarized Light"`, no `theme:`. Assert each tab renders the
  expected theme and that switching tabs does not redraw inactive tabs.
- Configure `[appearance.themes.directory_overrides]` with two entries.
  Open one tab in each matched directory and one in an unmatched
  directory. Assert each tab renders the expected theme. `cd` the
  unmatched tab into a matched directory; assert the tab redraws with
  the matched theme. `cd` it back out; assert it redraws with the
  global theme.
- Tab with `launch_config_pin = Some(Dracula)` (set via launch-config
  tab-level `theme:`) sits in a directory that maps to
  `Solarized Dark`. Assert Dracula wins (launch_config_pin > cwd).
  Right-click → "Forget launch config theme". Assert the tab redraws
  with Solarized Dark.
- Tab with both `menu_pin = Some(Light)` and `launch_config_pin =
  Some(Dracula)`. Assert Light wins (menu_pin > launch_config_pin).
  Right-click → "Reset theme" (clears menu_pin only). Assert the tab
  redraws with Dracula. Right-click → "Forget launch config theme".
  Assert the tab redraws with the next-priority theme. (Addresses
  Zach's v4 review — menu and launch-config pins must be
  independently clearable.)
- **Settings-change recompute.** With three tabs open in three
  different cwds, edit `directory_overrides` to add a new key that
  matches one of them. Assert that one tab redraws with the new
  theme; the other two are unchanged. Then edit the same key's value
  to a different theme name. Assert the matched tab redraws again.
  Then delete the key. Assert the tab falls through to its
  next-priority theme. (Pins product behavior #6 and addresses Oz's
  v2 IMPORTANT finding that this path was missing from the design.)
- **Save-from-snapshot for window-level theme.** Open a launch config
  whose window has `theme: "Dark City"` and three tabs (no per-tab
  pins, no directory matches). Save the resulting layout to a new
  launch config. Assert the saved YAML contains a window-level
  `theme: "Dark City"` and no per-tab `theme:` fields. Reopen it.
  Assert all three tabs render Dark City. (Addresses Oz v3 IMPORTANT
  finding on save-rule dropping window defaults.)
- **Save-from-snapshot for mixed pinning.** Open a layout with a mix
  of manually pinned tabs (different themes), tabs with only a window
  default, and tabs whose effective theme came from cwd. Save.
  Assert: window-level `theme:` is omitted (mixed manual pins make
  coalescing impossible); each manually pinned tab emits its own
  `theme:`; tabs with only `window_default` emit their default; tabs
  themed only by cwd emit no `theme:`. Reopen and assert the
  effective theme of every tab matches what it was pre-save.
- **Windows path normalization.** Cross-platform integration test
  matrix:
  - Linux: keys `~/Work/medone` (case-sensitive) — assert
    `~/Work/MEDONE` does **not** match.
  - macOS: same key — assert `~/Work/MEDONE` **does** match
    (case-insensitive default).
  - Windows: keys `C:\Work\medone` and `c:\Work\medone` collapse to
    one entry (drive-letter normalization). Cwd `C:/Work/medone/app`
    matches (separator normalization). Cwd
    `C:\Work\medone-archive` does not match (component boundary).
- **Diagnostic redaction.** Configure `directory_overrides` with a
  key `~/Work/AcmeCorp/2026` mapped to a bad theme name `"Drakula"`.
  Trigger validation. Capture the warning string. Assert it contains
  `"Drakula"` and `redacted_key_id("~/Work/AcmeCorp/2026", &salt)`,
  and does **not** contain `"AcmeCorp"`, `"Work"`, `"2026"`, or any
  substring of the raw key.
- **Per-installation salt.** Two test runs with different generated
  salts on the same key produce **different** identifiers (asserts
  the salt is actually keyed in). Two runs with the same salt
  produce the **same** identifier (asserts determinism within an
  installation).
- **Salt file mode.** On a Unix platform, after first-launch salt
  generation, `stat` the file and assert mode bits == `0600`.
- **Salt missing fallback.** Delete the salt file mid-test, trigger a
  fresh validation. Assert the warning string contains
  `[unsalted]` and does not contain any derived identifier.
- Open a launch configuration with a window-level `theme:` and three
  tabs, one of which sits in a `directory_overrides`-matched cwd.
  Assert the matched tab uses the cwd theme (cwd > window default);
  the other two use the window default. (Pins the priority order and
  addresses Oz's v2 CRITICAL finding.)
- Quit and relaunch. Assert manually-pinned tabs restore their pin;
  tabs that had only a `cwd_resolved` theme have `cwd_resolved` set
  to `None` after deserialization but are recomputed on startup from
  current settings + cwd.
- Unknown theme name in a launch configuration: the file opens,
  exactly that one tab falls through, the warning appears in the log,
  other tabs render correctly.

### Feature-flag tests

- With `appearance.themes.per_tab_overrides` **off**, open a launch
  configuration containing both window-level and tab-level `theme:`
  fields. Assert no tab carries any populated slot in `theme_state`
  (effective() falls through to global) and no warning is logged for
  the launch-config theme references.
- With the flag **off**, configure `directory_overrides` and `cd` into
  a matched directory. Assert no theme change occurs.
- With the flag **off**, the right-click tab menu does not include
  "Pin theme...", "Reset theme", or "Forget launch config theme".
- Toggle the flag from **off** to **on** while a session is restored
  with persisted slots from a prior preview run. Assert the persisted
  `menu_pin`, `launch_config_pin`, and `window_default` slots resume
  their effects without losing data, and `cwd_resolved` is recomputed
  from current settings.
- Toggle the flag from **on** to **off** mid-session. Assert all
  themed tabs redraw with the global theme (because
  `theme_state_effective` short-circuits when the flag is off, even
  though slot data remains populated in memory and on disk).
  Persisted slot data is not cleared. Toggle back **on**: assert
  every tab redraws with its prior effective theme without any
  reload.

### Privacy invariant tests

- `DirectoryThemeOverrides` settings group has `private == true` and
  `sync_to_cloud == SyncToCloud::Never`. Pinned by a settings-system
  test analogous to the existing tests that gate which settings sync.
- The redaction-salt file (`~/.warp/redaction_salt`) is created with
  mode `0600` on first launch and is **not** included in any
  cloud-synced settings payload (verified by the same serializer test
  that asserts no path keys leak).
- The settings sync serializer, when run over a populated
  `DirectoryThemeOverrides`, emits no entry containing a path key in
  the cloud-sync payload. Test by populating the map, running the
  serializer, and asserting the resulting payload does not contain
  the literal key string.
- Saving a window's state as a launch configuration with a populated
  `directory_overrides` map produces YAML with no
  `directory_overrides` field.

### Manual verification

- macOS, Linux, Windows: visually confirm terminal background and ANSI
  palette match the expected theme for each tab in the test scenarios.
- Confirm window chrome (title bar, sidebar, tab strip) follows the
  global theme in all scenarios.
- Toggle system light/dark while a mix of themed/unthemed tabs is open;
  confirm only unthemed tabs follow the system.
- Save layout as launch configuration; confirm tabs with manual pins
  emit `theme:`, tabs themed only by directory matching do not.
- After implementation, invoke the `verify-ui-change-in-cloud` skill
  per the repository rule for user-facing client changes.

### Tooling

- `cargo fmt` and
  `cargo clippy --workspace --all-targets --all-features --tests -- -D warnings`
  must pass.
- `cargo nextest run` for unit tests; `crates/integration` for
  end-to-end.
- `./script/presubmit` before pushing.

## Risks and mitigations

- **Render hot-path cost of the per-tab lookup.** The added work per
  frame is one `Option` deref, one `Arc` clone in the common case (no
  override or override == global), or one `RwLock::read` + `HashMap`
  hit when the override resolves to a non-global cached theme. The
  existing per-tab indicator color path runs on the same frame and has
  not been a bottleneck. If profiling flags it post-merge, cache the
  `Arc<WarpTheme>` directly on `TabData` and invalidate on
  override-change; the change is local.

- **Cache stampedes when the user changes the global theme with many
  overridden tabs open.** `theme_cache.clear()` followed by lazy
  reload means each unique overridden theme reloads once. Bounded by
  open-tab cardinality.

- **Mismatch between `directory_overrides` keys and how the OS reports
  paths.** Tilde expansion happens in the resolver; symlinks are not
  followed (matching whatever the shell's `pwd` reports). This is
  documented in the product spec; integration tests cover the common
  cases.

- **`cwd_resolved` mistakenly persists across sessions.** The slot's
  serde derives skip it on serialization and default it to `None` on
  deserialization; the integration test for restart behavior pins this.

- **Path keys leaking off-machine.** Addressed in *Privacy model* (§4):
  `private: true`, `sync_to_cloud: Locally`, no telemetry on contents,
  no roundtrip into shareable launch-config YAML. Privacy invariant
  tests pin each rule.

- **Window-default outranking cwd matches.** Addressed by giving
  window-default its own slot in `TabThemeState` (§3) rather than
  expanding it into the manual slot. `effective()` walks slots in the
  product spec's resolution order; the integration test for the
  launch-config-with-window-default scenario pins the priority.

- **Save layout dropping window-level theme.** Addressed by the
  `preserved_override` helper in §1. Both save tests above pin the
  round-trip.

- **Windows path matching ambiguity.** Component-boundary matching
  uses `Path::components()` rather than string operations, separator
  and drive-letter normalization happens before comparison, and case
  semantics are platform-conditional. Pinned by the cross-platform
  test matrix above. The lint `appearance_theme_in_tab_path` does not
  cover this — these are runtime correctness tests.

- **Path keys leaking through diagnostics.** Addressed by
  `redacted_key_id` and the *Diagnostic redaction* contract in §4.
  Pinned by the redaction test above plus a code-review checklist
  item; we considered a custom lint but the surface is small enough
  (one helper function, one logger call site) that a unit-test
  enforcement is sufficient.

- **Renderer call-site drift.** The
  `appearance_theme_in_tab_path` lint catches both the initial
  migration and any future call site added in a per-tab path. This
  replaces a one-time enumeration that would have rotted on the next
  PR. The trade-off is the small cost of maintaining the lint config
  (one entry per top-level area).

- **Concern 1 / Concern 2 / Concern 3 from Oz's first-pass review.**
  Addressed in §1 (Option<String> + apply-time resolver), §2
  (`resolve_theme_ref` returns `Option<ThemeKind>`), and §5 (theme
  catalog, `Arc<WarpTheme>` ownership) respectively. Each concern has
  a corresponding unit test above.

- **Per-pane theme creep.** Override slots live on the tab, not the
  pane; the resolver consults the focused pane's cwd. A future
  per-pane theming feature would need its own data path; this spec
  does not lock that out but does not pre-pay for it either.

- **Persistence schema collision with future per-window theming.**
  The field is named `theme_state` (a struct of slots) rather than
  `theme`, so a separate resolved theme can live elsewhere if added
  later.

## Follow-ups (deliberately not in this PR)

- **Glob pattern support in `directory_overrides`** (open question in
  product.md) — extends key matching from prefix to glob.
- **Auto-theme by SSH host or hostname** (`stevenchanin`, `pyronaur`,
  `zethon`, `janderegg` in #478). Requires detection inside the tab's
  shell; would add a fourth slot (`ssh_resolved`) to `TabThemeState`
  and a corresponding rung in `effective()`'s priority order.
- **Runtime escape-code or shell-hook protocol for setting a tab's
  theme** (`yatharth`, for Claude-Code session signaling). Defines a
  wire format and a security model; consumes the override field.
- **In-tab "Pin theme" command in the command palette** (so users can
  pin without right-clicking). Strictly additive.
- **Wallpaper-per-tab** (`scottaw66`, `SheepDomination`). Different
  surface area entirely; does not conflict.
- **Closing #2618 as a duplicate** of #478 once this spec lands.
