# Spec: Improve default proportional font readability (GH-9953)

## Problem

The default proportional font used in Settings panes and rendered
Markdown looks cramped and is hard to read compared to GitHub's
rendering of the same content. The issue includes a side-by-side
screenshot.

## Goal

Two-part fix: (1) ship a more readable default proportional font
choice, (2) expose the proportional font as a user-configurable
setting (it currently isn't).

## Surface scope

V1 changes apply to a defined set of surfaces. The new tokens
read from the same Appearance state that the rest of the app uses,
but only the listed surfaces consume the new tokens; all other UI
surfaces continue using existing typography unchanged in V1.

V1 surfaces:

- **Settings panes** — every page under Settings reads the new
  proportional font tokens.
- **Rendered Markdown blocks** — Markdown rendered in agent
  output and other in-product Markdown surfaces (changelog,
  product help text rendered as Markdown).

Out-of-scope surfaces for V1 (continue using existing typography):

- Block headers, command bar, prompt, and other shell chrome.
- Tooltips, toast notifications, popover chrome.
- Login / onboarding surfaces.

These surfaces may opt in in a follow-up; V1 keeps them on
existing typography to minimize visual churn.

## Default font stacks (per platform, no Inter)

The platform Default stack is what Warp uses when
`proportional_font_family` is `None`. Warp does **not** bundle the
Inter typeface, so Inter is **not** in any platform Default stack.
Users who have Inter installed locally and want Inter as their
primary proportional font MUST opt in by setting
`proportional_font_family` explicitly (e.g.,
`"Inter, system-ui, sans-serif"`). The Default behavior is
deterministic per-platform:

| Platform   | Default stack                                                                |
| ---------- | ---------------------------------------------------------------------------- |
| macOS      | `system-ui, -apple-system, "SF Pro Text", "Helvetica Neue", sans-serif`      |
| Windows    | `system-ui, "Segoe UI Variable", "Segoe UI", sans-serif`                     |
| Linux      | `system-ui, "Cantarell", "Ubuntu", "Liberation Sans", sans-serif`            |

Note: Inter is NOT in the default fallback. Users who have Inter
installed and want it as primary must explicitly configure
`proportional_font_family` to include it.

### Generic-family entries in the cascade

The Default stacks above contain CSS generic-family keywords
(`system-ui`, `sans-serif`) that are **not installed fonts** but
are well-defined identifiers in CSS Fonts Module Level 4. These
are **not subject to the "is the font installed?" check** in
B1b's left-to-right cascade; they always resolve, by browser
contract, to a real font picked by the platform's font-engine
fallback chain.

Concretely, the resolver classifies each family-list entry into
one of three classes before B1b runs:

1. **Generic-family keyword (always resolves).** The CSS generic
   identifiers `system-ui`, `sans-serif`, `serif`, `monospace`,
   `cursive`, `fantasy`, `ui-sans-serif`, `ui-serif`,
   `ui-monospace`, `ui-rounded`, `math`, `fangsong`, `emoji`.
   These are emitted to the CSS as-is (unquoted, lowercase) and
   the underlying font engine (Skia / Chromium / WebKit / DWrite,
   depending on the platform) maps them to a real font. The
   resolver does NOT short-circuit on a generic-family match; it
   simply emits the keyword and lets the font engine resolve.

2. **Vendor-prefixed identifier (always resolves on the matching
   platform).** Identifiers like `-apple-system`,
   `-webkit-system-font`. These are treated identically to a
   generic-family keyword on the platform that recognizes them
   (the font engine resolves them) and behave as a "missing font"
   on other platforms (the cascade falls through). This matches
   Chromium / WebKit behavior.

3. **Named family (subject to installed-font check).** Everything
   else — quoted-string entries (`"Helvetica Neue"`,
   `"Segoe UI"`, `"SF Pro Text"`) and unquoted custom identifiers
   (`Cantarell`, `Ubuntu`, `Inter`). The installed-font check in
   B1b applies ONLY to this class.

This means cross-platform resolution of the macOS Default stack
on a Linux device is well-defined:

- `system-ui` (class 1) → resolved by the Linux font engine to a
  platform default sans (typically Cantarell on GNOME, Noto Sans
  elsewhere). The cascade does NOT fall through this entry on the
  grounds that "system-ui isn't installed."
- `-apple-system` (class 2) → not recognized on Linux, treated as
  missing, cascade continues.
- `"SF Pro Text"` (class 3) → not installed on Linux, cascade
  continues.
- `"Helvetica Neue"` (class 3) → not installed on most Linux
  distros, cascade continues.
- `sans-serif` (class 1) → resolved by the Linux font engine.
  Cascade ends here regardless.

The class-1 / class-2 / class-3 distinction is part of the public
contract and is asserted by the new test
`T_generic_families_resolve` (added in Test plan). Without this
clarification, an implementer could reasonably interpret B1b as
"every entry is checked against installed fonts, and `system-ui`
is therefore always missing," which would force the cascade to
fall through to the appended Default stack on every render.

## Behavior contract

- B1. New setting `appearance.proportional_font_family:
  Option<String>`. `None` / omitted means "Default" and resolves to
  the per-platform Default stack listed above. The stored value is
  the **canonical serialization** (B1a) of the user-supplied
  family list.

  All three settings (family, size, line-height) use
  `SyncToCloud::Globally(RespectUserSyncSetting::Yes)` like other
  appearance settings.

- B1a. **Validation, parsing, and canonical serialization.** A
  non-empty user-supplied value is parsed as a CSS font-family
  list per CSS Fonts Module Level 4 grammar:

  - Families are separated by commas.
  - Each family is either an unquoted identifier sequence (one or
    more `<custom-ident>` tokens joined by ASCII space), or a
    double-quoted `<string>`.
  - Inside a quoted string, `"` and `\` MUST be backslash-escaped
    (`\"`, `\\`).

  After successful parse, Warp stores a **canonical serialization**
  of the family list (used for storage, sync, and Settings UI
  display):

  1. Each parsed family token is normalized in this order:
     1. **Internal whitespace within an unquoted identifier
        sequence is collapsed.** Multiple ASCII spaces between the
        custom-ident tokens of a single family become a single
        ASCII space. Tabs and other whitespace inside an unquoted
        identifier sequence are rejected at parse time per CSS
        Fonts L4 grammar.
     2. **Case is preserved verbatim from user input for class-3
        named families.** No case-folding is applied to class-3
        family names at any layer — storage, sync, settings UI,
        render. CSS family names are case-insensitive at *match*
        time (handled by the font engine, not by us), but the
        *stored* value preserves the user's chosen case so a peer
        device sees the same string the user typed. This rule
        applies to both unquoted and quoted class-3 families:
          - Unquoted `inter`, `Inter`, `INTER` are stored
            byte-for-byte as the user wrote them.
          - Quoted `"helvetica neue"`, `"Helvetica Neue"`, and
            `"HELVETICA NEUE"` are stored byte-for-byte as the user
            wrote them.
     3. **Class-1 and class-2 identifiers are normalized to
        ASCII-lowercase.** The CSS generic identifiers (class 1)
        and vendor-prefixed identifiers (class 2) are
        recognized case-insensitively at parse time (per CSS
        Syntax Module Level 3, `<custom-ident>` matching is
        ASCII-case-insensitive against the closed keyword set)
        and stored in their canonical ASCII-lowercase form. This
        is the ONE exception to rule 1.ii and is necessary
        because both sets are closed enumerations with a defined
        lowercase canonical spelling; storing `Sans-Serif` or
        `-Apple-System` would be a category-confusion bug.

        **Disambiguation rule (round-N fix).** "ASCII-lowercase"
        means each ASCII code point in the range `A`..`Z`
        (`0x41`..`0x5A`) is mapped to the corresponding code
        point in `a`..`z` (`0x61`..`0x7A`). No other code points
        are altered. The match against the closed class-1 /
        class-2 keyword set uses **simple ASCII case-folding**
        (the Unicode `simpleLowercase(input) == keyword` test
        restricted to ASCII), not full Unicode case-folding. This
        guarantees implementations and tests agree on stored
        values:

          | Raw input        | Class | Stored canonical | Reason                                               |
          | ---------------- | ----- | ---------------- | ---------------------------------------------------- |
          | `Sans-Serif`     | 1     | `sans-serif`     | ASCII-lowercased; matches `sans-serif` keyword       |
          | `SYSTEM-UI`      | 1     | `system-ui`      | ASCII-lowercased; matches `system-ui` keyword        |
          | `system-UI`      | 1     | `system-ui`      | ASCII-lowercased                                     |
          | `-APPLE-SYSTEM`  | 2     | `-apple-system`  | ASCII-lowercased; matches class-2 keyword            |
          | `-Apple-System`  | 2     | `-apple-system`  | ASCII-lowercased                                     |
          | `inter`          | 3     | `inter`          | NOT in class-1/class-2 set → class-3, preserved      |
          | `Inter`          | 3     | `Inter`          | NOT in class-1/class-2 set → class-3, preserved      |
          | `INTER`          | 3     | `INTER`          | NOT in class-1/class-2 set → class-3, preserved      |
          | `Sans-Serif-Pro` | 3     | `Sans-Serif-Pro` | NOT in class-1 closed set (extra suffix) → preserved |

        Class assignment is determined BEFORE case-normalization
        by the ASCII-case-insensitive match against the closed
        keyword set. Once a token is classified, the
        case-normalization rule for its class applies. There is
        no race / ordering ambiguity: classify first
        (ASCII-case-insensitive), then normalize (class-1 /
        class-2 → ASCII-lowercase; class-3 → preserve verbatim).
     4. **Quoting decision:**
        - If, after rule 1.i, the family token matches
          `<custom-ident>` (alphanumerics, underscore, hyphen, no
          leading digit, no internal whitespace, no special chars):
          emit unquoted (e.g., `Arial`, `Open-Sans`, `inter`).
        - Otherwise (contains spaces, commas, quotes, or any
          character outside `<custom-ident>`): emit double-quoted
          with backslash-escaping for `"` and `\` (e.g.,
          `"Helvetica Neue"`, `"My \"weird\" Font"`).
        - Class-1 and class-2 entries are emitted unquoted in
          their stored lowercase form (`system-ui`,
          `-apple-system`).
  2. Families are joined with `, ` (comma + ASCII space).
  3. **Round-trip identity:** parsing the canonical serialization
     and re-canonicalizing MUST yield the same string. Tested by
     T_canonical_serialization.

  Concretely, the case rule resolves the round-2 ambiguity:

  | User input                              | Canonical stored value                            |
  | --------------------------------------- | ------------------------------------------------- |
  | `inter , "Open Sans" ,Arial`            | `inter, "Open Sans", Arial`                       |
  | `Inter , "open sans" ,arial`            | `Inter, "open sans", arial`                       |
  | `Helvetica Neue, sans-serif`            | `"Helvetica Neue", sans-serif`                    |
  | `Helvetica  Neue,   Sans-Serif`         | `"Helvetica Neue", sans-serif`                    |
  | `system-UI, "SF Pro Text"`              | `system-ui, "SF Pro Text"`                        |
  | `SYSTEM-UI, "sf pro text"`              | `system-ui, "sf pro text"`                        |

  Therefore A7 in this spec is updated: writing
  `inter , "Open Sans" ,Arial` is stored as
  `inter, "Open Sans", Arial`. The previous wording
  ("`Inter, ...` (or the user's preferred case)") was the
  ambiguity; the contract is now: **case preserved verbatim except
  for generic and vendor-prefixed identifiers, which are
  lowercased.**

  Values that fail to parse are rejected on save with an inline
  error in Settings; the previously-valid (already-canonical)
  value is retained. Programmatic writes (e.g., from sync) that
  fail to parse are dropped, the family setting is cleared to
  `None` for this device, and a telemetry warning fires with the
  rule id (never the raw value).

- B1b. **Missing-font fallback (cross-platform contract).**
  Resolution proceeds left-to-right through the parsed family
  list. **Generic and vendor-prefixed identifiers are not subject
  to the "installed font" check** — they are emitted to the
  underlying font engine as-is and resolve via the platform's
  built-in CSS-generic mapping (see the class 1 / class 2 / class
  3 contract in "Generic-family entries in the cascade" above).
  The installed-font check applies ONLY to class-3 named family
  entries.

  Concrete cross-platform resolution rules (all platforms, all
  stacks, including the macOS Default stack on Linux and vice
  versa):

  1. For each entry in the user-supplied parsed family list,
     left-to-right, by class:
       - **Class 1 (generic).** Emit to the CSS as the lowercase
         keyword. The platform's font engine resolves the
         keyword to a real font. Resolution does NOT fall
         through this entry on any platform.
       - **Class 2 (vendor-prefixed).** Emit to the CSS as the
         lowercase keyword. The platform's font engine resolves
         it on the matching platform (e.g., `-apple-system` on
         macOS). On non-matching platforms the engine treats it
         as missing and the cascade continues to the next entry.
       - **Class 3 (named family).** Check installed-font set on
         the current OS. If installed, resolve and stop. If not
         installed, the cascade continues to the next entry.
  2. If the cascade exhausts the user-supplied list without
     terminating at a class-1 entry, **the platform Default stack
     is appended** and resolution restarts at the first entry of
     the appended Default stack (applying the same class rules).
     The appended Default stack always terminates at its own
     class-1 entry (every platform Default stack defined in
     "Default font stacks" above ends in `sans-serif`).
  3. If the parsed family list is empty after parsing, the
     platform Default stack is used directly.
  4. The stored value is never modified by fallback — it remains
     the user's preference in canonical form (B1a). Fallback is
     a render-time decision, not a storage-time decision.

  Cross-platform examples (all explicitly part of the contract;
  asserted by `T_generic_families_resolve` and the new
  `T_cross_platform_resolution`):

  | User-supplied list (after B1a)                | Linux render                                                                 | Windows render                                                          | macOS render                                                          |
  | --------------------------------------------- | ---------------------------------------------------------------------------- | ----------------------------------------------------------------------- | --------------------------------------------------------------------- |
  | `"SF Pro Text", -apple-system, sans-serif`    | class-3 miss → class-2 miss (vendor-prefixed, unmatched platform) → class-1 hit on `sans-serif` (terminates) | class-3 miss → class-2 miss → class-1 hit on `sans-serif`               | class-3 hit (terminates) |
  | `Inter, system-ui, sans-serif`                | class-3 hit if Inter installed; else class-1 hit on `system-ui` (terminates) | same                                                                    | same                                                                   |
  | `"Foo Bar"` (only one class-3 entry, missing) | cascade exhausts user list → appended Linux Default stack terminates at `sans-serif` | cascade exhausts user list → appended Windows Default stack             | cascade exhausts user list → appended macOS Default stack             |
  | `system-ui`                                   | class-1 hit (terminates)                                                     | class-1 hit (terminates)                                                | class-1 hit (terminates)                                              |

  No render path is left undefined: every user-supplied list,
  on every supported platform, either terminates at a class-1
  hit somewhere in the user list, or falls through to the
  appended platform Default stack which always terminates at
  its own class-1 entry. There is no "undefined" branch.

- B1c. **Per-platform sync schema.** All three settings sync as a
  per-platform map with a shared default key:

  ```json
  {
    "default": "<value or null>",
    "darwin":  "<value or null>",
    "win":     "<value or null>",
    "linux":   "<value or null>"
  }
  ```

  - On read, each platform consumes its own key. If that key is
    absent, it falls back to `default`. If `default` is also absent,
    the platform Default stack / clamp default applies.
  - On write, Warp writes the active platform's key. The `default`
    key is written by Settings only when the user explicitly
    chooses "Apply to all platforms"; otherwise it is left
    untouched.
  - Reset-to-Default clears the active platform's key only,
    reverting that platform to either `default` or the platform
    Default stack / clamp default.
  - Family values written across platforms are always canonical
    form per B1a. A peer device receiving a value re-parses it on
    read; if parse fails, B1a programmatic-write rules apply
    (drop value, fire telemetry).

- B1d. **Synced unsupported family.** A synced non-empty family
  that is unavailable on the current OS renders with the platform
  Default stack via B1b cascade. Settings shows the family as
  unavailable with a "Reset to Default" action. Reset clears the
  stored family so the setting returns to `None` for the active
  platform.

- B1e. **Render-time CSS construction (no string interpolation,
  injection-safe at the actual insertion point).**

  **The threat model has two layers, and BOTH MUST be addressed.
  Quoting the CSS string alone is explicitly INSUFFICIENT** —
  this section's contract requires the renderer to use a safe
  insertion path (CSSOM/style-property assignment) or, only as a
  documented fallback, HTML-end-tag-escape the emitted CSS. The
  round-N review correctly identified that the prior wording
  ("quoted CSS strings prevent breakouts") was wrong in the
  general case: HTML tokenizers do not understand CSS quoting.

  **Net contract** (binding for V1 implementers):

  1. **Quoting is necessary but NOT sufficient** to prevent
     injection at the HTML layer.
  2. The renderer MUST use **CSSOM / style-property assignment**
     (path (a) below) at every V1 agent-bound insertion site,
     including Settings panes and rendered Markdown. This is the
     V1 default.
  3. The renderer MUST NOT pass user-derived family strings
     through `innerHTML`, `outerHTML`, server-side string
     interpolation into a `<style>` tag, or any other
     HTML-tokenizer-bound path WITHOUT the HTML-end-tag escape
     pass in path (b).
  4. The acceptance tests below exercise the **actual render
     insertion path** end-to-end — not the serializer in
     isolation — so a future regression that bypasses CSSOM and
     reaches a `<style>` text node without escaping will fail CI.

  **Layer 1 — quoted CSS string serialization (necessary).** The
  renderer takes the **parsed token list** (the structured
  representation produced in B1a, not the stored canonical
  string) and constructs the CSS `font-family` declaration by
  feeding each token through the same canonical serializer.
  Equivalently: the path from user input to CSS is always
  `String -> ParsedFamilyList -> CssString`, never
  `String -> CssString`. CSS-level injection (a stray `;` or `}`
  breaking out of the property declaration) is impossible because
  named families are emitted as quoted CSS strings with `\` and
  `"` escaped; class-1 / class-2 keywords are members of a closed
  whitelist.

  **Layer 2 — insertion-context safety (REQUIRED).** Quoted CSS
  strings do NOT prevent an HTML `</style>` breakout because the
  HTML tokenizer parses `<style>` text content with a single
  rule: it ends the element on the next ASCII-case-insensitive
  match of `</style`. If the rendered CSS string is ever inserted
  into a `<style>` text node — which is how most browsers ship
  rendered CSS — a family value containing `</style><script>`
  would terminate the style element regardless of CSS quoting,
  because the HTML parser does not interpret CSS quoting.

  Therefore the renderer MUST use **one** of the two safe
  insertion paths. Implementations choose path (a) where possible
  and fall back to path (b) only when (a) is not available:

  (a) **CSSOM / style-property assignment (preferred).** Construct
      the rule via the CSSOM API, e.g.,
      `CSSStyleDeclaration::setProperty("font-family", canonical_css)`
      or by setting `element.style.fontFamily = canonical_css`.
      The browser parses the value through the CSS parser, never
      through the HTML tokenizer, so `</style>` is inert. This is
      the V1 default for all V1-scope surfaces (Settings panes,
      rendered Markdown).

  (b) **`<style>`-text-node insertion with HTML-end-tag escaping
      (fallback).** If the CSS must be embedded as a `<style>`
      text node (e.g., a server-rendered initial paint), the
      serializer emits the canonical CSS string with all
      occurrences of `</` *inside the quoted family name* escaped
      using the CSS hex-escape form `\3c\2f` (or the equivalent
      `\<\/` with whitespace-terminated CSS escapes). This is in
      addition to the Layer-1 escaping. The HTML tokenizer's
      lookahead for `</style` therefore never matches inside the
      quoted CSS string. Path (b) MUST NOT be used without this
      additional escape pass.

  **Implementation requirements:**

  1. The renderer MUST NOT pass user input through string
     interpolation directly into a `<style>` element's
     `innerHTML` / `textContent` setter.
  2. The CSS serializer exposes two entry points:
     `to_css_for_style_property()` (Layer 1 only — for path (a))
     and `to_css_for_inline_style_tag()` (Layer 1 + Layer 2 — for
     path (b)). The type system enforces this: each call site
     declares which path it uses.
  3. The default V1 implementation uses path (a) at every V1-scope
     insertion site (Settings panes, rendered Markdown). A
     comment in the code documents that any future caller using
     path (b) MUST call `to_css_for_inline_style_tag()`.

  **Tests cover the actual render insertion path, not just the
  serializer in isolation:**
  - `T_render_no_injection_property_path` (renamed from
    `T_render_no_injection`) — drives a real DOM render, sets
    `font-family` to `"</style><script>alert(1)</script>"`, and
    asserts (i) the script does NOT execute, (ii) the
    `<style>`/CSSOM rule is well-formed, (iii) the family value
    in the computed style is the canonical quoted string.
  - `T_render_no_injection_style_tag_path` — drives a render path
    that goes through `to_css_for_inline_style_tag()`, asserts
    the emitted text contains `\3c\2f` (or equivalent) instead of
    a raw `</`, and asserts the HTML parser tokenizes the
    `<style>` element as a single text run.

  Implementation MUST use the CSSOM/style-property path (a) for
  V1; the test plan exercises both paths so future callers cannot
  silently regress.

