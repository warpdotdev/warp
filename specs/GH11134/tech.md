# Tech Spec: Boolean parameters for tab configs

**Issue:** [warpdotdev/warp#11134](https://github.com/warpdotdev/warp/issues/11134)

**Product spec:** [`specs/GH11134/product.md`](product.md)

**Code researched at:** [`9e19f0741e3224c1bf8311c0223fd5f4d4a2e260`](https://github.com/warpdotdev/warp/tree/9e19f0741e3224c1bf8311c0223fd5f4d4a2e260)

## Context

Tab config parameters cross three boundaries today: TOML deserialization,
modal-owned field state, and string-only Handlebars contexts. The change should
introduce a typed default at the schema boundary, then deliberately normalize a
valid boolean to `"true"` or `"false"` before it enters the existing rendering
pipeline.

Relevant current code:

- [`app/src/tab_configs/tab_config.rs:61-92 @ 9e19f07`](https://github.com/warpdotdev/warp/blob/9e19f0741e3224c1bf8311c0223fd5f4d4a2e260/app/src/tab_configs/tab_config.rs#L61-L92)
  defines `TabConfigParamType::{Text, Branch, Repo}` and stores every default as
  `Option<String>`.
- [`app/src/tab_configs/tab_config.rs:166-177 @ 9e19f07`](https://github.com/warpdotdev/warp/blob/9e19f0741e3224c1bf8311c0223fd5f4d4a2e260/app/src/tab_configs/tab_config.rs#L166-L177)
  converts missing defaults to empty strings for the direct-open fallback.
- [`app/src/tab_configs/tab_config.rs:206-265 @ 9e19f07`](https://github.com/warpdotdev/warp/blob/9e19f0741e3224c1bf8311c0223fd5f4d4a2e260/app/src/tab_configs/tab_config.rs#L206-L265)
  builds raw title/directory and shell-quoted command contexts from a
  `HashMap<String, String>`.
- [`app/src/tab_configs/params_modal.rs:60-105 @ 9e19f07`](https://github.com/warpdotdev/warp/blob/9e19f0741e3224c1bf8311c0223fd5f4d4a2e260/app/src/tab_configs/params_modal.rs#L60-L105)
  resolves blank values and models modal fields as an editor, branch picker, or
  repository picker.
- [`app/src/tab_configs/params_modal.rs:183-329 @ 9e19f07`](https://github.com/warpdotdev/warp/blob/9e19f0741e3224c1bf8311c0223fd5f4d4a2e260/app/src/tab_configs/params_modal.rs#L183-L329)
  sorts fields, constructs the appropriate child view, seeds it from the string
  default, and chooses initial focus.
- [`app/src/tab_configs/params_modal.rs:394-453 @ 9e19f07`](https://github.com/warpdotdev/warp/blob/9e19f0741e3224c1bf8311c0223fd5f4d4a2e260/app/src/tab_configs/params_modal.rs#L394-L453)
  owns focus traversal and emits submitted values as
  `HashMap<String, String>`.
- [`app/src/tab_configs/params_modal.rs:480-625 @ 9e19f07`](https://github.com/warpdotdev/warp/blob/9e19f0741e3224c1bf8311c0223fd5f4d4a2e260/app/src/tab_configs/params_modal.rs#L480-L625)
  computes submit availability and renders labels, descriptions, default hints,
  and child fields.
- [`app/src/workspace/view.rs:6862-6903 @ 9e19f07`](https://github.com/warpdotdev/warp/blob/9e19f0741e3224c1bf8311c0223fd5f4d4a2e260/app/src/workspace/view.rs#L6862-L6903)
  consumes the string map and renders the config; no workspace API needs a
  boolean-specific value.
- [`app/src/user_config/util.rs:170-189 @ 9e19f07`](https://github.com/warpdotdev/warp/blob/9e19f0741e3224c1bf8311c0223fd5f4d4a2e260/app/src/user_config/util.rs#L170-L189)
  turns TOML failures into `TabConfigError`, while
  [`app/src/user_config/native.rs:130-140 @ 9e19f07`](https://github.com/warpdotdev/warp/blob/9e19f0741e3224c1bf8311c0223fd5f4d4a2e260/app/src/user_config/native.rs#L130-L140)
  emits those errors after a watched file changes.
- [`crates/warpui_core/src/ui_components/checkbox.rs:25-120 @ 9e19f07`](https://github.com/warpdotdev/warp/blob/9e19f0741e3224c1bf8311c0223fd5f4d4a2e260/crates/warpui_core/src/ui_components/checkbox.rs#L25-L120)
  provides the themed checkbox primitive. It is stateless and expects its caller
  to retain the checked value and `MouseStateHandle`.
- [`crates/warpui_core/src/core/view/mod.rs:62-83 @ 9e19f07`](https://github.com/warpdotdev/warp/blob/9e19f0741e3224c1bf8311c0223fd5f4d4a2e260/crates/warpui_core/src/core/view/mod.rs#L62-L83)
  and [`crates/warpui_core/src/accessibility.rs:150-224 @ 9e19f07`](https://github.com/warpdotdev/warp/blob/9e19f0741e3224c1bf8311c0223fd5f4d4a2e260/crates/warpui_core/src/accessibility.rs#L150-L224)
  provide focus announcements and a `CheckboxRole`; action-specific
  announcements are available through `TypedActionView`.
- [`resources/bundled/skills/tab-configs/SKILL.md:70-89 @ 9e19f07`](https://github.com/warpdotdev/warp/blob/9e19f0741e3224c1bf8311c0223fd5f4d4a2e260/resources/bundled/skills/tab-configs/SKILL.md#L70-L89)
  is the bundled canonical schema reference and currently documents only the
  three string-valued parameter types.

## Proposed changes

### 1. Make boolean defaults typed at the TOML boundary

In `app/src/tab_configs/tab_config.rs`:

- Add `Boolean` to `TabConfigParamType`. `rename_all = "snake_case"` makes
  `boolean` the only serialized spelling; do not add a `checkbox` alias.
- Introduce a serializable/deserializable untagged default value enum with
  `String(String)` and `Boolean(bool)` variants, and change
  `TabConfigParam::default` to `Option<TabConfigParamDefault>`.
- Deserialize `TabConfigParam` through a private raw representation and validate
  the `(param_type, default)` pair before constructing the public value:
  `Boolean` accepts a missing or boolean default, while the other three types
  accept a missing or string default. Return a Serde custom error that names the
  expected TOML type. This fails during the existing `from_toml` load, so the
  existing file-specific `TabConfigError` path handles invariant 13 without a
  second validation surface.
- Add narrow accessors for string defaults, boolean defaults, and the effective
  interpolation default. The effective value is lowercase `bool::to_string()`;
  a missing boolean default returns `"false"`, while a missing string default
  preserves the existing empty/required behavior.
- Update `TabConfig::default_param_values` to use that accessor. Keep its
  `HashMap<String, String>` return type so direct-open and workspace rendering
  remain unchanged.
- Update the programmatic text-param construction in
  `app/src/tab_configs/session_config.rs` and test fixtures to wrap string
  defaults in the new enum. Derived serialization must keep existing string
  defaults quoted and serialize boolean defaults as unquoted TOML booleans.

Encoding the default as a typed enum is preferable to storing all defaults as
strings and coercing later: it preserves correct TOML round-trips and prevents a
misspelled schema from becoming a plausible but wrong command value.

### 2. Add a focusable boolean field view

Add `app/src/tab_configs/boolean_param.rs` as a small child view owned by the
params modal:

- Declare the module from `app/src/tab_configs/mod.rs`. Register its fixed
  bindings from the existing `params_modal::init` entry point so app startup
  does not gain a second tab-config initialization call.
- Store `checked: bool`, the parameter name/optional description, a persistent
  `MouseStateHandle`, and whether the child currently has keyboard focus.
- Render the existing themed checkbox primitive with a clickable visible name.
  Apply the modal's normal label/description typography and a visible focus
  treatment; the params modal must not render a second copy of those labels for
  this variant.
- Handle mouse activation and a typed `Toggle` action. Register `Space` for that
  action when the boolean field is focused. Emit navigation events for `Tab` and
  `Shift-Tab` so the parent can reuse its indexed field traversal; allow `Enter`
  and `Escape` to continue to the modal's existing bindings.
- Implement `View::accessibility_contents` with `WarpA11yRole::CheckboxRole`.
  Include the parameter name and checked state in `value`, the description plus
  `Space` instruction in `help`, and announce the resulting state from
  `TypedActionView::action_accessibility_contents` after a toggle. The state is
  part of the announcement because Warp's current role enum has no separate
  checked-state property.

Keeping boolean interaction in a child view matches the editor/picker ownership
model and gives the control a real focus target. An inline checkbox element in
`TabConfigParamsModal::render` would not, by itself, satisfy the keyboard or
focus-announcement invariants.

### 3. Integrate the field without changing the submission contract

In `app/src/tab_configs/params_modal.rs`:

- Add `ParamField::Boolean(ViewHandle<BooleanParamField>)`; initialize it from
  the effective boolean default and subscribe to its navigation/change events.
- Extend the exhaustive type-priority match to
  `Repo`, `Branch`, `Boolean`, `Text`, preserving alphabetical order within a
  type.
- Extend `current_value` to return `checked.to_string()`, `focus_field` to focus
  the boolean child, and the form renderer to mount the child as the complete
  boolean row.
- Treat a boolean as always resolved. Text/picker blank/default resolution and
  submit-button behavior remain unchanged, while a boolean-only modal is
  immediately submittable.
- Keep `TabConfigParamsModalEvent::Submit` and
  `Workspace::open_tab_config_with_params` string-based. Once the modal emits
  `"true"` or `"false"`, `build_template_contexts` applies the current raw versus
  shell-quoted context split, covering title, directory, and commands without a
  boolean-specific Handlebars path.
- Clear the child handles in the existing `on_close` path. A new `on_open`
  rebuilds them from the config defaults, while an already-open modal remains
  isolated from later file-watcher reloads because it owns a cloned `TabConfig`.

No workspace, pane-template, Handlebars, or shell-quoting API changes are
required.

### 4. Update the canonical bundled schema documentation

Update `resources/bundled/skills/tab-configs/SKILL.md` to:

- List `"boolean"` as the fourth parameter type.
- State that its default is an unquoted TOML boolean and defaults to `false` when
  omitted, while the other parameter defaults remain strings.
- Document lowercase `true`/`false` interpolation and show a shell comparison
  example rather than implying Warp conditionally skips commands.

The new-config and default-worktree templates do not need a boolean parameter,
so they remain unchanged.

## Testing and validation

### Automated tests

Extend `app/src/tab_configs/tab_config_tests.rs` with schema and rendering tests:

- Parse `boolean` with `default = true`, `default = false`, and no default;
  assert the effective values. This covers product invariants 1-2 and 9.
- Reject `type = "checkbox"`, a quoted boolean default, a boolean default on each
  string-valued type, and numeric defaults; assert the error text identifies the
  mismatch. This covers invariants 1, 3, and 13.
- Serialize/deserialize both a boolean parameter and an existing text parameter;
  assert that the former remains an unquoted boolean and the latter remains a
  quoted string. This covers invariants 2-3 and 14.
- Render checked and unchecked values through title, directory, and command
  templates; assert exact lowercase `true`/`false` output and the existing
  command quoting behavior. This covers invariants 10-11.
- Keep all existing parsing, default, and interpolation tests passing to guard
  invariant 14.

Extend `app/src/tab_configs/params_modal_tests.rs` and add
`app/src/tab_configs/boolean_param_tests.rs`, included from the new module with
the repository's separate-test-file convention:

- Open boolean-only and mixed configs in `App::test`; verify initial checked
  state, `Repo -> Branch -> Boolean -> Text` ordering, independent state for
  multiple booleans, and immediate submit eligibility. This covers invariants
  4-6 and 9.
- Exercise typed toggle/navigation actions and emitted submit data; verify
  `Space`, `Tab`, `Shift-Tab`, `Enter`, and `Escape` preserve their specified
  responsibilities. This covers invariants 7 and 12.
- Close and reopen the modal and assert the value resets to the config default;
  keep the current value when an unrelated config clone is reloaded. This covers
  invariant 12.
- Assert focus and toggle accessibility content contains the name, state, help,
  and `CheckboxRole`. This provides structural coverage for invariant 8; native
  screen-reader output remains a manual check.
- Render a long name/description at the modal height limit to ensure the control
  remains in the scrollable form and its saved position is reachable. This
  covers invariant 15.

Run at minimum:

```sh
cargo nextest run -p warp --lib tab_configs::tab_config::tests
cargo nextest run -p warp --lib tab_configs::params_modal::tests
./script/format
cargo clippy --workspace --all-targets --all-features --tests -- -D warnings
```

### Manual validation

1. Add a config containing checked, unchecked, and omitted-default booleans plus
   one required text field. Capture before/after screenshots of the modal in
   light and dark themes; verify scrolling and long-description wrapping
   (invariants 4-6, 9, and 15).
2. Traverse the complete mixed form using only `Tab`/`Shift-Tab`, toggle each
   boolean with `Space`, submit with `Enter`, and cancel/reopen with `Escape`.
   Record the interaction because this is an interactive control (invariants
   7 and 12).
3. With VoiceOver enabled, focus and toggle the checkbox; verify the label,
   checked state, description/help, and resulting state are announced
   (invariant 8).
4. Use a command that prints `{{boolean_param}}` and a separate explicit shell
   comparison. Verify the terminal receives exact `true`/`false` values for both
   checkbox states, and verify title/directory interpolation with the same
   config (invariants 10-11).
5. Save configs containing `type = "checkbox"`, `default = "true"` for a
   boolean, and `default = true` for a text parameter. Verify each config is
   excluded and the existing error UI identifies the file and mismatch
   (invariants 1, 3, and 13).
6. Reopen representative existing text, branch, and repository configs and
   confirm their modal, defaults, and rendered commands are unchanged
   (invariant 14).

## Risks and mitigations

- **Typed-default refactor touches current string-param constructors.** Update all
  struct literals in the same change and rely on exhaustive compiler errors plus
  string round-trip regression tests; do not add permissive `From<bool>` or
  coercion paths.
- **A checkbox primitive alone is not keyboard accessible.** Keep focus,
  keybindings, and accessibility announcements in the dedicated child view and
  verify them with keyboard-only and VoiceOver passes.
- **`true`/`false` are values, not a portable conditional language.** Preserve
  one representation and document explicit shell comparison; do not add
  shell-specific spellings or implicit command filtering.
- **Field-order changes can disturb modal focus.** Make the priority function
  exhaustive and unit-test both visual order and forward/backward traversal for
  a mixed form.

## Parallelization

Implementation should stay on one branch. The schema helpers and modal are
tightly coupled through the new default type and field-state contract, while
both test files will evolve with those APIs; separate worktrees would add merge
churn without a useful independent boundary. Documentation and manual
validation can start after the schema/UI behavior is stable, and the final
format, Clippy, and test runs must be sequential on the integrated branch.
