# Spec: Tab configs specify agent profile (GH-10171)

## Problem

Tab configs support `type = "agent"` to open a pane in Agent Mode,
but there is no way to associate a specific agent profile (e.g.
"Coder devbox" with full autonomy on a remote dev machine) with
that pane. Users with multiple profiles must manually switch each
time the tab config is reused.

A tab config can also be authored, shared, or imported. Allowing
it to silently bind a high-autonomy profile and dispatch input on
open is a trust hazard. The spec must define a consent model so
that imported / shared configs cannot escalate autonomy without a
visible user gate.

## Security threat model and mitigations

This section maps the four standing concerns from oz-for-oss
review onto the binding clauses in the rest of the spec, so the
reviewer can verify coverage at a glance.

| # | Threat | Concrete mitigation in this spec |
| - | ------ | -------------------------------- |
| S1 | An attacker ships a deceptive imported tab config whose `prompt` or `commands` field contains ANSI control sequences, OSC 8 hyperlinks, bidi overrides, or zero-width characters that hide what would actually run. | B-CONSENT-INERT enumerates an explicit inert rendering contract: HTML-escaped, ANSI surfaced as visible escaped glyphs, bidi/RTL controls replaced with visible placeholders, zero-width and confusable characters surfaced as visible placeholders, control characters rendered as escape sequences, whitespace preserved, no click handlers, no execution, read-only typography. T-CONSENT-INERT-ANSI / T-CONSENT-INERT-BIDI / T-CONSENT-INERT-ZWSP enforce this in tests. |
| S2 | An attacker tricks a user into trusting a config from the settings row without ever showing the user the prompt + commands. | B-TRUST-UPGRADE-DISCLOSURE requires a modal Trust-review dialog presenting the SAME B-CONSENT-CONTENT verbatim payload (rendered inert per B-CONSENT-INERT) before any sidecar write from the settings path. B-TRUST-UPGRADE-NO-BYPASS forbids ANY other code path from writing `"trusted"` to the sidecar without first showing one of the two qualifying disclosure surfaces (consent card or Trust-review modal) in the same atomic UI interaction. |
| S3 | A multi-pane imported config smuggles a high-trust pane past consent by fronting an innocuous pane and getting `[Cancel]` only on the obvious one. | B-CANCEL-MULTIPANE makes `[Cancel]` on ANY pane cancel the ENTIRE tab-config open (all-or-nothing). B-CONSENT-MULTIPANE-STATE defines the four legal per-pane resolved states and forbids partial commits. B-CONSENT-MULTIPANE-NO-LEAK forbids any observable side effect (process spawn, env injection, sidecar write) until the whole open commits. A-CANCEL-MULTIPANE and T-CONSENT-MULTIPANE-CANCEL enforce this. |
| S4 | "Session-scoped consent" memory would let a user grant consent once and then accept follow-up opens silently, conflicting with the imported-until-trusted model. | B-CONSENT-EVERY-OPEN deletes the concept of session-scoped consent entirely: imported configs show the consent card on EVERY open within the same session, after close/reopen, and across sessions. The only durable shortcut is `(path_hash, content_hash)` trust in the local sidecar via B-TRUST-UPGRADE / B-TRUST-UPGRADE-DISCLOSURE. A-CONSENT-EVERY-OPEN and T-CONSENT-EVERY-OPEN enforce this. |

These mitigations are normative; an implementation that
satisfies all acceptance criteria below will close S1–S4. Any
change to the spec that weakens one of S1–S4 MUST also update
this table to make the regression explicit.

## Behavior contract

### Schema and lookup

- B1. Tab-config TOML schema accepts a `profile` string field on
  agent panes:
  ```toml
  [[panes]]
  type = "agent"
  profile = "Coder devbox"
  ```
- B1a. The `profile` field accepts EITHER a bare display name
  (e.g. `"Coder devbox"`) OR a qualified
  `source:name` form (e.g. `"team:reviewer"`,
  `"user:Coder devbox"`, `"builtin:Default"`).
- B1b. Profiles are uniquely identified internally by the
  `(name, source)` pair where `source ∈ {user, team, builtin}`.

### Profile identity invariant (single-source uniqueness)

- B-IDENTITY. **Within a single source (`user`, `team`,
  `builtin`), profile names MUST be unique.** The profile store
  enforces this on every write path:
  - Creating a profile whose name is already taken in the same
    source → reject with an error and prompt the user to rename.
  - Importing a team/builtin profile bundle whose name collides
    with an existing profile in the same source → reject with a
    rename prompt; user must resolve before commit.
  - Renaming a profile to a name already taken in the same
    source → reject.
