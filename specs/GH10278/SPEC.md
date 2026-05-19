# Option to clamp/disable truecolor BCE bg painting (GH-10278)

## Summary

Add a setting that controls how Warp renders truecolor (24-bit) ANSI background
codes (`\x1b[48;2;R;G;Bm`). Today, Warp paints the truecolor background
edge-to-edge across the block width on every emitting line. Tools that emit
contiguous bg-colored lines (Claude Code's diff renderer, `delta`, `git diff`
with custom config) cause the viewport to fill with a wall of color and read
as if the entire screen is tinted.

The new setting offers three modes: keep current behavior (`flood`), paint
only behind printed glyphs (`clamp_to_text`), or skip truecolor bg painting
entirely (`disabled`).

## Problem

- `\x1b[48;2;R;G;Bm` is widely emitted by modern diff/highlight tools.
- Warp applies the active bg color to the rest of the line/block per BCE
  semantics, producing large flat color rectangles.
- For multi-line diffs, the cumulative effect is perceived as a full-screen
  tint that overwhelms the rendered content.
- Users want a way to dial this back without disabling truecolor or losing fg
  highlighting.

## Goals

- A first-class setting controlling truecolor bg painting with three modes:
  `flood`, `clamp_to_text`, `disabled`.
- Surface the setting in Settings UI under Terminal → Appearance.
- Allow a per-pane override via tab config.
- Apply changes to new output without restart.

## Non-Goals

- Not changing 8-bit / 16-color BCE behavior.
- Not changing 256-color (`\x1b[48;5;Nm`) handling.
- Not changing how palette colors are configured.
- Not changing inline background semantics inside a printed character cell.
- Not retroactively repainting existing scrollback when the setting changes.
- Not changing Warp's own block-level diff highlight (non-ANSI driven).

## Behavior Contract

### B1. Setting enum

The user-level setting key is **`appearance.terminal.truecolor_background_mode`**
(matches the verified `appearance.*` namespace in `app/src/settings/pane.rs`
and is the canonical key used everywhere in this spec — Settings/API, UI,
acceptance, and tests). Enum values: `"flood"` (default), `"clamp_to_text"`,
`"disabled"`.

### B2. Mode `flood` (default)

Unchanged behavior — `\x1b[48;2;R;G;Bm` paints from cursor to end-of-line /
end-of-block per current rules. This preserves backwards compatibility for
users who rely on the existing visual.

### B3. Mode `clamp_to_text`

When a line ends with the truecolor bg still active, the background paints
only the cells that contain **explicitly written characters** — including
explicitly written spaces (`\x20`) emitted by the program while truecolor
bg was active. The bg-paint applies through the explicit space.

Cells **without** a written character — implicit blank cells produced by
EOL flood, erase ops, scroll-region reveals, insert/delete-line shifts,
and block-finalization padding — render with the terminal default
background. Implicit BCE fill is what `clamp_to_text` suppresses.

This resolves the prior Open Question and supersedes any earlier
acceptance language that conflicted with it: explicit spaces count as
painted cells; only IMPLICIT blank cells are clamped to default bg.

### B3a. Bg-fill paths covered by the mode

The truecolor bg-fill suppression principle applies **uniformly** to all
sources of bg-painted blank cells, not just cursor-to-EOL on `\r\n`. The
mode is enforced at every site where the renderer would otherwise commit
a bg-colored blank cell driven by an active truecolor `\x1b[48;2;R;G;Bm`
state.

Single guiding principle: **all bg-painted blank-cell production paths
follow the active `appearance.terminal.truecolor_background_mode` (the
canonical key declared in B1; this is the same path used everywhere in
this spec — Settings/API, UI, acceptance, and tests). Modes apply
uniformly across all sources of bg-painted cells.**

The covered paths are:

1. **Cursor-to-EOL on `\r\n` / `\n` / line wrap.** The original spec
   case. Behavior:
   - `flood`: paint with current truecolor bg from cursor to right edge.
   - `clamp_to_text`: do not paint — cells render as default bg.
   - `disabled`: do not paint — cells render as default bg.
2. **Erase ops: Erase In Line `\x1b[K`, `\x1b[1K`, `\x1b[2K`; Erase In
   Display `\x1b[J`, `\x1b[1J`, `\x1b[2J`, `\x1b[3J`.** When truecolor
   bg is active at the time of the erase, the spec's mode rules apply
   to the cleared cells:
   - `flood`: cleared cells take the current truecolor bg.
   - `clamp_to_text`: cleared cells render as default bg (no truecolor
     bg-paint of cells without an explicit write).
   - `disabled`: cleared cells render as default bg.
