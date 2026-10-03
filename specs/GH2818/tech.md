# Tech Spec: Open a launch configuration or tab config with a keyboard shortcut

**Issue:** [warpdotdev/warp#2818](https://github.com/warpdotdev/warp/issues/2818)

**Product spec:** [`specs/GH2818/product.md`](product.md)

Line references are against `master` at `d5d12d90f`.

## Context

### Keybindings are a static, compiled-in list

- `crates/warpui_core/src/keymap.rs:293-305` — `EditableBinding { name: &'static str, description,
  action: Arc<dyn Action>, context_predicate, enabled_predicate, trigger, custom_trigger, group:
  Option<&'static str>, id }`. `EditableBinding::new(name: &'static str, …)` at `:646`.
- `keymap.rs:25-39` — `Keymap` holds `editable_bindings: Vec<Tracked<EditableBinding>>` and an
  index `editable_bindings_by_name: HashMap<&'static str, Vec<usize>>` (`:33`).
- `keymap.rs:405-419` — `register_editable_bindings` is **append-only**; there is no API to remove
  or replace editable bindings (verified: no `remove`/`retain`/`unregister` in the file).
- `keymap.rs:422-432` — `update_custom_trigger(name, trigger)` iterates the bindings already
  registered under `name`. A custom trigger set for a name that is **not yet registered is silently
  dropped**.
- `crates/warpui_core/src/keymap/matcher.rs:111-133` (`register_editable_bindings`),
  `:136-141` (`set_custom_trigger(name: String, …)`), `:145-151` (`remove_custom_trigger`) —
  thin wrappers that clear `pending` keystrokes and forward to the `Keymap`.
- `crates/warpui_core/src/core/app.rs:1804-1810`, `:1814-1816`, `:1833-1839` — the
  `AppContext` surface for the above. Registration is legal at any time, not only during init.
- `keymap.rs:237` — `EnabledPredicate = fn() -> bool` (capture-free), used via `with_enabled`.

The user's overrides live in `keybindings.yaml`:

- `app/src/keyboard.rs:33` (`KEYBINDINGS_FILE_NAME`), `:37-58` `load_custom_keybindings` reads the
  file and calls `app.set_custom_trigger(name, …)` per entry; `:64` `write_custom_keybinding(name:
  String, …)`; `:79` `remove_custom_keybinding`; `:95-97` `keybinding_file_path()`. The on-disk map
  is already `String`-keyed; only the registration side is `&'static`.
- `app/src/lib.rs:3078` — `load_custom_keybindings` runs in `launch()`. The `WarpConfig`
  singleton is created earlier at `app/src/lib.rs:1498`, but it loads launch configs and tab
  configs **asynchronously** (`app/src/user_config/native.rs:31-52`), so config-derived bindings
  will always register *after* the custom triggers have been applied. Combined with the
  silent-drop behavior above, dynamic bindings need the keymap to remember pending triggers.

### How the workspace declares bindings today

- `app/src/workspace/mod.rs:709-716` — `NEW_TAB_BINDING_NAME` → `WorkspaceAction::AddDefaultTab`,
  `.with_context_predicate(id!("Workspace") & !id!("Workspace_PaneDragging"))`,
  `.with_enabled(|| ContextFlag::CreateNewSession.is_enabled())`.
- `mod.rs:1152-1162` — `"workspace:toggle_launch_config_palette"` gated with
  `.with_enabled(|| ContextFlag::LaunchConfigurations.is_enabled())`.
- `app/src/workspace/action.rs:129` — `WorkspaceAction` (`#[derive(Debug, Clone)]`), handled in
  `Workspace::handle_action` (`app/src/workspace/view.rs:24070-24075` shows `SelectTabConfig` and
  `SelectNewSessionMenuItem`). `action.rs:932` `should_save_app_state_on_action` matches variants
  exhaustively, so new variants must be added there.

### Launch configurations

- `app/src/launch_configs/launch_config.rs:16-21` — `LaunchConfig { name: String,
  active_window_index, windows: Vec<WindowTemplate> }`; `:182-190` `TabTemplate { title, layout:
  PaneTemplateType, commands, color }`; `:275` `make_mock_single_window_launch_config()` (tests).
- `app/src/user_config/mod.rs:90-103` — `WarpConfig { launch_configs, tab_configs, … }`;
  accessors `launch_configs()` / `tab_configs()` at `:118-124`; dirs at `:210-217`.
- `app/src/user_config/mod.rs:55-65` — `WarpConfigUpdateEvent::{LaunchConfigs, TabConfigs, …}`
  emitted after the initial async load and after every directory change
  (`user_config/native.rs:119-137`). `app/src/search/command_palette/launch_config/data_source.rs:51-60`
  is the canonical subscriber pattern.
- Opening: `app/src/root_view.rs:250-259` `OpenLaunchConfigArg { launch_config, ui_location,
  open_in_active_window }` dispatched as the global action `"root_view:open_launch_config"`, handled
  by `open_launch_config` at `root_view.rs:565-613`. With `open_in_active_window && windows.len()
  == 1` it calls `Workspace::open_launch_config_window` (`workspace/view.rs:3895-3929`), which
  appends the template's tabs via `add_tab_with_pane_layout` and focuses the active one.
  Otherwise it opens new windows. The palette's `execute_result` already uses
  `open_in_active_window: true` (`command_palette/launch_config/search_item.rs:66-70`).
- Existing entry points all set a `LaunchConfigUiLocation`
  (`app/src/server/telemetry/events.rs:608-613`: `CommandPalette | AppMenu | TabMenu | Uri`):
  palette `command_palette/view.rs:884-896`, app menu `app_menus.rs:940-960`, `+` menu
  `workspace/view.rs:7094-7124`, URI `uri/mod.rs:211-232`.
- Identity: the URI path resolves a config by `name`, case-insensitively
  (`uri/mod.rs:769-793` `find_matching_config` / `find_matching_config_name`).
- Pre-existing bug worth fixing in passing: `root_view.rs:606-612` reports
  `LaunchConfigUiLocation::Uri` in `TelemetryEvent::OpenLaunchConfig` regardless of
  `arg.ui_location`.

### Tab configs

- `app/src/tab_configs/tab_config.rs:138-165` — `TabConfig { name, title, color, panes, params,
  source_path: Option<PathBuf> /* #[serde(skip)] */ }`.
- `workspace/view.rs:7152-7192` — `Workspace::open_tab_config(tab_config, ctx)`: no params →
  `open_tab_config_with_params` (`:7126-7150`, renders and calls `add_tab_with_pane_layout`) and
  emits `TabConfigsTelemetryEvent::ExistingConfigOpened { open_mode: Direct, … }`; with params →
  opens `tab_config_params_modal`.
- Identity: `warp://tab_config/<stem>` resolves by the **file stem** of `source_path`,
  case-insensitively (`uri/mod.rs:853-866` `find_matching_tab_config`).
- Gating: `FeatureFlag::TabConfigs` (`crates/warp_features/src/lib.rs:809`), checked in
  `user_config/native.rs:45,130`.

### Settings → Keyboard Shortcuts

- `app/src/settings_view/keybindings.rs:749-813` — `on_page_selected` rebuilds the list from
  `ctx.editable_bindings()` → `CommandBinding::from_editable_lens`
  (`app/src/util/bindings.rs:765-774`, `name: lens.name.into()` → `String`), sorted by description,
  de-duplicated by `(name, description)`, and seeds the `ConflictMap`. Anything registered in the
  keymap appears here automatically; the list is only rebuilt when the page is (re)selected.
- `keybindings.rs:670-701` — `confirm_keystroke_editing` persists via `set_custom_keybinding`
  and emits `TelemetryEvent::KeybindingChanged { action: name, … }`.
- `keybindings.rs:59-84` — `KeybindingChangedNotifier` / `KeybindingChangedEvent`, already used
  by the Resource Center page to rebuild on changes (`resource_center/keybindings_page.rs:177`).
- `util/bindings.rs:817-838` — `BindingGroup::as_str` / `from_str` (via `enum_iterator::all`).
- `resources/bundled/skills/change-keybinding/SKILL.md` — the bundled agent skill documenting the
  `keybindings.yaml` format and how to identify action names.

### Integration-test precedents

- `crates/integration/src/test/launch_configs.rs:27` `test_add_launch_config_to_warp_config`
  writes a config into `integration_testing::launch_configs::launch_configs_dir()` with
  `WARP_CONFIG_WATCHER_DELAY_MS=10` and waits for the palette to pick it up.
- `launch_configs.rs:258` `test_open_launch_config_in_active_window` dispatches
  `root_view:open_launch_config` with `open_in_active_window: true` and asserts
  `assert_num_windows_open(1)` / `assert_tab_count(3)`.
- `crates/integration/src/test.rs:1659-1668` writes a fake `keybindings.yaml` via
  `integration_testing::create_file_with_contents(…, &keybinding_file_path())`, then drives the
  binding with `TestStep::with_keystrokes` (`crates/warpui_core/src/integration/step.rs:402`).

## Proposed changes

### 1. Keymap: allow owned binding names

`crates/warpui_core/src/keymap.rs`

- `EditableBinding.name: Cow<'static, str>`; `EditableBindingLens.name: &'a str`;
  `Keymap.editable_bindings_by_name: HashMap<Cow<'static, str>, Vec<usize>>`
  (`Cow<'static, str>: Borrow<str>`, so `get_binding_by_name(&str)` is unchanged).
- `EditableBinding::new(name: impl Into<Cow<'static, str>>, …)`. Every existing call site passes
  a string literal and compiles unchanged; `CommandBinding::from_editable_lens` already converts
  with `.into()`.
- `FixedBinding` is untouched.

### 2. Keymap: remember custom triggers for names registered later

`crates/warpui_core/src/keymap.rs`

- Add `custom_triggers: HashMap<String, Option<Trigger>>` to `Keymap`.
- `update_custom_trigger(name, trigger)` records into the map unconditionally, then applies to
  currently registered bindings as today. `None` (from `remove_custom_trigger`) removes the entry.
- `register_editable_bindings` applies `custom_triggers.get(name)` to each new binding.

This makes the order of `load_custom_keybindings` and any registration irrelevant. It also fixes
the latent hazard for built-in bindings registered lazily (none today, but the API allows it).

### 3. Keymap / Matcher / AppContext: replace a namespace of editable bindings

`crates/warpui_core/src/keymap.rs`, `keymap/matcher.rs`, `core/app.rs`

```rust
/// Drops every editable binding whose name starts with `prefix`, then registers `bindings`.
pub fn replace_editable_bindings_with_prefix<A>(&mut self, prefix: &str, bindings: A)
where
    A: IntoIterator<Item = EditableBinding>;
```

- Removes matching entries from `editable_bindings` and `editable_custom_action_bindings`,
  rebuilds `editable_bindings_by_name` from scratch (indices shift on removal), then delegates to
  `register_editable_bindings` so pending custom triggers (§2) apply.
- `Matcher` wrapper clears `pending` like its siblings; `AppContext` exposes it with the same
  shape as `register_editable_bindings` (`app.rs:1804`).
- Prefixes are owned by exactly one caller each (§5), so a full rebuild of the index on every
  config reload is acceptable: the list is a few hundred entries and reloads are user-driven.

### 4. Workspace actions that resolve a config by identity at dispatch time

`app/src/workspace/action.rs`, `app/src/workspace/view.rs`

```rust
/// Opens the launch configuration with this `name`, resolved from `WarpConfig` when dispatched.
OpenLaunchConfigNamed { name: String },
/// Opens the tab config whose file stem is `file_stem`, resolved from `WarpConfig` when dispatched.
OpenTabConfigNamed { file_stem: String },
```

- The action carries an identifier rather than a `LaunchConfig`/`TabConfig` snapshot so the
  binding never opens stale contents and stays cheap to clone and log.
- Handler for `OpenLaunchConfigNamed`: look up `WarpConfig::handle(ctx).as_ref(ctx).launch_configs()`
  by name, case-insensitively (reuse `find_matching_config_name`; move it out of `uri/mod.rs`
  into `launch_configs/` so both call sites share it), then
  `ctx.dispatch_global_action("root_view:open_launch_config", OpenLaunchConfigArg {
  launch_config, ui_location: LaunchConfigUiLocation::Keybinding, open_in_active_window: true })`.
- Handler for `OpenTabConfigNamed`: look up `tab_configs()` by `source_path` file stem,
  case-insensitively (reuse `find_matching_tab_config`, moved next to `TabConfig`), then
  `self.open_tab_config(config.clone(), ctx)` — params modal included for free.
- Not found: `log::warn!` and an ephemeral toast through the workspace's
  `DismissibleToastStack::add_ephemeral_toast` (`app/src/workspace/toast_stack.rs:24`) with the
  text from product invariant 9.
- Add both variants to `should_save_app_state_on_action` (`action.rs:932`) alongside
  `SelectTabConfig` / `OpenLaunchConfigSaveModal`, and to any other exhaustive `match` over
  `WorkspaceAction` the compiler points at (no `_` arms per AGENTS.md).

### 5. `config_keybindings`: derive bindings from `WarpConfig` and keep them in sync

New module `app/src/config_keybindings.rs` (+ `config_keybindings_tests.rs`), `init(app)` called
from `app/src/lib.rs` right after `WarpConfig` is added (`lib.rs:1498`).

- A small singleton model subscribes to `WarpConfig` and, on init and on
  `WarpConfigUpdateEvent::LaunchConfigs | TabConfigs`, rebuilds:

  ```rust
  const LAUNCH_CONFIG_BINDING_PREFIX: &str = "launch_config:open:";
  const TAB_CONFIG_BINDING_PREFIX: &str = "tab_config:open:";

  EditableBinding::new(
      format!("{LAUNCH_CONFIG_BINDING_PREFIX}{name}"),
      format!("Open launch configuration \"{name}\""),
      WorkspaceAction::OpenLaunchConfigNamed { name: name.clone() },
  )
  .with_context_predicate(id!("Workspace") & !id!("Workspace_PaneDragging"))
  .with_group(BindingGroup::LaunchConfigurations.as_str())
  .with_enabled(|| ContextFlag::LaunchConfigurations.is_enabled())
  ```

  and the tab-config twin (`file_stem` in the name, `config.name` in the description,
  `.with_enabled(|| FeatureFlag::TabConfigs.is_enabled())`).
- Identity rules: launch configs keyed by `name`, tab configs by `source_path` file stem — the
  same keys the URI handlers accept, so documentation can describe one identifier per config type.
  Duplicates (case-insensitive) keep the first and `log::warn!`; tab configs without a UTF-8 stem
  are skipped with a warning.
- Publishes with `ctx.replace_editable_bindings_with_prefix(LAUNCH_CONFIG_BINDING_PREFIX, …)` and
  the tab-config equivalent, then emits a new `KeybindingChangedEvent::BindingsReloaded` on
  `KeybindingChangedNotifier`.
- `BindingGroup` gains `LaunchConfigurations` (`"launch_configurations"`) and `TabConfigs`
  (`"tab_configs"`) in `util/bindings.rs:817-832`; `from_str` picks them up via `all::<Self>()`.
- `settings_view/keybindings.rs`: subscribe to `KeybindingChangedEvent::BindingsReloaded` and
  re-run the list build from `on_page_selected` (extract it into `rebuild_bindings`) so an open
  page reflects config changes (product invariant 10). The Resource Center page ignores the event:
  its sections are static allow-lists (`keybindings_page.rs:229-260`).
- No `CustomAction` is registered for these bindings, so they do not appear in the macOS app
  menus; the launch-configuration menu already lists configs by name.

### 6. Telemetry

- `LaunchConfigUiLocation::Keybinding` (`events.rs:608-613`).
- `root_view.rs:606-612`: report `arg.ui_location` instead of the hardcoded `Uri`.
- Optional, per product open question 3: add `open_source: TabConfigOpenSource { Menu, Uri,
  Keybinding }` to `TabConfigsTelemetryEvent::ExistingConfigOpened` (`tab_configs/telemetry.rs`),
  which requires updating the exhaustive telemetry table test in `events_tests.rs`. If declined,
  tab-config opens via keybinding are still counted under the existing event.

### 7. Feature flag

- `FeatureFlag::ConfigKeybindings` in `crates/warp_features/src/lib.rs`, on in `DOGFOOD_FLAGS`
  (`:1002`). `config_keybindings::init` returns early when disabled, so nothing registers and
  stored `keybindings.yaml` entries are inert (product invariant 14). Runtime check, not `cfg`,
  per AGENTS.md.

### 8. Documentation shipped with the app

- `resources/bundled/skills/change-keybinding/SKILL.md` → "Identifying the action": document the
  two name patterns and that the description in Settings is
  `Open launch configuration "<name>"` / `Open tab config "<name>"`, so the agent can write the
  key directly when the user names a config.

## End-to-end flow

1. Startup: `WarpConfig::new` kicks off the async load; `launch()` applies `keybindings.yaml`
   through `set_custom_trigger`, which now also records into `Keymap.custom_triggers` (§2).
2. The load completes and emits `WarpConfigUpdateEvent::LaunchConfigs`. `config_keybindings`
   builds one `EditableBinding` per config and calls `replace_editable_bindings_with_prefix`
   (§3). Registration applies the remembered trigger for `"launch_config:open:backend"`.
3. The user presses `ctrl-alt-1` with a terminal focused. The matcher resolves the binding in the
   `Workspace` context and dispatches `WorkspaceAction::OpenLaunchConfigNamed { name: "backend" }`.
4. `Workspace::handle_action` resolves the config from `WarpConfig` and dispatches
   `root_view:open_launch_config` with `open_in_active_window: true`;
   `open_launch_config` → `open_launch_config_window` appends and focuses the tab.
5. The user edits `backend.yaml` → the watcher fires → `LaunchConfigs` → step 2 repeats; the
   binding is re-registered with the same name and keeps its trigger. Deleting the file removes
   the binding; the `keybindings.yaml` entry stays and re-applies if the file returns.
6. Settings → Keyboard Shortcuts: `on_page_selected` lists the config actions; assigning a key
   goes through `confirm_keystroke_editing` → `set_custom_keybinding` → file + keymap, exactly as
   for built-ins.

## Testing and validation

### Unit tests

- `crates/warpui_core/src/keymap_tests.rs`
  - `test_custom_trigger_applies_to_binding_registered_later` (invariant 5/11 mechanics): set a
    trigger for an unregistered name, register, assert the lens trigger.
  - `test_replace_editable_bindings_with_prefix` : register `a:x`, `cfg:1`, `cfg:2`; replace
    `cfg:` with `cfg:2`, `cfg:3`; assert `get_binding_by_name` for all five names, that
    `custom_action_bindings` no longer contains the dropped ones, and that a custom trigger for
    `cfg:2` survives the replace.
  - `test_editable_binding_owned_name`: an `EditableBinding::new(String::from(…))` round-trips
    through `bindings()` and `get_binding_by_name`.
- `app/src/config_keybindings_tests.rs`
  - Name/description derivation for launch configs and tab configs (invariants 1, 2, 5, 15).
  - Case-insensitive duplicate collapse keeps the first (invariant 12); non-UTF-8 stem skipped
    (invariant 16).
  - Feature flag off → no bindings (invariant 14), using the existing `FeatureFlag` test helpers.
- `app/src/workspace/view_tests.rs`: `OpenLaunchConfigNamed` with an unknown name adds one
  ephemeral toast and dispatches nothing (invariant 9).

### Integration tests (`crates/integration/src/test/launch_configs.rs`, registered in
`crates/integration/tests/integration/ui_tests.rs`)

