# Product Spec: Boolean parameters for tab configs

**Issue:** [warpdotdev/warp#11134](https://github.com/warpdotdev/warp/issues/11134)

## Summary

Tab config authors can declare a native boolean parameter that is collected with
a checkbox and interpolated as a predictable `true` or `false` string. This
removes the need to model on/off choices as free-form text.

## Problem

Tab config parameters currently support text, branch, and repository values.
Authors who need an on/off choice must ask users to type a magic value such as
`yes`, then duplicate that convention in every command that consumes it. The
modal cannot communicate the two-state nature of the value or prevent typos.

## Goals / Non-goals

Goals:

- Add one canonical boolean parameter type with a native TOML boolean default.
- Make mouse, keyboard, and screen-reader behavior explicit.
- Produce one stable interpolation representation on every supported template
  surface.
- Preserve every currently valid tab config without migration.

Non-goals:

- Adding conditional Handlebars syntax or automatically including/excluding a
  command based on a boolean. Authors continue to write the shell conditional
  appropriate for their command.
- Accepting aliases or coercing `yes`/`no`, `1`/`0`, or quoted strings into
  booleans.
- Remembering a user's checkbox choice between separate launches of a config.
- Changing the behavior of text, branch, or repository parameters.

## Figma

Figma: none provided.

## Behavior

1. A tab config author declares a boolean parameter with the canonical spelling
   `type = "boolean"`:

   ```toml
   [params.set_upstream]
   type = "boolean"
   description = "Set upstream to the base branch"
   default = true
   ```

   `type = "checkbox"` and other spellings are not aliases and remain invalid.

2. A boolean parameter's `default` is an unquoted TOML boolean. Both
   `default = true` and `default = false` are valid. If `default` is omitted, the
   effective default is `false`.

3. A quoted default such as `default = "true"` is invalid for a boolean
   parameter. Conversely, an unquoted boolean default is invalid for text,
   branch, and repository parameters. Warp does not silently coerce either
   mismatch.

4. Opening a config with parameters shows each boolean parameter as a checkbox
   in the existing parameter fill-in modal. The checkbox is checked when its
   effective default is `true` and unchecked when it is `false`.

5. Parameter ordering stays deterministic. Repository parameters appear first,
   followed by branch parameters, boolean parameters, and text parameters;
   parameters of the same type are ordered alphabetically by name.

6. Clicking the checkbox or its visible name toggles only that parameter. The
   new checked state is visible immediately. Multiple boolean parameters retain
   independent state while the modal remains open.

7. Boolean controls participate in the modal's keyboard order. `Tab` and
   `Shift-Tab` move forward and backward through fields in their visible order,
   a focused checkbox has a visible focus treatment, and `Space` toggles it.
   `Enter` keeps the modal's existing submit behavior and `Escape` keeps its
   existing cancel behavior.

8. When a boolean checkbox receives keyboard focus, assistive technology
   announces its parameter name, checked or unchecked state, and that `Space`
   toggles it. The description is included as help text when present. Toggling
   the value announces the resulting state.

9. A boolean parameter is always satisfied because it always has a true or
   false value. A boolean-only form can therefore be submitted immediately.
   In a mixed form, an unsatisfied required text, branch, or repository field
   continues to disable submission exactly as it does today.

10. On submission, `{{set_upstream}}` resolves to the lowercase ASCII string
    `true` when checked and `false` when unchecked. The representation is the
    same in `title`, `directory`, and every entry in `commands`; it never changes
    to `1`/`0`, `yes`/`no`, or platform-specific spellings.

11. Boolean interpolation follows the existing context rules: title and
    directory templates receive the raw `true`/`false` value, while command
    templates receive the same value through the existing shell-quoting path.
    This feature does not interpret the boolean or choose shell syntax for the
    author.

12. Cancelling the modal does not open a tab or persist checkbox changes.
    Reopening the config starts again from its effective defaults. If the config
    file reloads while a modal is already open, the open form keeps its current
    values; a later open uses the reloaded defaults.

13. Invalid boolean declarations use the existing tab config parse-error flow:
    the invalid config is not offered as a runnable config, and the error names
    the affected file and explains the invalid type/default combination. Warp
    never opens a text field as a fallback for an invalid boolean declaration.

14. Every previously valid config remains valid and behaves the same. Omitting
    `type` still means `text`; existing string defaults and their interpolation
    are unchanged; a text parameter whose literal value is `true` or `false`
    remains text.

15. Boolean rows remain usable with long names or descriptions, at modal scroll
    boundaries, and in every supported Warp theme. Text may wrap, but the
    checkbox, checked state, focus treatment, and click target remain visible.