- B-IDENTITY-1. Because of B-IDENTITY, **same-source duplicate
  names cannot exist**. A bare-name lookup that has narrowed to a
  single source therefore always resolves to at most ONE profile
  in that source. There is no "same-source ambiguity" case to
  handle in lookup.
- B-IDENTITY-2. The only legal multi-match for a bare name is
  **across sources** (e.g. user has `reviewer` and team also has
  `reviewer`). This is resolved deterministically by precedence —
  see B-LOOKUP. It is never ambiguous.

### Profile resolution

- B-LOOKUP. Profile resolution by `profile` field:
  1. If `profile` is qualified `source:name`, look up exactly
     that `(name, source)` pair. If not found → missing
     (B-MISSING). By B-IDENTITY there can be at most one match.
  2. If `profile` is a bare name, search in priority order
     `user > team > builtin` and return the first source that
     contains a profile with that name. By B-IDENTITY that source
     contains at most one such profile, so the result is unique.
  3. If no source contains the name → missing (B-MISSING).
  4. Renaming a profile breaks the binding — same as missing.
  5. Partial matching is never used.

### Qualified `source:name` parsing

- B-QUALIFY. The qualified form `source:name` is parsed by
  splitting on the **first unescaped `:`**. Everything before
  that `:` is the source token; everything after is the profile
  name.
- B-QUALIFY-1. The source token must be one of `user`, `team`,
  `builtin`. Anything else → parse error → treated as missing
  (B-MISSING).
- B-QUALIFY-2. **Profile names that contain `:`** must escape
  each literal colon as `\:` in the qualified form. The parser
  treats `\:` as a literal `:` in the name and does NOT split on
  it. Example: `team:foo\:bar` parses as
  `source = "team", name = "foo:bar"`.