- `test_open_launch_config_via_keybinding` (invariants 6, 8): setup writes a single-window,
  single-tab launch config whose `cwd` is a temp dir into `launch_configs_dir()` and
  `"launch_config:open:<name>": ctrl-alt-1` into `keybinding_file_path()` (precedents
  `launch_configs.rs:27`, `test.rs:1659-1668`); wait for the palette data source to see the
  config; `.with_keystrokes(&["ctrl-alt-1"])`; assert `assert_num_windows_open(1)`,
  `assert_tab_count(2)`, focused tab index 1; run `pwd` in it and `validate_block_output` against
  the temp dir.
- `test_open_tab_config_via_keybinding` (invariant 7): same shape with a param-less
  `tab_configs_dir()` TOML and `"tab_config:open:<stem>": ctrl-alt-2`.
- `test_config_keybinding_follows_file_lifecycle` (invariants 10, 11): after the first open,
  delete the config file, wait for reload, press the key, assert tab count unchanged; restore the
  file, wait, press, assert a tab was added.
- `test_config_keybinding_conflict_shown_in_settings` (invariant 4): open Settings → Keyboard
  Shortcuts (`KeybindingsView` is already reached in `test.rs:2909`), assign a key already used by
  `workspace:new_tab`, assert the conflict indicator on both rows.

