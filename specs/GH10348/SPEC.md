# Configurable Word Delimiters for Word-Deletion and Word-Navigation Shortcuts (GH-10348)

## Summary

Make the set of characters that act as word boundaries configurable for word-deletion (Delete Word Left, Delete Word Right) and word-navigation (Move Cursor Word Left, Move Cursor Word Right) shortcuts. Today these shortcuts treat path-like strings such as `/var/www/example.com/logs` as a single word, deleting backward all the way to whitespace. With a configurable delimiter set including `/`, `.`, `-`, `_`, `:`, and `=`, Delete Word Left stops at each path segment, matching expectations from VS Code, JetBrains IDEs, and most modern editors. Settings are global with per-input-context overrides (terminal, agent, editor).

## Problem

The four word-cursor shortcuts use a hard-coded boundary definition that effectively only treats whitespace as a delimiter. Real-world inputs are usually paths, URLs, identifiers, or kebab/snake-case strings — none of which are split by whitespace alone:

- `/var/www/example.com/logs` is one "word", so Delete Word Left wipes the entire path.
- `feature-branch-name` is one "word", so Move Cursor Word Right jumps the whole identifier.
- `key=value` is one "word", so users can't quickly correct just the value.

Every modern editor exposes a configurable word-separator set for exactly this reason. Warp does not.

## Goals

- One configurable delimiter set used by **all four** word-cursor shortcuts: Delete Word Left, Delete Word Right, Move Cursor Word Left, Move Cursor Word Right.
- A sensible default set that matches typical editor behavior (path/punct chars plus whitespace).
- Per-input-context overrides so terminal, agent prompt, and editor surfaces can diverge when needed.
- Live-effective changes (no app restart).

## Non-Goals

- Not redefining single-character delete (Backspace, Delete) — only the word-granular shortcuts.
- Not changing line-deletion shortcuts (Delete Line Left/Right, Delete Line).
- Not adding regex- or predicate-based boundary definitions in V1. (Tracked as a V1.5 follow-up — see Open Questions.)
- Not supporting Unicode category-based boundaries in V1 (e.g., "any punctuation"). The setting is a literal character list.

## Behavior Contract

### B1. Global setting and default

A new setting `editor.word_delimiters` (string of characters) defines the global word-boundary set. The literal default character set is:

`/`, `.`, `-`, `_`, `:`, `=`, space, tab (U+0009), newline (U+000A), carriage return (U+000D), vertical tab (U+000B), form feed (U+000C).

Expressed as a valid TOML basic string. The full encoding (including VT/FF
explicitly) uses ONLY TOML-valid escapes — `\t`, `\n`, `\r`, `\f`, and
the 4-hex-digit Unicode form `\u00XX`. **`\xXX` is NOT a valid TOML escape
and MUST NOT appear in this value** (TOML basic strings reject `\xXX`; the
parser will raise an error). See B6.2 for the canonical escape table:

```toml
# Authoritative literal default — TOML-valid encoding of the full set,
# including VT (U+000B via \u000B) and FF (U+000C via \f).
editor.word_delimiters = "/.-_:= \t\n\r\u000B\f"
```

A pragmatic shorter form that omits the rarely-hand-typed VT and FF
characters is also valid; VT and FF are added back at runtime by the B5
whitespace floor regardless of whether the user-supplied value includes
them. Either form is accepted:

```toml
# Equivalent at runtime (VT/FF added back by the B5 whitespace floor).
editor.word_delimiters = "/.-_:= \t\n\r"
```

The default includes path/punct characters (`/`, `.`, `-`, `_`, `:`, `=`) plus all standard whitespace (space, tab, newline, carriage return, vertical tab, form feed). Whitespace is included for back-compat with current behavior.

### B2. Canonical word-deletion algorithm

Let `D` be the resolved **delimiter character set** for the current input context (per B4). Define:

- A **non-delimiter run** is a maximal contiguous sequence of characters NOT in `D`.
- A **delimiter run** is a maximal contiguous sequence of characters IN `D`.

The buffer is therefore an alternating sequence of non-delimiter runs and delimiter runs.

#### Delete Word Left

From the cursor position, walk LEFT and apply the following rules to a single Delete Word Left invocation:

1. If the cursor is at start-of-line, do nothing.
2. If the character immediately LEFT of the cursor is in `D`: consume the entire delimiter run that ends just left of the cursor, **then** consume the non-delimiter run that ends just left of that (if any). Stop at start-of-line.
3. Otherwise (the character immediately LEFT of the cursor is NOT in `D`): consume only the non-delimiter run from the cursor leftward to its start. Do **not** consume any preceding delimiter run on this invocation.

In all cases, do not cross start-of-line.

#### Delete Word Right

Mirror of Delete Word Left:

1. If the cursor is at end-of-line, do nothing.
2. If the character immediately RIGHT of the cursor is NOT in `D`: consume only the non-delimiter run from the cursor rightward to its end. Do **not** consume any following delimiter run on this invocation.
3. Otherwise (the character immediately RIGHT of the cursor is in `D`): consume the entire delimiter run that begins just right of the cursor, **then** consume the non-delimiter run that begins just right of that (if any). Stop at end-of-line.

In all cases, do not cross end-of-line.

#### Worked example: `/var/www/example.com|` (cursor at end)

Using the default `D` (`/`, `.`, `-`, `_`, `:`, `=`, plus the whitespace floor — space, tab, newline, CR, vertical tab, form feed; see B1 for valid TOML encoding):

| Step | Buffer state                        | Action                                                                                       | Result                              |
| ---- | ----------------------------------- | -------------------------------------------------------------------------------------------- | ----------------------------------- |
| 0    | `/var/www/example.com\|`            | start                                                                                        | —                                   |
| 1    | `/var/www/example.com\|`            | char left = `m` (∉ D). Rule 3 → consume `com`.                                              | `/var/www/example.\|`              |
| 2    | `/var/www/example.\|`               | char left = `.` (∈ D). Rule 2 → consume delim run `.`, then non-delim run `example`.        | `/var/www/\|`                       |
| 3    | `/var/www/\|`                       | char left = `/` (∈ D). Rule 2 → consume delim run `/`, then non-delim run `www`.            | `/var/\|`                           |
| 4    | `/var/\|`                           | char left = `/` (∈ D). Rule 2 → consume delim run `/`, then non-delim run `var`.            | `/\|`                               |
| 5    | `/\|`                               | char left = `/` (∈ D). Rule 2 → consume delim run `/`. No preceding non-delim run.          | `\|`                                |
| 6    | `\|`                                | start-of-line. No-op.                                                                        | `\|`                                |

This is the canonical reference. All examples, acceptance criteria, and tests below MUST match this algorithm.

### B3. Word-navigation semantics

Move Cursor Word Left / Right use the **same** delimiter set `D` and the **same** run-walking rules from B2, but they move the cursor instead of removing characters. The cursor lands at the position the equivalent Delete Word Left / Right would have left the cursor (the start of the consumed region for Left; the end for Right).

### B4. Per-context overrides

Three optional overrides exist:

| Setting key                | Applies in            |
| -------------------------- | --------------------- |
| `terminal.word_delimiters` | Terminal input lines  |
| `agent.word_delimiters`    | Agent prompt input    |
| `editor.word_delimiters`   | All editor surfaces (default fallback for the other contexts when their override is unset) |

Resolution order for a given input context:

1. The context-specific override, if present and non-empty.
2. Otherwise, `editor.word_delimiters`.
3. Otherwise, the built-in default from B1.

### B5. Whitespace is always a delimiter

Whitespace characters (space, tab, newline, carriage return, vertical tab, form feed) are **always** treated as delimiters, regardless of the configured set. The `WordBoundaryClassifier` ORs the user's set with the fixed whitespace set, so even if the user provides a value that omits whitespace, whitespace remains in the effective set `D`. This prevents users from accidentally configuring a state where Delete Word Left deletes across line breaks or runs forever.

### B6. Setting-value semantics, validation, and fallback

This subsection is the single source of truth for how the setting value string is interpreted, including escape sequences, validation, and missing/empty/invalid handling. The Settings/API surface and Settings UI sections reference this section.

#### B6.1. Value type and representation

- **Type.** A string of characters. Each codepoint in the string is added to the delimiter set `D`. Order is irrelevant. Duplicates are coalesced (set semantics).
- **Whitespace floor.** Per B5, the effective `D` always also contains the standard whitespace set. The user-provided value adds to that floor; it cannot subtract from it.

#### B6.2. Escape sequences in TOML config