- B2. New setting
  `appearance.proportional_font_size: f32`
  (default `14.0`, clamped to `[14.0, 24.0]`). Persistence and
  sync follow B1c (per-platform map with shared default).
  Manually edited or synced values outside the range are clamped
  on read to the nearest bound; the stored value is left unchanged
  to preserve user intent if the bounds expand later.

- B3. New setting
  `appearance.proportional_font_line_height: f32`
  (default `1.55`, clamped to `[1.0, 2.5]`). Persistence and sync
  follow B1c (per-platform map with shared default). Same clamp
  behavior as B2.

- B4. The default values are chosen to match GitHub's typography
  baseline (1.5–1.6 line-height, 14–16px). Verified by side-by-side
  screenshot in the test plan.

- B5. Settings → Appearance gets a "Proportional text" subsection
  with these three controls and a live preview block. Family input
  shows canonical serialization; the inline error on parse failure
  references the offending token.

- B6. Existing fixed-pixel layouts that depend on the current
  cramped baseline are audited and migrated to em-relative or
  line-height-relative spacing where they would otherwise break
  on larger fonts.

## Acceptance criteria

- A1. Default install on macOS, Windows, and Linux uses the
  platform Default stack listed in "Default font stacks" (NO Inter
  in the stack on any platform) and renders Markdown side-by-side
  with GitHub visually comparable (same paragraph fits in same
  vertical space ±10%).