### Manual verification (attach to the implementation PR)

- Screen recording: create the two example configs from `product.md`, open Settings → Keyboard
  Shortcuts, search "launch", assign `ctrl-alt-1` and `ctrl-alt-2`, show `keybindings.yaml`,
  press both keys with a terminal focused, show the resulting tabs' `pwd`.
- Delete `backend.yaml` while Settings is open → row disappears; recreate → row returns with the
  shortcut.
- Multi-window launch config bound to a key → opens new windows as today.
- Toggle `FeatureFlag::ConfigKeybindings` off → rows gone, key inert.

### Tooling

`./script/presubmit` (fmt, clippy `-D warnings`, `cargo nextest run`). The `Cow<'static, str>`
change touches every `EditableBinding::new` call only through type inference; no call-site edits
are expected, which presubmit confirms.

## Risks and mitigations

- **Index invalidation in `Keymap`.** Removing entries shifts `Vec` indices that
  `editable_bindings_by_name` stores. Mitigation: rebuild the index inside
  `replace_editable_bindings_with_prefix` and cover it with the unit test above; the only mutation
  paths remain `register_*` and this one function.
- **macOS menu cache.** `editable_custom_action_bindings` is a filtered copy consulted from
  `menuNeedsUpdate`. Config bindings never carry `Trigger::Custom`, but the replace path still
  prunes the copy so a future `with_custom_action` on them cannot leave stale menu entries.