3. **Insert/Delete Line (`\x1b[L`, `\x1b[M`).** Cells shifted in by these
   ops follow the mode:
   - `flood`: shifted-in cells paint with current truecolor bg.
   - `clamp_to_text`: shifted-in cells render as default bg.
   - `disabled`: shifted-in cells render as default bg.
4. **Scroll-up / scroll-down regions (DECSTBM and friends).** Cells
   revealed by scrolling follow the mode:
   - `flood`: revealed cells paint with current truecolor bg.
   - `clamp_to_text`: revealed cells render as default bg.
   - `disabled`: revealed cells render as default bg.
5. **Block finalization padding.** When Warp finalizes a command block
   (e.g., adds bottom padding rows beneath the last line of output),
   padding cells follow the mode under the bg state active at finalize
   time:
   - `flood`: padding cells paint with the active truecolor bg.
   - `clamp_to_text`: padding cells render as default bg.
   - `disabled`: padding cells render as default bg.
6. **Cursor save/restore (DECSC / DECRC, `\x1b[s` / `\x1b[u`).** Saving
   and restoring cursor state does **not** alter the active mode and
   does not itself paint bg cells. The mode that applies after a
   restore is the currently configured
   `appearance.terminal.truecolor_background_mode` (canonical key from
   B1; all references in this spec use the full `appearance.*` form).
7. **256-color and 16-color BCE.** Out of scope of this setting in all
   modes — see B7. Only truecolor (24-bit) bg fills are gated.

### B4. Mode `disabled`

Truecolor bg ANSI codes are PARSED but NOT rendered as background fill. The
cell foreground continues to use any active fg color. State is treated as if
the user had emitted `\x1b[49m` (default bg) for the affected cells.

### B5. Per-pane override

Setting applies to all terminal panes by default. Power users running mixed
workloads can override the mode on a specific pane via the tab-config TOML
schema in `~/.warp/tab_configs/*.toml`.

The verified schema (`app/src/tab_configs/tab_config.rs`,
`TabConfigPaneNode`) stores pane fields as flat keys inside each `[[panes]]`
array entry — there is **no** `[pane]` table. The override field is added
as `truecolor_background_mode` directly on the pane entry. Exact valid TOML:

```toml
name = "Mixed mode example"

[[panes]]
id = "main"
type = "terminal"
truecolor_background_mode = "clamp_to_text"   # this pane only

[[panes]]
id = "logs"
type = "terminal"
# no key here — this pane inherits the user-level setting
```

Field semantics:

- Field name on `TabConfigPaneNode`: `truecolor_background_mode`.
- Type: `Option<TruecolorBackgroundMode>`. Absent / `None` ⇒ inherit the
  user-level `appearance.terminal.truecolor_background_mode`.
- Allowed values when present: `"flood"` | `"clamp_to_text"` | `"disabled"`.
- Resolution: per-pane value (when `Some`) wins over the user-level setting
  for that pane only; sibling panes are unaffected.
- The `TabConfigPaneNode` struct is `#[serde(deny_unknown_fields)]`, so the
  field must be added to the struct itself; it cannot be set via a nested
  `[pane]` table.

### B6. Live application

The setting applies to NEW output from the moment of change. Already-rendered
scrollback is NOT retroactively repainted; the toggle effect is "from now on".

### B7. Other color depths unchanged

256-color (`\x1b[48;5;Nm`) and 16-color BCE behavior is UNCHANGED in all
modes — only truecolor (24-bit) bg is gated by this setting.

### B8. Block-level highlight unaffected

Warp's own per-line block colors that are not driven by ANSI (e.g. command
output highlight, error block tint) are unaffected by this setting.

## Settings / API surface

### User-level setting — verified against existing schema

The setting is registered in `app/src/settings/` using the existing
`define_settings_group!` macro pattern (sibling examples: `pane.rs`,
`accessibility.rs`, `font.rs`). It lives on the terminal-appearance
settings group and follows the existing `appearance.terminal.*` /
`appearance.panes.*` namespace conventions.