The config file is TOML. The setting key is read as a TOML basic string, which honors **only** the standard escape sequences listed in the [TOML spec](https://toml.io/en/v1.0.0#string):

| Escape          | Meaning                              |
| --------------- | ------------------------------------ |
| `\b`            | Backspace (U+0008)                   |
| `\t`            | Tab (U+0009)                         |
| `\n`            | Newline (U+000A)                     |
| `\f`            | Form feed (U+000C)                   |
| `\r`            | Carriage return (U+000D)             |
| `\"`            | Double quote (U+0022)                |
| `\\`            | Backslash (U+005C)                   |
| `\uXXXX`        | Unicode codepoint, 4 hex digits      |
| `\UXXXXXXXX`    | Unicode codepoint, 8 hex digits      |

**Non-standard `\xXX` is NOT a valid TOML escape** and will fail TOML parsing if used. Earlier drafts of this spec referenced `\x0b` and `\x0c` for vertical tab and form feed; those references were incorrect. The valid encodings are:

- Vertical tab (U+000B) — encode as the 4-hex-digit Unicode escape `\u000B`. TOML has no single-character escape for VT, so this is the only valid form.
- Form feed (U+000C) — encode as `\f` (preferred, single-character) OR `\u000C` (Unicode escape). Both forms are equivalent at parse time.

Copy-pasteable TOML literal that includes the full whitespace floor (space, tab, newline, CR, VT, FF) PLUS the path/punct chars from B1, suitable to paste directly into config:

```toml
editor.word_delimiters = "/.-_:= \t\n\r\u000B\f"
```

A simpler form `"/.-_:= \t\n\r"` (no VT/FF) also works — VT and FF are added back at runtime by the B5 whitespace floor regardless of whether they appear in the user-provided string.

Whitespace can be entered literally inside the string OR via the corresponding escape. The two are equivalent.

Example (TOML), showing a valid encoding of the literal default:

```toml
editor.word_delimiters = "/.-_:= \t\n\r"
```

The hyphen in TOML basic strings is just a literal hyphen — no escape is needed.

#### B6.3. Settings UI (single-line text field)

The Settings text field is a SINGLE-LINE text input (no multi-line
textarea). It accepts characters either literally (e.g., typing `/`) or
as escape sequences (e.g., typing `\t`). The single-line nature
constrains how line-break characters can be entered and displayed:

- **Literal Enter / Return key is intercepted, not inserted.** Pressing
  Enter inside the field commits the current value (same as Tab-out)
  and does NOT insert a literal U+000A newline into the string. This
  matches every other single-line text field in the Settings UI.
- **Literal Tab key is also intercepted, not inserted.** Pressing Tab
  moves focus to the next control. To include a tab character in the
  delimiter set, the user types the two-character escape sequence
  `\t`. The same applies to other line-break characters: a literal
  CR (U+000D), VT (U+000B), or FF (U+000C) cannot be typed directly
  into the field; the user types `\r`, `\v`, or `\f` instead.
- **Escape sequences typed in the field are interpreted.** When the
  user types the literal two-character sequence `\t`, the stored
  value contains a U+0009 tab. The same applies to `\n`, `\r`,
  `\v`, `\f`, `\b`, `\\`, and the Unicode forms `\uXXXX` /
  `\UXXXXXXXX`. The character set of accepted escapes mirrors the
  TOML basic-string escape table in B6.2 (so a value the user types
  in the UI round-trips identically through the on-disk TOML).
- **Pasting a multi-line string into the field is collapsed.** If a
  user pastes a string that contains literal U+000A or U+000D bytes
  (e.g., copied from another editor), each such byte is REPLACED in
  the stored value with its escape representation (`\n` for U+000A,
  `\r` for U+000D) so the field content remains a single visual line.
  An inline tooltip explains the substitution: "Newlines in pasted
  text were converted to `\n` escapes — they're still part of your
  delimiter set."

Two display modes:

- **Literal mode (default).** Printable characters render as
  themselves. Whitespace and control characters are present in the
  value but invisible in the field; the live preview below the field
  (see Settings / API surface) is the canonical way to confirm they
  are present.
- **"Show escapes" toggle.** When enabled, whitespace and control
  characters in the rendered field are shown as their escape
  representations (`\t`, `\n`, `\r`, `\v`, `\f`, space rendered as a
  visible middle-dot or similar) so the user can see and edit them
  precisely. With the toggle ON, typing the literal characters of an
  escape sequence (`\` then `t`) and a real tab character render
  identically.

A "Reset to default" button restores the documented default string.

#### B6.4. Validation

- Allowed characters in the resolved set: any printable Unicode codepoint, plus space, tab, newline, carriage return, vertical tab, form feed.
- Disallowed: other ASCII control characters (e.g., NUL, BEL, ESC). If the user enters one, the Settings UI shows an inline error and refuses to save until corrected; the runtime, if it ever encounters such a value, treats the setting as invalid (see B6.5) and logs a warning.

#### B6.5. Missing / empty / whitespace-only / invalid fallback

**Single source of truth — fallback chain by key.** Resolution depends on
WHICH key is being resolved. The B4 chain is authoritative. The
"built-in default from B1" is ONLY used at the END of the chain — never
as a shortcut that skips the per-context fallthrough. Concretely:

- For `editor.word_delimiters` (the global key): the chain is
  `(user value) → B1 built-in default`. Missing or empty resolves
  directly to the B1 default. There is no intermediate step because
  `editor.word_delimiters` is itself the editor-level key.
- For `terminal.word_delimiters` and `agent.word_delimiters` (the
  per-context keys): the chain is
  `(user value) → editor.word_delimiters (resolved) → B1 built-in default`.
  Missing or empty falls THROUGH to the resolved
  `editor.word_delimiters` (which itself may be a user value or the B1
  default). It does NOT shortcut directly to B1.

The "missing" and "empty string" cases are **always** treated as
"unset" — they are indistinguishable to the resolver. There is no
warning and no error for the empty case, regardless of which key is
empty. Specifically:

- The runtime resolves an empty/absent value by walking the chain above
  (per-key) — no warning, no error.
- The Settings UI text field, when empty (because the user cleared it
  OR the key is absent), renders the value the next step of the chain
  resolves to as a visible hint inside the field. For
  `editor.word_delimiters` that hint is the B1 default; for
  `terminal.word_delimiters` / `agent.word_delimiters` that hint is the
  current resolved value of `editor.word_delimiters`. A
  "Reset to default" indicator is present. Subtitle reads
  "Empty = inherits from editor default" (per-context) or
  "Empty = default delimiters" (editor).
- There is **no inline error** for the empty case. Earlier drafts that
  suggested showing an error when the field is empty are superseded by
  this section.

The only invalid cases that DO show inline errors are (a) whitespace-only values and (b) values containing disallowed control characters.

Authoritative table — note that the "Runtime fallback" column is
key-aware:

| Stored value                              | Editor key (`editor.word_delimiters`)                                | Per-context key (`terminal.word_delimiters`, `agent.word_delimiters`)                                                                                                                                | Settings UI                              |
| ----------------------------------------- | -------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------- |
| Key absent / unset                        | "Use default" — same as empty; B1 built-in default; no warning       | "Inherit from editor" — falls through to resolved `editor.word_delimiters`; no warning                                                                                                               | Field is empty; chain target rendered as visible hint; "Reset to default" indicator; subtitle "Empty = default delimiters" (editor) or "Empty = inherits from editor default" (per-context). No error. |
| `""` (empty string)                       | Same as missing                                                      | Same as missing                                                                                                                                                                                       | Same as above. No error.                 |
| Whitespace-only (e.g., `" "`, `"\t\n"`)   | **Invalid** — B1 built-in default; warning logged                    | **Invalid** — falls through to resolved `editor.word_delimiters` (NOT directly to B1 default); warning logged                                                                                         | Inline error: "Add at least one non-whitespace delimiter, or clear the field to use the default." |
| Some non-whitespace + any standard whitespace | Valid; effective `D` = user set ∪ whitespace floor                 | Valid; same as editor                                                                                                                                                                                 | Accepted                                  |
| Contains disallowed control char          | **Invalid** — B1 built-in default; warning logged                    | **Invalid** — falls through to resolved `editor.word_delimiters` (NOT directly to B1 default); warning logged                                                                                         | Inline error; save disabled              |

In all "valid" cases, B5's whitespace floor still applies — whitespace IS forced into the effective set even if the user did not include it.

This subsumes any earlier per-context fallback wording. The single
authoritative rule is: "for any per-context key that is missing /
empty / whitespace-only / contains a disallowed control char, the
fallback target is the RESOLVED `editor.word_delimiters` value, not
the B1 built-in default." Tests T9d and T9g pin this contract.

### B7. Delimiter run collapse

Consecutive delimiter characters are treated as a single boundary run (a single delimiter run per B2). Example with the default set on input `"foo//bar|"` (cursor at end):

- Press 1: char left = `r` (∉ D). Rule 3 → consume `bar`. Buffer: `"foo//|"`.
- Press 2: char left = `/` (∈ D). Rule 2 → consume the delimiter run `//`, then the non-delimiter run `foo`. Buffer: `"|"`.

The `//` run is consumed atomically as one delimiter run; it never produces an intermediate stop. This matches the canonical algorithm in B2 and aligns with VS Code, Sublime, and JetBrains conventions.

## Settings / API surface

| Key                        | Type     | Default                      | Notes                                                  |
| -------------------------- | -------- | ---------------------------- | ------------------------------------------------------ |
| `editor.word_delimiters`   | `string` | `/`, `.`, `-`, `_`, `:`, `=`, space, `\t`, `\n`, `\r`, U+000B, U+000C — see B1 for the valid TOML encoding (`\xXX` is NOT a valid TOML escape) | Global default. See B6 for value-string semantics, escape sequences, validation, and missing/empty/whitespace-only/invalid handling. Whitespace floor (B5) always applies. |
| `terminal.word_delimiters` | `string` | unset (falls back to editor) | Override for terminal input. Per B4 + B6.5, missing/empty/invalid falls through to `editor.word_delimiters` (NOT directly to the built-in default). |
| `agent.word_delimiters`    | `string` | unset (falls back to editor) | Override for agent prompt input. Same fallthrough as terminal. |

UI placement: **Settings → Editor → "Word Boundary Characters"**. Single-line text input with a "Reset to default" button and a "Show escapes" toggle (per B6.3). Validation per B6.4 — disallowed control characters trigger an inline error and disable save. Whitespace-only values trigger an inline error per B6.5. **Empty values do NOT trigger an error** — they are treated as "use default" and the field renders the default as a hint with a "Reset to default" indicator and the subtitle "Empty = default delimiters" (per B6.5). Below the input, a live preview line:

```
/var/www/example.com    ←| ←| ←| ←|
```

Arrows render at each position Delete Word Left would land using the current setting value, giving immediate feedback. The preview reflects the effective `D` (user value ∪ whitespace floor) so the user can see the real boundaries.

Per-context overrides are surfaced under their respective Terminal and Agent settings sections, each with the same input + preview + Show-escapes pattern and a "Use editor default" reset button.

No new keybindings, command palette actions, or context flags are added.

## Acceptance Criteria

All criteria below match the canonical algorithm in B2.

- **A1.** Delete Word Left on `/var/www/example.com|` (cursor at end) with the default `D` produces, on successive presses, this canonical sequence of 6 buffer states (start state plus 5 deletions):
  - State 0 (start): `/var/www/example.com|`
  - After press 1: `/var/www/example.|` (Rule 3 — char left=`m` ∉ D — consume non-delim run `com`)
  - After press 2: `/var/www/|` (Rule 2 — char left=`.` ∈ D — consume delim `.` then non-delim `example`)
  - After press 3: `/var/|` (Rule 2 — consume delim `/` then non-delim `www`)
  - After press 4: `/|` (Rule 2 — consume delim `/` then non-delim `var`)
  - After press 5: `|` (Rule 2 — consume delim `/`; no preceding non-delim run)
- **A2.** Delete Word Right mirrors A1 by the canonical algorithm:
  - **Mid-non-delim case (cursor inside a non-delim run):** consume from the cursor to the END of the run, do NOT cross into the following delimiter run on this press.
  - **Otherwise (cursor is at a boundary, i.e. char immediately right is in `D`, or the cursor is at start/end of a delimiter region):** consume the next delimiter run AND the next non-delimiter run.

  Worked example for `|/var/www/example.com` (cursor at start of line, default `D`). Char immediately right = `/` (∈ D), so each press is the "otherwise" branch (consume delim run + non-delim run). Canonical 5-state sequence (start + 4 deletions, 4 presses to fully clear):
  - State 0 (start): `|/var/www/example.com`
  - After press 1: `|/www/example.com` (consume delim `/` then non-delim `var`)
  - After press 2: `|/example.com` (consume delim `/` then non-delim `www`)
  - After press 3: `|.com` (consume delim `/` then non-delim `example`)
  - After press 4: `|` (consume delim `.` then non-delim `com`)

  Worked example for `var/www|.com` (cursor mid-line, between `www` and `.`). Char immediately right = `.` (∈ D); cursor is at a boundary (not inside a non-delim run). One DWR press consumes delim `.` then non-delim `com`. Result: `var/www|`.

  Worked example for `var/w|ww.com` (cursor mid-non-delim-run inside `www`). Char immediately right = `w` (∉ D); cursor IS inside a non-delim run. The "mid-non-delim" branch applies: consume from cursor to end of the run (`ww`). Result: `var/w|.com`. The following delimiter run `.` is NOT consumed on this press.
- **A3.** With default set, on `/path/to/file|` the first Delete Word Left removes only `file`, leaving `/path/to/|`. (Rule 3: char left=`e` ∉ D, consume just the non-delimiter run `file`.)
- **A4.** Per-context override applies only in that input context; other contexts use the editor default or their own override (per B4).
- **A5.** Missing/empty setting value falls back to the built-in default from B1 (per B6.5).
- **A6.** Whitespace-only setting value is rejected as invalid; the runtime falls back per the B6.5 fallback chain (which is per-context — NOT a flat fall-back to the built-in default in every case) and logs a warning; the Settings UI shows an inline error (per B6.5).
  - **A6.a.** For the global `editor.word_delimiters`: a whitespace-only value falls back to the built-in default from B1.
  - **A6.b.** For per-context overrides `terminal.word_delimiters` and `agent.word_delimiters`: a whitespace-only value falls THROUGH to the resolved `editor.word_delimiters` (which itself may be the user value or the B1 default per the B4 chain), NOT directly to the B1 built-in default. This matches the B6.5 per-context override fallthrough rule and is verified by T9g.
- **A7.** Whitespace remains a delimiter even when the user provides a value with no whitespace; the whitespace floor is always present in the effective `D` (per B5).
- **A8.** Delimiter run collapse: consecutive delimiters form a single boundary run consumed atomically by Rule 2 (per B7).
- **A9.** Move Cursor Word Left / Right use the same `D` and the same run rules as Delete Word Left / Right (per B3).
- **A10.** Setting changes take effect for the next keystroke without app restart.
- **A11.** Setting persists across app restarts.

## Implementation Pointers

- **Boundary module.** Add or extend `app/src/editor/word_boundary.rs` exposing a `WordBoundaryClassifier` that takes the resolved delimiter string and provides `is_delimiter(char) -> bool` plus helper iterators for left/right boundary search.
- **Resolution helper.** Add `resolve_word_delimiters(context: InputContext) -> String` in the same module, implementing the B4 fallback chain. `InputContext` enum: `Terminal`, `Agent`, `Editor`.
- **Key handler dispatch.** Update `app/src/editor/key_handler.rs` so the four shortcut handlers (Delete Word Left, Delete Word Right, Move Cursor Word Left, Move Cursor Word Right) call into `WordBoundaryClassifier` instead of the existing whitespace-only check.
- **Whitespace floor.** Inside `WordBoundaryClassifier`, OR the user's set with the fixed whitespace set so B5 holds without callsite branching.
- **Settings schema.** Add the three keys to `app/src/settings/editor.rs` (and matching terminal/agent settings modules). **All three are typed as `Option<String>` with NO literal default in the schema** — this is required to keep "absent" and "empty string" both meaning "use B1 default" per B6.5, and to keep the Settings UI hint/"Reset to default" indicator state consistent. Specifically:
  - `editor.word_delimiters: Option<String>` — schema default is `None`; the B1 character set is applied at RESOLUTION TIME by `resolve_word_delimiters(InputContext::Editor)`, NOT at schema-load time.
  - `terminal.word_delimiters: Option<String>`, `agent.word_delimiters: Option<String>` — same `Option<String>` shape, same `None` default; resolved per the B4 chain.
  - **Why NOT a literal schema default for `editor.word_delimiters`.** A literal schema default would (a) make the on-disk TOML look populated even when the user never set it, (b) cause the Settings UI to render the value in the field rather than as a hint with the "Reset to default" indicator (per B6.5), and (c) break the B6.5 contract that "Key absent" and "empty string" are indistinguishable at runtime. Resolving the default at access time is the only shape that satisfies all three.
  - The B1 default character set lives as a `const DEFAULT_WORD_DELIMITERS: &str = ...` next to `WordBoundaryClassifier` (NOT in the settings schema), and is consumed only by `resolve_word_delimiters` when the resolved option is `None` or empty.
- **Settings UI.** Add the input row plus live-preview component under `app/src/settings_view/editor_page.rs`. Reuse the existing text-input component; the preview is a small custom widget rendering arrow markers at each computed boundary.
- **No persistence migration** — these are additive optional keys.

## Tests

All tests assume the canonical algorithm in B2.

### Delete Word Left

- **T1.** Default behavior on `/var/www/example.com|`: assert all 6 buffer states in the canonical sequence (state 0 is the start; each subsequent state is the result of one DWL press). The test MUST walk every intermediate state — none may be elided:
  - State 0 (start): `/var/www/example.com|`
  - State 1 (after press 1): `/var/www/example.|`
  - State 2 (after press 2): `/var/www/|`
  - State 3 (after press 3): `/var/|`
  - State 4 (after press 4): `/|`
  - State 5 (after press 5): `|`

  This matches the A1 acceptance criterion and the B2 worked example. Press 6 is asserted as a no-op (cursor at start-of-line, B2 Rule 1).
- **T2.** Custom delimiter set `D = {":"}` (whitespace floor still applies) on `key:value:other|`: presses produce `key:value:|`, `key:|`, `|`.
- **T3.** Run collapse on `foo//bar|` with default set: press 1 → `foo//|`; press 2 → `|`. (Two presses total — `//` is a single delimiter run consumed atomically with the preceding non-delimiter run by Rule 2.)
- **T4.** Cursor mid-non-delimiter run on `var|/www` with default set: Delete Word Left consumes only `var` from cursor leftward (Rule 3), result `|/www`.

### Delete Word Right

- **T8a.** Default set, `|/var/www/example.com`: assert all 5 buffer states in the canonical sequence. The test MUST walk every intermediate state:
  - State 0 (start): `|/var/www/example.com`
  - State 1 (after press 1): `|/www/example.com` (consume delim `/` then non-delim `var`)
  - State 2 (after press 2): `|/example.com` (consume delim `/` then non-delim `www`)
  - State 3 (after press 3): `|.com` (consume delim `/` then non-delim `example`)
  - State 4 (after press 4): `|` (consume delim `.` then non-delim `com`)

  Matches A2.
- **T8b.** Custom delimiter set `D = {"/"}` (whitespace floor still applies) on `|/foo/bar/baz`: presses produce `|/bar/baz`, `|/baz`, `|`.
- **T8c.** Delete Word Right at end-of-line on `foo|` is a no-op.
- **T8d.** Cursor mid-non-delimiter run on `foo|bar/baz` with default set: char right=`b` ∉ D and cursor is INSIDE the non-delim run `foobar`. The "mid-non-delim" branch applies: consume from cursor to end of run (`bar`). Result: `foo|/baz`. The following delim run `/` is NOT consumed.
- **T8e.** Cursor at boundary mid-line on `var/www|.com` with default set: char right=`.` ∈ D; cursor is at a boundary (not inside a non-delim run). One DWR press consumes delim `.` then non-delim `com`. Result: `var/www|`. Matches the second worked example in A2.

### Move Cursor Word Left / Right

- **T7.** Move Cursor Word Left / Right land at the same boundaries Delete Word Left / Right would operate on, for identical input and setting (per A9 / B3). Verify on the A1 and A2 examples.
- **T7a.** **Authoritative Move Cursor Word Right parity test.** On
  `foo|/bar/baz` with default set, the cursor walks per the B3 parity
  rule: each press lands the cursor where the equivalent Delete Word
  Right would have left it, i.e. at the END of the consumed region.
  Char immediately right of the cursor at press 1 is `/` (∈ D), so the
  "otherwise" branch in B2 applies: the press consumes the entire
  delimiter run AND the following non-delimiter run together. Concretely:
  - State 0 (start): `foo|/bar/baz`
  - After press 1: `foo/bar|/baz` (DWR would consume `/` then `bar`,
    leaving cursor at the end of `bar` — i.e. between `bar` and the
    next `/`)
  - After press 2: `foo/bar/baz|` (DWR would consume `/` then `baz`,
    leaving cursor at end-of-line)
  - After press 3: `foo/bar/baz|` (no-op — at end-of-line)

  Earlier drafts that asserted MWR lands BETWEEN the delimiter run and
  the non-delimiter run on each press (e.g. `foo/|bar/baz` after press
  1) are superseded by this test. Those positions correspond to a
  different algorithm (consume delim run only, OR stop at every
  delimiter boundary) that is NOT the canonical algorithm in B2 and
  would diverge from Delete Word Right. T7a as written above matches
  B3's parity rule and the canonical B2 algorithm.
- **T7b.** Mid-non-delim parity for Move Cursor Word Right: on
  `foo|bar/baz` with default set, char right = `b` (∉ D) and the
  cursor is inside the non-delim run `foobar`. DWR's "mid-non-delim"
  branch applies: consume from cursor to end of run (`bar`). MWR
  parity → cursor lands at `foobar|/baz`. The following delim run `/`
  is NOT crossed on this press. (Mirrors T8d for DWR.)

### Settings resolution

- **T9a.** Context override precedence: set `editor.word_delimiters = "/"` and `terminal.word_delimiters = "."`. Assert terminal uses `.` ∪ whitespace; agent uses `/` ∪ whitespace (falls back to editor); editor uses `/` ∪ whitespace.
- **T9b.** Missing key fallback: leave `editor.word_delimiters` unset; assert the B1 literal default applies (set `{ '/', '.', '-', '_', ':', '=' }` ∪ whitespace floor including U+000B and U+000C). Per B6.5, no warning, no error.
- **T9c.** Empty string fallback: set `editor.word_delimiters = ""`; assert default applies; **no warning logged, no UI inline error shown**. Settings UI renders the default as a hint with the "Reset to default" indicator and subtitle "Empty = default delimiters" (per B6.5). Behavior is identical to T9b.
- **T9d.** Whitespace-only invalid: set `editor.word_delimiters = "  \t"`; assert default applies; warning logged; Settings UI shows inline error (per B6.5).
- **T9e.** Disallowed control char invalid: set `editor.word_delimiters = "/"` (BEL, encoded with valid TOML `\u00XX`); assert default applies; warning logged; Settings UI shows inline error.
- **T9f.** Whitespace floor enforced: set `editor.word_delimiters = "/"` (no whitespace). Assert Delete Word Left still stops at spaces and newlines (per A7 / B5).
- **T9g.** Per-context override invalid → falls through to editor (not directly to default): set `editor.word_delimiters = "/"`, `terminal.word_delimiters = "  "`. Assert terminal uses editor's `/`, NOT the built-in default.

### TOML escape sequences

- **T10a.** TOML basic-string escapes: `editor.word_delimiters = "/.\t\n"` parses to set `{ '/', '.', '\t', '\n' }` ∪ whitespace floor.
- **T10b.** Literal whitespace and escape are equivalent: a literal space character and `" "` typed via the field both add space to the set (already in floor; verify no duplication issue).

### Runtime behaviors

- **T11.** Live effect: change `editor.word_delimiters` at runtime; assert next Delete Word Left keystroke uses the new set without restart.
- **T12.** Persistence: set custom value, restart app, assert value preserved and applied.
- **T13.** Multi-byte safety: with a delimiter set containing only ASCII, UTF-8 multi-byte characters (e.g., emoji, CJK) are treated as non-delimiters and not split mid-character.

## Open Questions

- **Regex / predicate boundaries (V1.5).** Should advanced users be able to define word boundaries via a regex or pattern instead of a literal character list? Recommendation: **yes, deferred to V1.5** behind a separate setting key (`editor.word_delimiter_pattern`) that, when set, takes precedence over the literal list. Out of scope for V1.
- **Unicode-aware default.** Should the default include common Unicode punctuation (em-dash, curly quotes)? Recommendation: defer pending user feedback; the literal-character model handles this once we have a concrete request.
- **Per-language overrides.** Should code-aware contexts (e.g., agent reasoning over Rust source) have language-tuned defaults? Out of scope; revisit if signal emerges.

## Telemetry

No new telemetry events. Standard `setting.changed` coverage on the three new keys is sufficient to gauge adoption and detect misconfiguration patterns.