- A2. User can change the family in Settings → Appearance and the
  change applies to V1 surfaces (Settings panes and rendered
  Markdown) without restart. Out-of-scope surfaces (Surface
  scope) are unchanged.
- A3. Setting size to the configured maximum of `24` does not
  break Settings layout (tabs don't overflow, buttons don't clip)
  and is reflected in the live preview and rendered Markdown.
- A3a. Setting size to the configured minimum of `14` renders
  cleanly with no clipping at the smallest supported breakpoint.
- A3b. Programmatic / synced size of `13` clamps to `14` on read;
  size of `25` clamps to `24` on read. Stored value is left
  unchanged.
- A3c. Programmatic / synced line-height of `0.9` clamps to `1.0`
  on read; `2.7` clamps to `2.5` on read. Stored value is left
  unchanged.
- A4. An unavailable synced font family (e.g., a peer device wrote
  `"Foo Bar Custom"` that isn't installed on the local OS) falls
  back to the platform Default stack via B1b cascade and exposes
  Reset to Default in Settings.
- A5. Per-platform sync round-trip: a value written on macOS is
  read back on macOS only; Linux on the same account either reads
  its own platform key or falls back to `default` / platform
  Default stack per B1c.
- A6. Invalid family input (e.g., unbalanced quotes, illegal
  characters in an unquoted identifier) is rejected in Settings
  with an inline error; the previous canonical value is retained.
- A7. Canonical-form round-trip: writing
  `inter , "Open Sans" ,Arial` (with stray spaces) is stored as
  `inter, "Open Sans", Arial` (case preserved verbatim per
  B1a.1.ii — `inter` stays lowercase if the user typed lowercase).
  Reading back yields the same canonical string. Writing
  `SYSTEM-UI, "SF Pro Text"` is stored as
  `system-ui, "SF Pro Text"` (generic identifier lowercased,
  named family preserved).
- A8. CSS-injection guard at the actual render insertion path
  (B1e). A family value containing
  `</style><script>alert(1)</script>` is rendered into V1's
  CSSOM / style-property insertion path (B1e Layer 2 path (a)),
  and:
  - The injected `<script>` does NOT execute (DOM render
    asserts `window.__injected_alert__` is unset).
  - The computed style's `font-family` reads back as the
    canonical quoted CSS string with the malicious token escaped.
  - The DOM tree contains zero `<script>` elements introduced by
    the family value.

  If a future caller chooses the `<style>`-text-node fallback
  path (B1e Layer 2 path (b)), the same value MUST emit
  `\3c\2f` (or equivalent CSS hex-escape) inside the quoted
  family token so the HTML tokenizer cannot find a `</style`
  match inside the embedded CSS.

## Implementation pointers

- Existing proportional font is set in
  `app/src/appearance.rs` (search for "proportional" or
  "ui_font"). Replace the hard-coded value with a setting read.
- Markdown renderer reads the proportional font from the same
  source — verify the helper used by the editor's
  `BufferBlockStyle` paths.
- Settings sync layer: extend the existing per-setting sync codec
  to encode/decode the per-platform map in B1c. Existing scalar
  values from older clients deserialize into the `default` key.
- Family-list parser/serializer is a single module exposing
  `parse_family_list(s: &str) -> Result<FamilyList, ParseErr>` and
  `serialize_family_list(list: &FamilyList) -> String`. The
  renderer's CSS builder takes `&FamilyList`, not `&str`, to
  enforce B1e at the type level.

## Test plan

- T1–T3. Setting round-trips for family / size / line-height,
  including the per-platform sync schema (B1c) — write on darwin,
  read on darwin, read on linux without darwin key (should hit
  `default`), read on linux without any key (should hit Default
  stack / clamp default).
- T4. Snapshot test: rendered README.md fixture matches a stored
  reference within tolerance.
- T5. Settings page renders without overflow at default and at
  max font-size (24), and with no clipping at min (14).
- T6. Empty, whitespace-only, and unavailable synced font-family
  values render with the platform Default stack and expose Reset
  to Default.
- T7. Clamp boundary tests: size `13`→`14`, size `25`→`24`,
  line-height `0.9`→`1.0`, line-height `2.7`→`2.5`. Stored value
  unchanged after read.
- T8. Validation: invalid family input rejected with inline error;
  programmatic write of invalid family drops the value and emits
  a telemetry warning with rule id only.
- T9. Visual comparison harness: side-by-side rendering of a
  fixture Markdown document and a fixture Settings pane against a
  stored reference at sizes `14`, `18`, and `24`. Each comparison
  asserts pixel-difference within tolerance and asserts paragraph
  vertical-space parity with the GitHub baseline (±10%).
- T10. Synced unsupported family: peer-device sync sets
  `proportional_font_family` to a family not installed on the
  current OS — local device renders with platform Default stack
  and Settings exposes Reset to Default.
- T_canonical_serialization. Round-trip: a parsed family list,
  re-serialized through the canonical serializer, parses again to
  the same family list. Run on a fixture set including stray
  spaces, missing/extra commas-around-spaces, mixed quoting.
- T_canonical_quoting. Family names containing spaces are
  emitted double-quoted; names containing `"` and `\` are emitted
  with backslash-escaped quoted form; simple identifiers stay
  unquoted.
- T_inter_not_in_default. The platform Default stack on each of
  macOS, Windows, and Linux does NOT contain `Inter`. Tested by
  asserting the literal Default stack strings.
- T_render_no_injection_property_path. Setting
  `proportional_font_family` to a value containing
  `</style><script>window.__injected_alert__ = 1;</script>` and
  rendering through the V1 default path (CSSOM /
  style-property assignment, B1e path (a)) MUST:
  1. Not execute the injected script (DOM render asserts
     `window.__injected_alert__` is `undefined` and that the
     computed `<head>` and target element trees contain ZERO
     additional `<script>` nodes after the render).
  2. Round-trip the family through `getComputedStyle`'s
     `font-family` and produce the canonical quoted CSS string
     with `\` and `"` properly escaped per B1a.
  3. Pass on each V1 surface (Settings panes, rendered Markdown)
     — driven via the actual render entry point, not a
     synthetic harness.
- T_render_no_injection_style_tag_path. The
  `to_css_for_inline_style_tag()` serializer path (B1e path
  (b)), invoked on the same payload, emits a CSS string in
  which every `</` inside a quoted family token is replaced
  with `\3c\2f` (or the equivalent whitespace-terminated CSS
  escape). Asserts:
  1. The emitted bytes contain ZERO occurrences of the literal
     ASCII substring `</style` (case-insensitive) inside the
     quoted CSS string.
  2. Inserting the emitted bytes into a `<style>` text node and
     parsing the document yields a `<style>` element whose text
     content is the entire CSS rule (not truncated at the
     attempted `</style>` breakout).
  3. No script execution, no `<script>` element introduced.
- T_generic_families_resolve. The B1b cascade does NOT treat
  CSS generic-family keywords (class 1: `system-ui`,
  `sans-serif`, `serif`, `monospace`, `cursive`, `fantasy`,
  `ui-sans-serif`, `ui-serif`, `ui-monospace`, `ui-rounded`,
  `math`, `fangsong`, `emoji`) and vendor-prefixed identifiers
  (class 2: `-apple-system`, `-webkit-system-font`) as
  "missing fonts" subject to the installed-font check. For each
  platform Default stack defined in "Default font stacks":
  - Render with that stack and assert the cascade does NOT
    fall through past the first class-1 entry on the basis of
    "not installed."
  - Render with a deliberately fully-missing class-3 stack
    `"FontA, FontB"` (no class-1 fallback) and assert the
    appended platform Default stack is used.
  - Render with `"FontA, FontB, sans-serif"` (class-3
    misses + class-1 terminator) and assert the cascade
    terminates at the class-1 entry without appending the
    Default stack a second time.
- T_cross_platform_resolution. Drive the resolver with each row
  of the B1b cross-platform examples table on each of macOS,
  Windows, and Linux. For every (row, platform) pair, assert:
  1. The cascade walk visits the expected entries in order.
  2. The terminating class-1 entry (either in the user list or
     in the appended platform Default stack) resolves to a real
     font on the current OS.
  3. No "undefined" / no-font branch is ever taken.
  Includes a synthetic case where the user list has only
  class-3 misses with no class-1 terminator (`"Foo Bar"`), and
  asserts the appended platform Default stack is used and
  itself terminates at its own class-1 entry.
- T_canonical_case. Round-trip the user inputs in the case
  table under B1a and assert each stored value matches the
  canonical-form column byte-for-byte. Specifically:
  - `inter , "Open Sans" ,Arial` → `inter, "Open Sans", Arial`
    (no auto-capitalization).
  - `Inter , "open sans" ,arial` → `Inter, "open sans", arial`.
  - `Helvetica  Neue,   Sans-Serif` →
    `"Helvetica Neue", sans-serif` (internal whitespace
    collapsed in the unquoted form is moot here because spaces
    force quoting; the test uses the leading whitespace +
    Sans-Serif → sans-serif transformation).
  - `system-UI, "SF Pro Text"` → `system-ui, "SF Pro Text"`
    (generic identifier lowercased, named family preserved).
  Tests A7 with the disambiguated case-rule.

## Out of scope

- Bundling Inter or any custom font with the binary (use system
  defaults; user can install Inter manually if desired).
- Separate fonts for Settings vs Markdown — they share one knob
  in V1.
- Extending the new typography to the out-of-scope surfaces
  enumerated in Surface scope (block headers, command bar,
  tooltips, etc.) — targeted for a follow-up.