- **Startup ordering.** Covered by §2; without it, shortcuts would work only after the user
  re-saved them in Settings. The unit test `test_custom_trigger_applies_to_binding_registered_later`
  guards the invariant.
- **Name collisions with built-in namespaces.** `launch_config:` and `tab_config:` are not used
  by any existing binding (all built-ins are `workspace:`, `terminal:`, `editor_view:`, …). A
  debug assertion in `config_keybindings` rejects a prefix that already has registrations.
- **Rename semantics.** Renaming a config changes its identity and orphans the stored shortcut
  (invariant 11). This matches URIs and is called out in the SKILL.md update; the follow-up
  `keybinding:` field would make shortcuts travel with the file.
- **Behavioral change for keyboard path only.** `open_in_active_window: true` is new only for the
  keyboard entry point; palette, menu, and URI behavior is untouched.

## Follow-ups

- `keybinding:` field in launch config YAML / tab config TOML as a default trigger (product open
  question 2): parse with `Keystroke::parse` (`keymap.rs:897`), pass as `.with_key_binding(…)`
  in §5, and surface parse errors through the existing `TabConfigErrors` toast path.
- Show the assigned shortcut next to each config in the `+` menu and the launch-configuration
  palette rows, the way built-in palette entries show theirs.
- Public docs (docs.warp.dev keybindings and launch-configuration pages).