- Setting name: `truecolor_background_mode`.
- Type: enum (`flood` | `clamp_to_text` | `disabled`).
- Default: `flood`.
- Serde representation: lowercase string (`"flood"`, `"clamp_to_text"`,
  `"disabled"`).
- TOML path: `appearance.terminal.truecolor_background_mode` (parallels
  the verified `appearance.panes.should_dim_inactive_panes` style in
  `app/src/settings/pane.rs`).
- New module: `app/src/settings/truecolor_background_mode.rs`, wired
  into the terminal-appearance settings group via the same
  `define_settings_group!` registration pattern used by `pane.rs`.
- `SupportedPlatforms::ALL`, `SyncToCloud::Globally(RespectUserSyncSetting::Yes)`,
  `private: false` — matching the conventions of similar appearance
  settings.

### Per-pane / tab-config override — verified against existing schema

The per-pane override lives on the tab-config schema in
`app/src/tab_configs/tab_config.rs`. Tab configs already host pane-level
options on `TabConfigPaneNode` / related structs; the override is added
there:

- Field name: `truecolor_background_mode`.
- Type: `Option<TruecolorBackgroundMode>` (None = inherit user setting).
- Serde representation: lowercase string when present.
- TOML key on a pane node: `truecolor_background_mode` (a flat key on a
  `[[panes]]` array entry — see B5 for the full TOML example). There is no
  `[pane]` table in the tab-config schema.
- Re-uses the enum defined in the new
  `app/src/settings/truecolor_background_mode.rs` module so the user
  setting and tab-config override share a single type.
- Override resolution: per-pane value, if set, wins over the user-level
  setting for that pane only. Sibling panes are unaffected.

### Settings UI

Settings → Terminal → Appearance → "Truecolor background painting"
radio group:

- `Flood (default)` — paints the full line.
- `Clamp to text` — paints only behind explicitly written cells (incl.
  explicit spaces).
- `Disabled` — does not paint truecolor backgrounds.

## Acceptance Criteria

- A1: Default mode is `flood` and matches current rendering byte-for-byte for
  a recorded `vim_24bitcolors_bce` reference test.
- A2: `clamp_to_text` does not paint cells past the last **explicitly
  written** cell on multi-line `git diff` colored output (i.e., implicit
  EOL fill is suppressed).
- A_clamp_explicit_spaces_painted: In `clamp_to_text`, an explicitly
  written space (`\x20`) emitted while truecolor bg is active **is**
  painted with the truecolor bg. Sequence
  `\x1b[48;2;10;20;30m  text  \x1b[0m` paints all 4 spaces and the
  `text` glyphs; only cells beyond the trailing explicit spaces fall
  back to default bg.
- A_clamp_implicit_blank_not_painted: In `clamp_to_text`, cells produced
  by **implicit** bg-fill paths — EOL flood after `\r\n` / wrap, erase
  ops (`\x1b[K`/`\x1b[J` and variants), insert/delete-line shifted-in
  cells, scroll-region revealed cells, and block-finalization padding
  rows — render as default bg even while truecolor bg is the active
  state.
- A3: `disabled` renders truecolor bg cells with the terminal default
  background; foreground colors remain intact.
- A4: 256-color bg output is unchanged in `disabled` mode.
- A5: Setting changes take effect for new output without restart.
- A6: Already-rendered scrollback is not repainted on toggle.
- A7: A per-pane tab-config override declared as a flat
  `truecolor_background_mode = "<mode>"` key inside a `[[panes]]` array
  entry correctly overrides the user-level
  `appearance.terminal.truecolor_background_mode` for the affected pane only.

## Implementation Pointers

Verified paths (via `git ls-files` and `ls`):

- ANSI parser / handler: `app/src/terminal/model/ansi/handler.rs`,
  `app/src/terminal/model/ansi/mod.rs`,
  `app/src/terminal/model/grid/ansi_handler.rs`.
- Crate-level ANSI: `crates/warp_terminal/src/model/ansi/mod.rs`,
  `crates/warp_terminal/src/model/ansi/control_sequence_parameters.rs`.
- Existing truecolor BCE reference test fixtures:
  `app/src/terminal/ref_tests/data/vim_24bitcolors_bce/` (use as the
  flood-mode regression baseline).
- Settings module (verified): existing files like
  `app/src/settings/pane.rs`, `app/src/settings/accessibility.rs`,
  `app/src/settings/font.rs` use `define_settings_group!` (from
  `app/src/settings/macros.rs`) with `toml_path` strings under the
  `appearance.*` namespace. The new
  `app/src/settings/truecolor_background_mode.rs` follows the same
  pattern with `toml_path: "appearance.terminal.truecolor_background_mode"`.