- B-QUALIFY-3. **Profile names that contain `\`** must escape
  each backslash as `\\` in the qualified form. Example:
  `user:path\\to` parses as `source = "user", name = "path\to"`.
- B-QUALIFY-4. The bare-name form (no unescaped `:` in the
  string) is only valid when the profile name itself contains no
  `:` and no `\`. If the profile name contains either, the
  qualified form with escapes is required.
- B-QUALIFY-5. Serialization (export, settings UI) MUST emit the
  escaped qualified form whenever the profile name contains `:`
  or `\`, so that round-trip parsing is deterministic.

### Trust & Consent model

- B-TRUST. Trust state for a tab config is **per-user local
  metadata only**. It is NOT a field on the tab-config TOML and
  is NOT carried in any imported file.
- B-TRUST-STORE. Trust is recorded in a per-user local sidecar
  file `~/.config/warp/tab_config_trust.json` (or the
  platform-equivalent app-data path). Schema:
  ```json
  {
    "<file_path_canonical_hash>:<content_hash>": "trusted"
  }
  ```
  - `file_path_canonical_hash` = SHA-256 of the canonicalized
    absolute path of the source file.
  - `content_hash` = SHA-256 of the tab-config file bytes
    AFTER the import-time strip of any in-file trust marker
    (B-TRUST-STRIP). This guarantees the hash is over normalized
    content and is stable across re-imports of the same file.
  - Only the value `"trusted"` is recorded. Absence from the
    sidecar means `imported` (untrusted) by default.
- B-TRUST-DEFAULT. Every tab config loaded from any external
  source (file import, share link, team config bundle, drag-drop)
  defaults to `imported` (untrusted). The local sidecar is the
  ONLY way a tab config becomes `trusted`.
- B-TRUST-OWNED. Tab configs the user authored from scratch
  inside the in-app Tab Configs editor and saved to their own
  settings are recorded directly as `trusted` in the sidecar
  using the saved-file path + content hash.
- B-TRUST-STRIP. **The import process MUST strip any field named
  `trust`, `owned`, or `imported` from the parsed tab-config
  before storage and before content-hash computation.** External
  files cannot self-declare trust state. Stripping is silent (no
  user-facing error) but is logged at debug level.
- B-TRUST-INVALIDATE. Editing a tab-config file changes its
  content hash, which invalidates any prior `"trusted"` entry
  for that `(path_hash, content_hash)` key. The next open
  defaults to `imported` until the user re-trusts.
- B-TRUST-UPGRADE. Upgrading an `imported` config to `trusted`
  requires an explicit user action via EXACTLY ONE of two
  surfaces:
  1. The "Trust this tab config" checkbox on the consent card
     (B-CONSENT) — combined with a non-cancelled
     `[Open with profile]` resolution (per
     B-CONSENT-MULTIPANE-PROMOTE for multi-pane).
  2. The "Trust this tab config" control on the Tab Configs
     settings row, which MUST first show the Trust-review modal
     defined in B-TRUST-UPGRADE-DISCLOSURE.
  No other code path may write a `"trusted"` entry to the sidecar.
  Trust is one-time, per file content, and does NOT cascade to
  other tab configs.
- B-TRUST-UPGRADE-DISCLOSURE. **Trust promotion from settings
  requires the same verbatim disclosure as the consent card —
  no exceptions, no bypass.** When the user clicks "Trust this
  tab config" from the Tab Configs settings row (i.e., NOT
  inside the consent-card flow), the app MUST present a modal
  "Trust review" dialog before writing the trusted entry to the
  sidecar. The dialog displays the SAME verbatim content
  enumerated in B-CONSENT-CONTENT (profile, autonomy level,
  initial prompt rendered inert per B-CONSENT-INERT, full
  `commands` array rendered inert per B-CONSENT-INERT, all
  other open-time agent inputs, and the source file path) and
  offers exactly two actions:
  1. `[Trust this tab config]` — writes the
     `(path_hash, content_hash)` entry to the sidecar.
  2. `[Cancel]` — leaves trust state unchanged.
  Without this gate the settings path could promote a malicious
  imported config to trusted without the user ever seeing its
  open-time inputs. Sidecar writes from settings are FORBIDDEN
  unless this disclosure dialog has been shown and accepted in
  the same user action.
- B-TRUST-UPGRADE-NO-BYPASS. **Every code path that writes a
  `"trusted"` value to the sidecar MUST first have shown the
  user the B-CONSENT-CONTENT verbatim disclosure (rendered
  inert per B-CONSENT-INERT) AND received an explicit
  affirmative action by the user in the SAME atomic UI
  interaction.** The two and only two qualifying paths are:
  (a) the consent card itself (B-CONSENT-ACTIONS option 1 +
  "Trust this tab config" checkbox, subject to
  B-CONSENT-MULTIPANE-PROMOTE deferral), and (b) the
  Trust-review modal opened from the settings row
  (B-TRUST-UPGRADE-DISCLOSURE). Any other surface that wants
  to offer a "trust this config" affordance — keyboard
  shortcut, command palette, context menu, programmatic API,
  CLI flag — MUST route through one of these two disclosure
  surfaces. There is no "quick trust" path, no "trust without
  preview" path, and no batch-trust path. A future feature
  that adds such a surface without disclosure is a security
  regression and MUST be rejected in review.

### Consent card (imported configs)

- B-CONSENT. When a tab config is `imported` (not `trusted` per
  B-TRUST-STORE) AND any agent pane in it has either:
  - a non-default profile bound, OR
  - a profile whose autonomy mode is anything other than Manual
    (any elevated-autonomy profile, including the default if it
    has been raised), OR
  - any open-time agent input (initial prompt field, `commands`
    array, or any other tab-config-driven dispatch path),
  then the pane MUST display an inline consent card before
  dispatching ANY agent input.
- B-CONSENT-CONTENT. The consent card displays **exact,
  inspectable, verbatim** open-time inputs:
  - **Profile**: name + qualified source, e.g. `team:reviewer`.
  - **Autonomy level**: the profile's mode (Manual / Auto / Yolo
    / etc.) shown literally.
  - **Initial prompt**: full verbatim text of the initial prompt
    field, rendered inert per B-CONSENT-INERT inside a code
    block. If length exceeds 1000 characters, show the first 800
    characters followed by an `[expand]` toggle that reveals the
    rest. The card MUST NOT truncate silently.
  - **`commands` array**: every entry rendered on its own line
    inert per B-CONSENT-INERT inside a code block, in source
    order, no summarization.
  - **Other open-time agent inputs**: any additional
    tab-config-driven dispatch payload (file references,
    env-var injection, working-directory overrides, etc.) is
    disclosed verbatim and inert per B-CONSENT-INERT, one row
    per input, labeled by source.
  - **Source file**: path of the imported tab config (so the
    user can inspect the file directly).
- B-CONSENT-INERT. **Inert rendering contract for untrusted
  prompts, commands, and other open-time inputs.** All verbatim
  fields rendered on the consent card (and on the settings-path
  Trust-review dialog per B-TRUST-UPGRADE-DISCLOSURE) MUST be
  displayed using the inert rendering pipeline:
  1. **No execution.** The string is treated as opaque text —
     it is NOT parsed as a shell command, agent prompt, or
     markdown / HTML / link / image directive while displayed
     in the consent surface.
  2. **No live links / no auto-launch.** URLs, file paths, and
     `command:`-style URIs in the displayed text are NOT
     activatable. Click handlers on the displayed strings are
     disabled. The displayed text MUST NOT trigger any
     navigation, download, or process spawn.
  3. **Escaped output.** The renderer escapes:
     - HTML special characters (`&`, `<`, `>`, `"`, `'`) and
       any markup that the surrounding chrome would otherwise
       interpret.
     - ANSI escape sequences (`\x1b[...m`, hyperlink OSC 8,
       cursor-control sequences) — rendered as visible escaped
       glyphs so a malicious prompt cannot relabel buttons,
       hide content, or fake the consent UI's own chrome.
     - Bidi / RTL-override Unicode controls (U+202A–U+202E,
       U+2066–U+2069, U+200E, U+200F) — replaced with a
       visible placeholder so spoofed direction cannot mask
       intent.
     - Zero-width and confusable characters (ZWJ, ZWNJ,
       ZWSP, soft hyphen) — surfaced as a visible placeholder
       glyph or rendered with a "contains hidden characters"
       badge so command spoofing via invisible runs is
       detectable.
     - Control characters (`\x00`–`\x1f` other than `\n`,
       `\t`) — rendered as escape sequences (e.g. `\x07`).
  4. **Whitespace preserved.** Newlines and tabs in commands
     and prompts are preserved literally so the user sees the
     true line structure of what would run.
  5. **Read-only typography.** Rendered inside a non-editable
     code block with monospace font; the user cannot
     accidentally edit the disclosed text from the consent
     card.