- Tab-config schema (verified): `app/src/tab_configs/tab_config.rs`
  hosts `TabConfig`, `TabConfigPaneNode`, etc. The per-pane override
  field is added on the pane-node struct and (de)serialized at
  `pane.truecolor_background_mode`. Tests live alongside in
  `app/src/tab_configs/tab_config_tests.rs`.

Likely change shape:

1. Add the enum + serde + default in the new settings module
   `app/src/settings/truecolor_background_mode.rs` using
   `define_settings_group!` with `toml_path:
   "appearance.terminal.truecolor_background_mode"`.
2. Add an `Option<TruecolorBackgroundMode>` field on the relevant
   pane-node struct in `app/src/tab_configs/tab_config.rs` so per-pane
   override is parsed at `pane.truecolor_background_mode`.
3. Thread the resolved mode (per-pane override > user setting > default)
   into the ANSI handler / grid renderer at every bg-fill site
   enumerated in B3a — cursor-to-EOL, erase ops, insert/delete line,
   scroll regions, block-finalization padding — gating the BCE fill
   path on mode at each site rather than only at line-end.
4. Wire the Settings UI control under Terminal → Appearance.

## Tests

- T1: Flood mode matches `vim_24bitcolors_bce` recorded grid (regression).
- T2: `clamp_to_text` against a synthetic multi-line diff recording — assert
  cells beyond the last **explicitly written** cell use default bg.
- T3: `disabled` mode — truecolor bg cells render as default bg, fg color
  preserved.
- T4: 256-color bg output unchanged across all three modes.
- T5: Foreground color preserved when truecolor bg is disabled.
- T6: Per-pane override changes a single pane without affecting siblings.
- T7: Setting change applies to subsequent output only.
- T8: Toggling the setting does not repaint scrollback.
- T_clamp_explicit_spaces: In `clamp_to_text`, the sequence
  `\x1b[48;2;10;20;30m  text  \x1b[0m` paints all 4 explicit spaces and
  the `text` glyphs with the truecolor bg. (Aligns with
  A_clamp_explicit_spaces_painted.)
- T_clamp_eol_flood: After `\x1b[48;2;10;20;30mhello\r\n`, in
  `clamp_to_text` the cells from after the `o` to right edge render as
  default bg (no implicit EOL fill).
- T_clamp_erase_in_line: After `\x1b[48;2;10;20;30m\x1b[K` (Erase In
  Line, cursor to EOL), in `clamp_to_text` cleared cells render as
  default bg; in `flood` they take truecolor bg.
- T_clamp_erase_in_display: Same expectations as T_clamp_erase_in_line
  for `\x1b[J` (clear to end of screen) and `\x1b[2J` (clear screen).
- T_clamp_insert_delete_line: After `\x1b[L` / `\x1b[M` with truecolor
  bg active, shifted-in cells render as default bg in `clamp_to_text`
  and `disabled`; truecolor bg in `flood`.
- T_clamp_scroll_region: Cells revealed by scroll-up/scroll-down within
  a DECSTBM region follow the same mode rule.
- T_clamp_block_padding: Block-finalization padding rows render as
  default bg in `clamp_to_text` and `disabled`; truecolor bg in
  `flood`.
- T_per_pane_override_toml: Loading a tab-config TOML where a `[[panes]]`
  entry contains the flat key `truecolor_background_mode = "clamp_to_text"`
  (no `[pane]` table — flat field on the array entry, matching the verified
  `TabConfigPaneNode` schema in `app/src/tab_configs/tab_config.rs`)
  produces a pane whose resolved mode is `clamp_to_text` even when the
  user-level `appearance.terminal.truecolor_background_mode` is `flood`.
  Sibling `[[panes]]` entries without the key inherit the user-level value.

## Open Questions

(The earlier question about whether explicitly written spaces count as
"written" in `clamp_to_text` is now resolved in the spec body — explicit
spaces **do** count as written cells and **are** painted with the
truecolor bg. See B3, A_clamp_explicit_spaces_painted, and
T_clamp_explicit_spaces.)

- None outstanding for V1.

## Telemetry

No new telemetry events. If the setting is changed, the existing settings
update event already records the key change without payload extensions.