- B-CONSENT-ACTIONS. The card offers exactly three actions:
  1. `[Open with profile]` — proceed: bind the profile, dispatch
     all disclosed inputs, and (optionally, via a checkbox)
     "Trust this tab config" to record the entry per
     B-TRUST-UPGRADE.
  2. `[Open with default profile]` — see B-CANCEL-DEFAULT.
  3. `[Cancel]` — see B-CANCEL.
- B-CONSENT-EVERY-OPEN. **Imported (untrusted) configs require
  consent on EVERY open — there is no per-session memory of
  prior consent.** The consent card is shown each time an
  imported config is opened, including:
  - Multiple opens within the same app session (e.g., closing
    the tab and re-opening the same imported config).
  - Multiple panes within the same tab-config open that each
    bind their own imported config (each pane gets its own
    consent card per B-CONSENT-MULTIPANE).
  - The next session, exactly as before.
  Choosing `[Open with profile]` WITHOUT checking "Trust this
  tab config" does NOT remember consent. The only way to skip
  the consent card on subsequent opens is to upgrade the config
  to `trusted` per B-TRUST-UPGRADE — which records a
  `(path_hash, content_hash)` entry in the local sidecar. This
  resolves the prior tension between "session-scoped consent"
  and the imported-until-trusted model: there is **no**
  session-scoped consent state. Only sidecar trust persists.

### Multi-pane consent / cancel behavior

- B-CONSENT-MULTIPANE. A single tab-config open may instantiate
  several agent panes, each with its own `profile` field and
  its own open-time agent inputs. The trust + consent gate runs
  **per pane**, in source order:
  1. Each pane's trust state is computed from the SAME shared
     `(path_hash, content_hash)` of the tab-config file, so
     every pane in the same tab-config open shares the same
     trust verdict (trusted-fast-path-for-all OR
     consent-required-for-all).
  2. When the tab-config is `imported` (untrusted), the panes
     that need a consent gate per B-CONSENT (those with a non-
     default profile, an elevated-autonomy profile, or any
     open-time agent input) display their consent cards in
     source order. Each card discloses ONLY that pane's
     verbatim profile/prompt/commands/other inputs, but the
     header indicates "Pane K of N from `<file>`" so the user
     can see the full set.
  3. **No agent process is spawned and no input is dispatched
     in ANY pane until every consent-gated pane in the same
     tab-config open has been resolved.** Panes that don't
     need a consent gate (e.g., agent panes with no elevated
     profile and no open-time input) wait at a "pending tab
     open" state during this resolution; they are NOT opened
     ahead of consent resolution.
- B-CANCEL-MULTIPANE. **`[Cancel]` on ANY pane's consent card
  cancels the ENTIRE tab-config open**, per B-CANCEL: no panes
  are created, no agent processes start, no inputs are
  retained, the tab-config import path is rolled back as a
  whole. Partial opens are forbidden — the consent gate is
  all-or-nothing across the tab config so a malicious config
  cannot smuggle a high-trust pane through by fronting a less
  threatening pane and getting `[Cancel]` only on the obvious
  one.
- B-CANCEL-DEFAULT-MULTIPANE. `[Open with default profile]` on
  a single pane's consent card applies B-CANCEL-DEFAULT to
  THAT pane only (its profile is discarded, all its open-time
  agent inputs are cleared) and continues with the next
  consent-gated pane. The remaining panes still require their
  own per-pane action.
- B-CONSENT-MULTIPANE-PROMOTE. The "Trust this tab config"
  checkbox appears on the FIRST consent card of a tab-config
  open and is shared across all panes in the same open. If the
  user accepts the first pane with the checkbox checked, the
  trust write is deferred until ALL consent-gated panes in
  the open have been resolved with `[Open with profile]` and
  no `[Cancel]` was chosen on any pane. If `[Cancel]` (or
  `[Open with default profile]` for any pane) occurs after
  the checkbox was checked, no sidecar entry is written. (The
  user can still trust the file later from the settings row
  via B-TRUST-UPGRADE-DISCLOSURE.)
- B-CONSENT-MULTIPANE-STATE. **Per-pane consent state during
  multi-pane resolution.** While the user is stepping through
  consent cards 1..N for a multi-pane imported tab-config
  open, each pane has exactly one of these resolved states:
  - `Pending` — its consent card has not been shown yet
    (panes after the currently displayed card).
  - `OpenWithProfile` — user chose `[Open with profile]` on
    that pane's card; the pane's profile binding and
    open-time inputs are queued for dispatch IF the whole
    open commits.
  - `OpenWithDefault` — user chose `[Open with default
    profile]` on that pane's card; the pane will open under
    the default profile with all open-time agent input
    cleared (per B-CANCEL-DEFAULT).
  - `CancelledAll` — user chose `[Cancel]` on that pane's
    card; per B-CANCEL-MULTIPANE this terminates the entire
    tab-config open immediately. No further consent cards
    are shown; all queued state from earlier panes is
    discarded. The tab-config open is rolled back as a
    whole.
  An open commits only when every consent-gated pane has
  reached `OpenWithProfile` or `OpenWithDefault`. There is
  no third "skip pane" or "partial open" state.
- B-CONSENT-MULTIPANE-NO-LEAK. Until the open commits, NO
  side effect from any pane is observable: no agent process
  spawned, no working directory created, no env var
  injection, no sidecar trust write, no shell history
  pollution. If `CancelledAll` occurs, the dispatcher state
  is wiped as if the open never started. This includes panes
  that did NOT require a consent gate (default-profile +
  no-input panes), which sit in pending-tab-open state
  during multi-pane consent resolution per
  B-CONSENT-MULTIPANE step 3.

### Cancel and "default profile" paths

- B-CANCEL. **Cancel rolls back the open.** No pane is opened.
  No agent process starts. No input is retained. No draft pane
  is created. The tab-config import path is rolled back to the
  state before the user attempted to open the imported config.
  Specifically: imported draft input does NOT survive Cancel.
- B-CANCEL-DEFAULT. `[Open with default profile]` opens the
  pane bound to the user's default profile, BUT:
  - The configured `profile` from the tab config is discarded
    for this open.
  - **All open-time agent input fields are CLEARED before the
    pane opens**: the initial prompt field, the `commands`
    array, and any other tab-config-driven dispatch payload are
    all dropped. The pane opens with empty agent input.
  - The user must manually re-enter any prompt or commands they
    want to run under the default profile.
  - This prevents the consent gate from being bypassed by
    "default profile elevation" — i.e., a high-autonomy default
    cannot piggyback on imported-config inputs.
- B-CANCEL-TRUSTED-NA. `[Cancel]` and `[Open with default
  profile]` only apply to `imported` configs (the consent card
  is the only place they appear). Trusted configs go through
  B-TRUST-FAST.

### Trusted-config fast path

- B-TRUST-FAST. A tab config marked `trusted` in the sidecar
  skips the consent card and dispatches the configured profile
  + input directly on open. Profile resolution still runs and
  still respects B-MISSING.

### Missing / disabled-agent fallback

- B-MISSING. If `profile` is set but cannot be resolved at open
  time (renamed, deleted, never existed, or qualified-form parse
  error), the pane opens in **Disabled-Agent state**:
  - No agent process is started.
  - The initial prompt input AND the `commands` array are HELD
    (not dispatched, not discarded).
  - A one-time toast appears scoped to the affected tab-config
    open. Wording: `"Profile '[name]' not found — this tab will
    not start an agent. Edit the tab config or install the
    profile."`
  - The toast dismisses on user click or after 8 seconds.
- B-MISSING-1. There is NO silent fallback to a different
  profile (default or otherwise) when the configured profile
  cannot be resolved. The user must edit the config to fix it.
- B-MISSING-2. Multiple panes in the same tab-config open
  referencing the same missing name produce ONE coalesced toast
  for that name, mentioning the number of affected panes.
  Distinct names get distinct toasts. Reopening the same tab
  config in a later session may show the toast again.
- B-MISSING-3. The Disabled-Agent state offers an inline "Edit
  tab config" affordance. On save with a now-resolvable profile,
  the pane re-attempts open and proceeds through the normal
  trust/consent path (B-CONSENT / B-TRUST-FAST).
- B-MISSING-NOAMBIG. By B-IDENTITY there is no "ambiguous bare
  name" case at lookup time. Cross-source matches are resolved
  deterministically by precedence (B-LOOKUP), not flagged as
  ambiguous.

### Other

- B3. `profile` is optional — omitting it preserves today's
  behavior (default profile, subject to B-CONSENT if the config
  is `imported` AND has open-time agent input).
- B4. The setting roundtrips through tab-config import/export
  for the `profile` field, including the qualified
  `source:name` form (with escapes per B-QUALIFY). The trust
  marker is NEVER serialized; trust is local sidecar state.
- B5. The Tab Configs settings UI gets a profile picker on the
  agent-pane row. Picker shows current profile names from
  `Settings → Agents → Profiles`. Profiles whose bare names
  collide ACROSS sources are shown in qualified `source:name`
  form so the user's choice is unambiguous in the saved config.
- B6. Profile name resolution does not use partial matching.

### Ordering invariant

- B-ORDER. **Profile resolution AND the trust check both
  complete before ANY initial input or `commands` array is
  dispatched.** The agent dispatcher receives a fully bound
  profile context as the argument of its very first invocation.
  Implementations MUST NOT spawn the agent, dispatch input, or
  begin command execution before profile resolution and consent
  resolution have both terminated.

## Acceptance criteria

- A1. A trusted tab config with `profile = "Coder devbox"` opens
  an Agent Mode pane already on that profile, with no consent
  card, regardless of whether initial input exists.
- A2. With `profile` omitted, behavior is identical to today
  (subject to A-TRUST-IMPORTED if the config is `imported`).
- A3. With `profile = "Nonexistent"`, the pane opens in
  Disabled-Agent state with the missing-profile toast per
  B-MISSING coalescing scope. Initial input AND `commands` are
  held, not dispatched.
- A-IDENTITY. The profile store rejects creating, importing, or
  renaming to a name already taken in the same source. Two
  profiles named `reviewer` cannot coexist within `user`. They
  CAN coexist across sources (e.g. `user:reviewer` and
  `team:reviewer`); the bare name `reviewer` resolves to
  `user:reviewer` by precedence.
- A4. Tab-config TOML round-trips the `profile` field through
  import/export, including qualified `source:name` form with
  escapes per B-QUALIFY. The trust marker is NOT in exported
  TOML.
- A5. (Trusted-config flow.) A trusted tab config with both
  `profile = "Coder devbox"` and initial agent input dispatches
  the input under that profile on open with no consent card.
- A-TRUST-IMPORTED. (Imported-config flow.) An imported tab
  config with any non-default / elevated-autonomy profile OR any
  open-time agent input MUST show the consent card displaying
  the exact verbatim profile, autonomy, prompt, commands, and
  any other open-time inputs (B-CONSENT-CONTENT). It MUST NOT
  dispatch anything until the user picks
  `[Open with profile]`.
- A-CANCEL. `[Cancel]` rolls back the open: no pane, no agent,
  no draft, no retained input.
- A-CANCEL-MULTIPANE. `[Cancel]` on ANY pane's consent card in
  a multi-pane tab-config open rolls back the ENTIRE tab open
  — zero panes are created, zero agents start, zero inputs
  remain.
- A-CONSENT-MULTIPANE-ORDER. A multi-pane imported tab config
  shows one consent card per consent-gated pane, in source
  order, with "Pane K of N" header. No agent process is
  spawned in any pane until all consent-gated panes are
  resolved.
- A-CONSENT-EVERY-OPEN. An imported (untrusted) tab config
  shows the consent card on EVERY open — within the same
  session, after closing/reopening, and across sessions.
  Choosing `[Open with profile]` without "Trust this tab
  config" does NOT skip the next consent prompt.
- A-CONSENT-INERT. Every verbatim field on the consent card
  (initial prompt, every command in the `commands` array,
  every other open-time input) is rendered inert per
  B-CONSENT-INERT: no execution, no live links, ANSI escapes
  surfaced as visible glyphs, bidi/zero-width controls
  replaced with placeholders, control characters escaped.
- A-TRUST-UPGRADE-DISCLOSURE. Promoting a config to trusted
  from the Tab Configs settings row pops a Trust-review modal
  showing the same B-CONSENT-CONTENT verbatim disclosure
  rendered inert per B-CONSENT-INERT, with `[Trust this tab
  config]` and `[Cancel]` actions. No sidecar write occurs
  unless `[Trust this tab config]` is chosen in that modal.
- A-CANCEL-DEFAULT. `[Open with default profile]` opens the
  pane on the default profile and clears every open-time agent
  input field (initial prompt, `commands` array, others). The
  pane opens empty; the user must re-enter inputs manually.
- A-TRUST-PROMOTE. Choosing `[Open with profile]` with the
  "Trust this tab config" checkbox checked (or invoking "Trust
  this tab config" from settings) records a `trusted` entry in
  the local sidecar keyed by `(path_hash, content_hash)`.
  Subsequent opens of the same file (same content) skip the
  consent card.
- A-TRUST-INVALIDATE. Editing the imported tab-config file
  changes its content hash and invalidates the trust entry. The
  next open shows the consent card again.
- A-TRUST-STRIP. An imported tab-config TOML containing a
  `trust = "owned"` field (or any synonym) is parsed with that
  field stripped. The file is treated as `imported` regardless
  of in-file claims.
- A-COMMANDS. Same trust + missing-profile rules apply
  identically to the `commands` array as to the initial prompt
  field.
- A-DISABLED-HOLD. In Disabled-Agent state, neither the initial
  prompt field nor the `commands` array runs. Editing the config
  to a resolvable profile and saving re-attempts open and then
  follows the normal trust/consent path.

## Implementation pointers

- Tab-config schema in
  `app/src/persistence/tab_configs/...` (search for `pane` /
  `type = "agent"` parsing). Add `profile: Option<String>`. Do
  NOT add a trust field to the schema. Implement B-TRUST-STRIP
  in the deserializer.
- Trust sidecar in
  `app/src/persistence/tab_config_trust/...` reading and writing
  `~/.config/warp/tab_config_trust.json`.
- Pane open path in the workspace view's tab-config restoration
  logic. Insert ordering gate per B-ORDER: resolve → trust check
  → (consent card if needed) → dispatch.
- Profile model in
  `app/src/ai/execution_profiles/...` exposes a lookup-by
  `(name, source)` plus a bare-name resolver implementing
  `user > team > builtin` precedence. Enforce B-IDENTITY at
  every write path (create, import, rename).
- Qualified-name parser in the tab-config schema layer
  implementing B-QUALIFY (split on first unescaped `:`,
  `\:` and `\\` escapes).
- Disabled-Agent state should be a first-class pane state, not
  an ad-hoc "no agent process" branch — surface it in the pane
  view, the status bar, and the inline edit-config affordance.

## Test plan

- T1. Schema round-trip with the `profile` field, including
  qualified `source:name` form.
- T2. Open path applies the named profile when present (trusted
  config flow).
- T3. Open path enters Disabled-Agent state on missing profile,
  emits the coalesced toast, and HOLDS both initial input and
  `commands` array (T3-INPUT, T3-COMMANDS sub-cases).
- T-IDENTITY. Profile store rejects creating a second `user`
  profile named `reviewer`. Same for import collision and for
  rename collision. Cross-source `user:reviewer` +
  `team:reviewer` coexist; bare `reviewer` resolves to
  `user:reviewer`.
- T_qualified_name_with_colon. Profile name `foo:bar` exists in
  `team`. Tab config sets `profile = "team:foo\\:bar"`. Lookup
  resolves to `(team, "foo:bar")`. Export of that binding emits
  exactly `"team:foo\\:bar"`.
- T_qualified_name_with_backslash. Profile name `path\to`
  exists in `user`. Tab config sets
  `profile = "user:path\\\\to"`. Lookup resolves to `(user,
  "path\to")`. Round-trip is stable.
- T_qualified_invalid_source. Tab config sets
  `profile = "wat:foo"`. Source token is invalid → treated as
  missing per B-QUALIFY-1; pane enters Disabled-Agent state.
- T4. UI picker shows current profile list, surfaces qualified
  `source:name` only for cross-source bare-name collisions.
- T5. Imported tab config with elevated profile + initial input
  shows the consent card displaying verbatim prompt + commands
  + any other open-time inputs;
  `[Open with profile]` dispatches; `[Cancel]` rolls back fully
  (no pane, no draft); `[Open with default profile]` opens on
  default and clears all agent inputs.
- T5-TRUSTED. Trusted tab config with the same content
  dispatches immediately with no consent card.
- T5-PROMOTE. `[Open with profile]` + "Trust this tab config"
  records a sidecar entry; subsequent open skips card.
- T5-INVALIDATE. Editing the trusted file changes content hash;
  next open shows the consent card again.
- T5-STRIP. Imported tab-config TOML containing
  `trust = "owned"` is parsed with that field stripped; file is
  still treated as `imported`.
- T5-CANCEL-DEFAULT-CLEARS. Imported config has
  `commands = ["rm -rf /"]` and an initial prompt. User picks
  `[Open with default profile]`. Pane opens; the `commands`
  array did NOT run; the prompt field is empty; nothing was
  retained.
- T5-CONSENT-LONG-PROMPT. Initial prompt is 5000 characters.
  Consent card shows the first 800 characters with an
  `[expand]` toggle and never truncates silently.
- T6. Multiple panes referencing the same missing profile during
  one tab-config open produce one coalesced toast; reopening the
  tab config can show it again.
- T-COMMANDS. `commands` array is gated by the same
  consent/missing-profile rules as the initial prompt; both are
  held in Disabled-Agent state.
- T-ORDER. **Ordering test.** Profile resolution AND trust
  check both terminate before the dispatcher's first invocation.
  Verified via instrumentation that records the time of profile
  bind, trust resolution, and first dispatch — assertion is
  `bind_done <= dispatch_start && trust_done <= dispatch_start`.
- T-CONSENT-INERT-ANSI. Imported tab config sets `commands =
  ["echo \\u001b[31mfake-error\\u001b[0m"]` and `prompt =
  "\\u001b]8;;file:///etc/passwd\\u0007click here\\u001b]8;;\\u0007"`.
  Consent card MUST render the ANSI color code as a visible
  escaped sequence (not as red text), and the OSC 8 hyperlink
  as visible escaped glyphs (not an activatable link).
  Click on the displayed text must produce no navigation, no
  download, no process spawn.
- T-CONSENT-INERT-BIDI. Imported tab config sets
  `prompt = "rm -rf \\u202E/safe-path\\u202C"` (RTL override).
  Consent card MUST replace the bidi-control characters with a
  visible placeholder (and a "contains hidden characters"
  badge), so the user sees the true command text and cannot be
  spoofed by direction reversal.
- T-CONSENT-INERT-ZWSP. Imported tab config sets
  `commands = ["rm\\u200B -rf /"]` (zero-width space hidden in
  command). Consent card MUST surface the ZWSP as a visible
  placeholder; the displayed text makes the spoof visible.
- T-CONSENT-MULTIPANE-CANCEL. Imported tab config has 3 agent
  panes, all with elevated profiles + initial prompts. Open
  the config; consent card 1 of 3 appears. Click `[Cancel]`.
  Assert: zero panes are created, zero agent processes
  spawned, zero inputs retained, the tab-config open is
  rolled back as a whole (no partial open of the first two
  panes).
- T-CONSENT-MULTIPANE-MIXED. Imported tab config has 3 panes:
  pane 1 elevated profile, pane 2 default profile + no input,
  pane 3 elevated profile. Open: consent cards appear in
  source order ONLY for panes 1 and 3 (header reads "Pane 1 of
  2 from <file>" and "Pane 2 of 2 from <file>"). Pane 2 stays
  in pending-tab-open state until both cards are resolved.
  Approve both → all 3 panes open in source order. No pane
  spawns its agent before the second consent card resolves.
- T-CONSENT-EVERY-OPEN. Imported tab config never trusted.
  Open it, choose `[Open with profile]` without ticking "Trust
  this tab config". Close the tab. Re-open the same file in
  the SAME session. Assert: consent card appears again. Repeat
  across a new session — consent card appears again. Only
  ticking "Trust this tab config" + B-CONSENT-MULTIPANE-PROMOTE
  resolution writes the sidecar entry that skips future cards.
- T-TRUST-UPGRADE-FROM-SETTINGS. From the Tab Configs settings
  row of an imported config, click "Trust this tab config".
  Assert: a Trust-review modal pops up showing the same
  verbatim profile/prompt/commands/other inputs (rendered inert
  per B-CONSENT-INERT). Choose `[Cancel]` → sidecar unchanged.
  Re-trigger and choose `[Trust this tab config]` → sidecar
  records the `(path_hash, content_hash)` entry; subsequent
  open uses the trusted-fast-path.
- T-TRUST-UPGRADE-MULTIPANE-DEFER. Multi-pane imported config.
  On the FIRST consent card, tick "Trust this tab config" and
  click `[Open with profile]`. Then on the SECOND consent
  card, click `[Cancel]`. Assert: NO sidecar entry was
  written (deferred trust write is dropped because the open
  was cancelled).

## Out of scope

- Multiple profiles per pane (e.g. fallbacks).
- Profile autocompletion in the TOML editor.
- Per-tab-config profile overrides at the tab-config level (this
  is per-pane).
- Cross-machine profile sync — profiles are looked up locally;
  team/builtin sources are populated through existing channels.
- Cross-machine sync of the trust sidecar — trust is a local
  per-user signal and does NOT sync.
