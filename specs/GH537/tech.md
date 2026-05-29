# TECH.md — Honor user-defined shell bindkeys in Warp's input editor

Issue: https://github.com/warpdotdev/warp/issues/537
Product spec: [`product.md`](./product.md)

## Context

Warp's input editor receives raw keystrokes, matches them against the
`Keymap` table, and dispatches `InputAction` variants. Today that table
knows nothing about the user's shell bindings, so user customizations
(`bindkey '^X^E' edit-command-line`, readline `bind`, fish `bind`) are
ignored. See `product.md` for the user-visible behavior we want.

Relevant code, with line ranges:

- **Input editor and actions** — `app/src/terminal/input.rs (1072-1149)`.
  `InputAction` is the dispatched action type. Today it covers
  Warp-flavored actions (`FocusInputBox`, `CtrlR`, `CtrlD`,
  `MaybeOpenCompletionSuggestions`, etc.) but does **not** have the
  fine-grained editor verbs ZLE / readline expose
  (`backward-kill-word`, `transpose-chars`, `kill-line`, `yank-pop`,
  `up-history`, `vi-cmd-mode`, …). The buffer model lives in
  `InputBufferModel` in the same file. `crates/editor/src/editor.rs
  (18-55)` exposes the underlying `EditorView` trait.
- **Keymap** — `crates/warpui_core/src/keymap.rs (25-38, 44-49, 72-150)`.
  `Keymap { fixed_bindings, editable_bindings }` indexed by name, with
  `Trigger::{Keystrokes(Vec<Keystroke>), Standard(StandardAction),
  Custom(CustomTag)}` and `ContextPredicate` for context-scoped layering.
  Resolution: `editable_bindings` (user-overridden) wins over
  `fixed_bindings` (Warp defaults). Matching lives in
  `crates/warpui_core/src/keymap/matcher.rs`.
- **Shell type and session** — `crates/warp_terminal/src/shell/mod.rs
  (58-96, 250-255)`. `ShellType { Zsh, Bash, Fish, PowerShell }`,
  `Shell { type, version, options, plugins, shell_path }`,
  `ShellStarter::init()` at line 79. `app/src/terminal/local_tty/shell.rs
  (1-200)` for spawn details.
- **Bootstrap and DCS hooks** — `app/src/terminal/bootstrap.rs (1-150)`
  injects a per-shell init script from `bundled/bootstrap/{zsh,bash,fish,
  pwsh}.sh`. Script-to-app communication uses
  `app/src/terminal/model/ansi/dcs_hooks.rs (1-150)`: `DProtoHook`
  variants (`Bootstrapped`, `Precmd`, `Preexec`, `InputBuffer`,
  `InitShell`, …) carry hex-encoded JSON payloads
  (`HEX_ENCODED_JSON_MARKER = 'd'`). DCS dispatch arrives as
  `ModelEvent::PluggableNotification` in
  `app/src/terminal/model_events.rs (468-472)`. **There is no live
  "invisible command exec" primitive today**; bootstrap-emitted DCS
  payloads are the right plumbing to extend.
- **Settings** — `app/src/terminal/keys_settings.rs (15-71, 26-34)`.
  `define_settings_group!` macro is the pattern for new boolean toggles
  (see `quake_mode_enabled`). Feature flags live in
  `crates/warp_features/src/lib.rs (9+)`.
- **Telemetry** — `app/src/server/telemetry/events.rs (1237+, 2920)`.
  `TelemetryEvent` enum + `send_telemetry_from_ctx!` macro.
- **Vi-mode tracking** — none today. The `vim` crate is Warp's own
  in-editor vi emulation, not shell awareness.

## Proposed changes

The implementation has five logical pieces. Each maps cleanly to one
subsystem above.

### 1. Bootstrap-side binding query

Extend the bootstrap scripts to dump the user's binding table to Warp
via a new DCS hook variant. Doing the query in bootstrap (rather than
adding a runtime invisible-exec primitive) avoids polluting history,
scrollback, and last-status; it also runs before the first prompt so
bindings are available when the user starts typing.

- `bundled/bootstrap/zsh.sh`: discover keymaps dynamically with
  `bindkey -l` (this enumerates the standard set — `main`, `emacs`,
  `viins`, `vicmd`, `vivis`, `viopp`, `command`, `isearch`,
  `menuselect` — and any user-defined keymaps created via
  `bindkey -N <name>`), then run `bindkey -L -M $keymap` per keymap and
  emit a JSON object `{ keymap_name: [ { keys, widget }, … ] }`. Also
  emit `KEYMAP` so the active keymap is known. User-defined keymaps
  pass through with their declared name; the matcher honors them when
  they are referenced as the active keymap (resolves PRODUCT #2's
  reference to "user-defined keymaps").
- `bundled/bootstrap/bash.sh`: `bind -p` for the current keymap and
  `bind -p -m emacs / vi-insert / vi-command` for the others. Detect vi
  vs emacs via `set -o | grep -E '^(vi|emacs)'`.
- `bundled/bootstrap/fish.sh`: this requires reworking the existing
  bootstrap, which currently sets
  `fish_key_bindings = fish_default_key_bindings` (line 306) and then
  installs four Warp-required binds (`\cP`, `\ep`, `\ew`, `\ei`) on
  top — clobbering any user `fish_vi_key_bindings` setting and any
  user-installed binds. To honor user fish bindings without losing
  Warp's required reporting binds, we change the bootstrap to:

  1. Capture the user's `fish_key_bindings` value at the very top of
     the bootstrap, and stop the unconditional reset at line 306. The
     user's chosen scheme runs as configured.
  2. After the user's scheme runs, install Warp's four reserved binds
     (`\cP`, `\ep`, `\ew`, `\ei`) explicitly in every bind mode the
     user uses (default, insert, visual for vi mode; default for
     emacs; plus any custom modes discovered via `bind -L`). Those
     four keys are reserved for Warp and intentionally shadow user
     bindings on them — the explicit precedence boundary from
     PRODUCT #14.
  3. Snapshot the resulting `bind` output per mode and emit it as the
     `ShellBindings` payload. The vi-mode-vs-input-reporting conflict
     that originally motivated the reset is resolved here because the
     reporting bind is reinstalled in whichever mode is active, instead
     of the scheme being reset wholesale.

  Mode tracking uses `$fish_bind_mode` for the initial snapshot and
  the in-app vi state machine described in the open-questions section
  for transitions.

The payload is emitted as a new `DProtoHook::ShellBindings` variant in
`dcs_hooks.rs` carrying `{ shell, keymaps: Vec<KeymapTable>,
active_keymap, schema_version, nonce }`. Reuse `HEX_ENCODED_JSON_MARKER`.

The `ShellBindings` payload is a privileged terminal-control message
(it can rewrite local key handling) and is only accepted from the
bootstrap context:

- Each Warp-spawned shell receives a per-session, per-tab nonce in its
  initial environment (`WARP_BOOTSTRAP_NONCE`). The very first action in
  the bootstrap script is to copy this value into a non-exported,
  shell-local variable (`typeset -g` in zsh, plain assignment in bash
  with `export -n`, `set -l` plus careful scoping in fish), then
  `unset WARP_BOOTSTRAP_NONCE` and remove it from the inherited
  environment so it is not visible to any descendant process. Every
  `ShellBindings` and `Precmd` payload the bootstrap emits embeds this
  value. The app rejects any payload whose nonce does not match the
  expected value for that tab.

  **Threat model** (documented explicitly so the limits are not
  oversold). The nonce defends against:
  - Innocent process output that happens to contain a DCS sequence
    (`cat`'d binary file, curl response, log dump, terminal-art).
  - Descendants of the user's shell that did not exist at bootstrap
    time and never had the chance to read the nonce.

  It does **not** defend against:
  - A process spawned during the window between the shell starting
    and the bootstrap unsetting the variable. For zsh and bash this
    window is closed by making the unset the first non-trivial line
    of the bootstrap, before any user rc file is sourced.

    **Fish-specific caveat.** Warp launches fish as
    `fish -f no-mark-prompt --login --init-command '<bootstrap>'`
    (`app/src/terminal/local_tty/shell.rs:632`). Fish runs `config.fish`
    and any user functions *before* `--init-command`, so the env-var
    nonce is readable to user code that runs at config time. To close
    this gap the fish path passes the nonce out-of-band: Warp writes
    the nonce to a tempfile under the user's runtime dir with mode
    `0600`, passes the path as the first argument of `--init-command`,
    and the bootstrap reads it then `rm`s the file before any further
    work. The `WARP_BOOTSTRAP_NONCE` env var is not used for fish at
    all. This brings fish to parity with zsh/bash on later-spawned
    descendants but does not protect against an adversarial
    `config.fish` written before Warp launched, which is consistent
    with the same-uid threat model below.
  - A same-user process that already has read access to the parent
    shell's environment (`/proc/<pid>/environ` on Linux,
    `procfs`/`ps eww` on macOS — both gated by same-uid). Such a
    process can already inject keystrokes through `TIOCSTI` (where
    enabled), modify rc files, or attach via debugger; defending the
    DCS channel against this attacker offers no marginal security.
  - A privileged adversary; out of scope for any user-mode mitigation.

  This trust boundary is the same one Warp's existing shell-integration
  hooks already implicitly rely on. The nonce makes that boundary
  explicit and raises the bar above pure-output spoofing.
- **Validation order, single rule.** Validation runs in three
  strictly ordered phases on the receive side, each of which rejects
  the entire payload on failure:
  1. **Pre-decode byte cap.** The DCS frame's hex-decoded byte length
     is checked against the 256 KiB total cap *before* JSON
     deserialization runs. Frames exceeding the cap are dropped at
     the framing layer and logged; no allocation for parsed structures
     happens. This bounds memory and CPU before untrusted data
     reaches `serde_json`.
  2. **Schema decode.** JSON is decoded into the `ShellBindings`
     struct. Any field type mismatch, unknown `schema_version`, or
     malformed `Keystroke` string discards the entire payload — no
     partial application. There is no per-entry "drop one and keep
     the rest" branch.
  3. **Post-decode bounds.** After successful decode, the parsed
     structure is checked against the per-entry caps (max 4 KiB
     per binding entry, max 16 keymaps, max 8192 bindings total).
     Any violation discards the entire payload.

  The previous draft's "drop oversized entries before parsing"
  language is retired; the rule is uniform — every validation
  failure is whole-payload rejection so the app never applies a
  partial table.
- The same nonce check applies to the binding-hash field on the
  existing `Precmd` hook; an unsigned or mismatched hash leaves the
  previous binding table in place.

### Re-query mechanism

Re-queries are driven entirely shell-side; the app never has to mutate
shell state to trigger a re-emit (which the running shell can't observe
anyway — flipping an env var from outside has no effect on the live
session). The bootstrap script keeps a shell-scoped variable
`__warp_bindings_hash` initialized at startup to the hash emitted
alongside the first `ShellBindings` payload. On every `precmd` the
script:

1. Recomputes the 64-bit hash of the current binding table.
2. Emits the hash in the `Precmd` DCS payload (informational; the app
   uses it for telemetry and to detect mid-session resyncs).
3. If the new hash differs from `__warp_bindings_hash`, emits a fresh
   `ShellBindings` payload with the full table and updates
   `__warp_bindings_hash` to the new value.

The app-side handler simply consumes whatever arrives. Steady state is
one hash computation per prompt; the full payload is re-emitted only on
real changes (new `bindkey`, mode switch via `bindkey -v`, sourcing a
new rc file, plugin rebind). PRODUCT #26 holds because the work runs
inside `warp_precmd` after the user's command output, asynchronously to
keystrokes.

**Preserving shell state during the hash step (PRODUCT #27).** The
hash function runs as the very first action of `warp_precmd` and must
leave shell-observable state untouched. The discipline:

- **Last-status (`$?` / `$status`).** Save before any other
  expression: zsh `local __warp_status=$?`, bash
  `local __warp_status=$?`, fish `set -l __warp_status $status`. Any
  value the user reads from `$?` later in their own `precmd` chain
  sees the saved value, restored via `return $__warp_status` at the
  end of the function (or `set -e status $__warp_status` in fish).
- **Shell options.** No `set -o`, `setopt`, `shopt`, or
  `set -gx fish_<option>` calls inside the hash path. The hash reads
  bindings via `bindkey -L` / `bind -p` / `bind`, which are pure
  reads.
- **Keymap state.** No `bindkey -v` / `bindkey -e` / `set -o vi` /
  `set fish_key_bindings ...` calls; the hash only reads. Specifically
  for zsh, do not `bindkey -A` between maps, and do not change
  `KEYMAP` (it is read as a value but never assigned).
- **Variables.** All temporaries are `local`/`typeset -g
  __warp_<name>` (zsh, bash) or `set -l __warp_<name>` (fish), with a
  `__warp_` prefix to avoid collisions with user variables. The
  shell-scoped `__warp_bindings_hash` tracker is the single
  long-lived variable; it is created with `typeset -g`/`set -g`
  exactly once on bootstrap entry.
- **Pipelines.** The hash computation avoids subshells where
  possible (subshells in zsh/bash inherit `$?` clobbering rules).
  Where a subshell is unavoidable, `$?` is captured before the
  subshell and restored after.
- **Aliases.** All command invocations inside `warp_precmd` use
  `\bindkey` / `command bind` / `builtin bind` form so user-defined
  aliases or function shadowing of `bindkey` / `bind` cannot
  interfere with the read or with state.
- **Traps and DEBUG hooks.** zsh's `TRAPDEBUG` and bash's `trap …
  DEBUG` are not modified. The hash function does not add or remove
  any trap.

A unit test under `crates/integration` runs each shell with a
synthetic precmd chain that asserts every one of these invariants
(`$?` round-trips an arbitrary value, every `set -o` flag is
unchanged, `KEYMAP` is unchanged, no new shell variables outside the
`__warp_` prefix exist after `warp_precmd` returns).

### 2. Shell-bindings storage on `Shell`

Add `bindings: Option<ShellBindings>` and `active_keymap: KeymapMode` to
the `Shell` struct in `crates/warp_terminal/src/shell/mod.rs`. New
types:

```rust
pub struct ShellBindings {
    pub schema_version: u32,
    pub keymaps: HashMap<KeymapMode, Vec<ShellBinding>>,
    pub table_hash: u64,
}

pub struct ShellBinding {
    pub keys: Vec<Keystroke>,           // parsed from "^X^E", "\C-x\C-e", "\\cx\\ce"
    pub widget: ShellWidget,            // see #3
    pub raw_widget_name: String,        // for telemetry/debug UI
}

pub enum KeymapMode {
    Emacs,
    ViInsert,
    ViCommand,
    ViVisual,
    ViReplace,           // overwrite mode (zsh `vi-replace`,
                         // bash `vi-replace-mode`, fish `replace_one`/
                         // `replace`).
    ViOperatorPending,   // post-operator state for vi (`d`, `c`, `y`,
                         // etc., awaiting motion) — zsh `viopp`,
                         // and the equivalent transient state in
                         // bash/fish. Required for the dispatch
                         // transitions in the open-questions
                         // vi-mode section below.
    Other(String),       // user-defined zsh keymaps from `bindkey -N`,
                         // surfaced verbatim. The matcher consults
                         // these only when the active keymap reported
                         // by the shell matches the same name.
}
```

Mutation flows through a new `ModelEvent::ShellBindingsUpdated { tab_id,
bindings }` raised when a `ShellBindings` DCS hook arrives.
`active_keymap` is updated from the `Precmd` payload.

### 3. Widget mapping

`ShellWidget` is an enum covering the widgets enumerated in PRODUCT.md
#10 — e.g. `BackwardKillWord`, `KillLine`, `AcceptLine`, `Yank`,
`HistorySearchBackward`, `ViCmdMode`, `CompleteWord`,
`SelfInsert(String)`, `Unsupported(String)`. Parsing
`bindkey -L` / `bind -p` / fish `bind` happens in a new
`crates/warp_terminal/src/shell/bindings.rs` with three small parsers
(one per shell) feeding a common normalizer.

This forces a real expansion of `InputAction` in
`app/src/terminal/input.rs`. Today's coarse actions are not granular
enough; we add fine-grained verbs that match ZLE/readline semantics
(`BackwardKillWord`, `KillLine`, `TransposeChars`, `UpHistory`,
`HistorySearchBackward`, `Yank`, `YankPop`, `ViChange`, …) and route
them through `InputBufferModel`. Many of these are small additions
because the buffer already supports the underlying mutations
(word-aware cursor motion, kill-ring) — they just lack public action
entry points.

A widget→`InputAction` map (`shell/widget_dispatch.rs`) is the bridge.
Honored widgets dispatch the matching `InputAction`. The widget enum
distinguishes:

- `SelfInsert` (no payload) — the dispatched key character is inserted
  literally at the cursor. This is the trivial `bindkey -e` /
  `bind self-insert` case, plus any binding that evaluates to a single
  printable keystroke.
- `Macro(String)` — the bound text is fed back through the input
  pipeline one keystroke at a time, exactly as if the user had typed
  each character. The injected stream goes through the same
  key-resolution chain as real input (PRODUCT #9): a newline therefore
  triggers `accept-line` and submits the command, `^A` triggers
  `beginning-of-line`, and so on. This is the path for zsh
  `bindkey -s '^X' 'echo hi\n'`, readline `"\C-x": "echo hi\n"`, and
  fish string-bind macros. Macro re-injection is bounded (a small
  per-macro-character limit prevents bind-cycle infinite loops; the
  input pipeline rejects further macro expansion once the limit is
  reached and emits a diagnostic).
- `Action(InputAction)` — every other Category A built-in widget. The
  dispatcher fires the mapped `InputAction` directly.
- `External(widget_name: String)` — Category C external shell-function
  widgets (atuin, fzf, custom user widgets, plugin widgets,
  `edit-command-line`, etc.). The dispatcher routes to the
  pass-through path described in §6 below.
- `Unsupported(name)` — Category A widgets without a Warp equivalent
  only. Returns a sentinel that tells the matcher to fall through
  (PRODUCT #11, #16). Category C widgets never land here.

**Detection of External widgets** (the parser's job):

- **zsh:** the widget name in `bindkey -L` output is matched against
  the documented ZLE built-in widget list. Anything not in that list
  is treated as `External(widget_name)`. The list is colocated in
  `crates/warp_terminal/src/shell/bindings.rs` so it serves both the
  parser and the redaction allowlist.
- **bash:** `bind -p` lists readline-function bindings (Category A
  candidates); `bind -X` lists `bind -x` shell-command bindings —
  those are always `External("__bind_x_<key>")` (the bound shell
  command is what runs).
- **fish:** `bind` output's right-hand side is either a built-in fish
  input function (well-known list) or a fish function name —
  user-defined or plugin-defined. Anything not in the built-in list
  is `External(function_name)`.

### 4. Keymap matcher integration

`warpui_core` is a UI-layer crate and must not learn about shells,
tabs, or PTYs. Shell bindings are therefore normalized into ordinary
`Binding` instances at the terminal layer before they are handed to
the matcher; the matcher itself stays unchanged at the type level.

The current `ContextPredicate` only takes `&'static str`
identifiers/values (`crates/warpui_core/src/keymap/context.rs:10-17`),
so a `TabIs(tab_id: u64)` predicate cannot be expressed without
extending it — and we don't want to. Instead, tab scoping happens at
the storage tier, not inside the predicate. The new API is:

```rust
// crates/warpui_core/src/keymap.rs
pub struct ScopeKey { pub category: &'static str, pub id: u64 }

impl Keymap {
    pub fn set_contextual(&mut self, scope: ScopeKey, bindings: Vec<Binding>);
    pub fn clear_contextual(&mut self, scope: ScopeKey);
    pub fn set_active_scopes(&mut self, scopes: SmallVec<[ScopeKey; 4]>);
}
```

Internally `Keymap` stores `contextual: HashMap<ScopeKey, Vec<Binding>>`
plus `active_scopes: SmallVec<[ScopeKey; 4]>`. The matcher iterates
only over bindings in `active_scopes`, in priority order, alongside
the existing fixed/editable tiers. `Binding`s themselves keep using
the existing `ContextPredicate` for any further conditional matching
within a scope (e.g. "only when the input editor is focused"); they
don't need to know about tabs.

The terminal layer (`app/src/terminal/keymap_bridge.rs`, new) owns
shell-binding state per tab and writes through this API:

1. On `ShellBindingsUpdated(tab_id, bindings)`, translates each
   `ShellBinding`'s widget into an `InputAction` (or `Macro` injection
   / `Unsupported` sentinel) via `shell/widget_dispatch.rs`, then
   builds `Vec<Binding>` with `BindingOrigin::Contextual` tags and a
   regular `ContextPredicate` matching "input editor focused".
2. Calls `keymap.set_contextual(ScopeKey { category: "shell", id:
   tab_id }, bindings)`.
3. On tab focus change, calls `keymap.set_active_scopes(...)` with
   the focused tab's shell scope (plus any other always-active
   scopes).
4. On tab close, calls `keymap.clear_contextual(...)`.

`BindingOrigin` is a generic, shell-free enum that lives in
`warpui_core` alongside the existing `Binding` struct, because the
matcher needs to know about it to enforce precedence:

```rust
// crates/warpui_core/src/keymap.rs
pub enum BindingOrigin {
    Fixed,        // built-in defaults
    Editable,     // user customizations from settings
    Contextual,   // installed via set_contextual; tier slotted
                  // between Editable and Fixed in the matcher
}

pub struct Binding {
    // existing fields …
    pub origin: BindingOrigin,
}
```

`Contextual` is the generic name for the tier — `warpui_core` does not
know what populates it (shells, plugins, anything that wants
short-lived scope-keyed bindings). The terminal/app layer is the only
caller that uses it for shell bindings today; future callers
(e.g. plugin-provided bindings) reuse the same tier without any
further `warpui_core` changes.

The matcher walks candidates in
Editable → Contextual → Fixed order within each active scope's set,
which is the exact PRODUCT #14 ordering (reserved infrastructure keys
sit above all of these and are handled at the terminal layer before
the matcher sees the keystroke at all). The terminal layer's
shell-specific types (`ShellBinding`, `ShellWidget`, the
shell-vocabulary widget allowlist, the `keymap_bridge`) stay confined
to the terminal/app crate; they are translated into core
`Binding { origin: Contextual, … }` instances before being handed to
`Keymap::set_contextual`.

Effective resolution order for a keystroke in the active tab
(PRODUCT #14, enforced by origin-tag ordering within
`active_scopes`):

1. Reserved infrastructure keys for the tab's shell.
2. Bindings tagged `BindingOrigin::Editable` (user Warp overrides)
   whose context predicate matches.
3. Bindings tagged `BindingOrigin::Contextual` from the active tab's
   contextual scope.
4. Bindings tagged `BindingOrigin::Fixed` (Warp defaults).

Multi-tab independence (PRODUCT #5, #17) falls out of scope-keyed
storage. The terminal layer maintains active-scope membership in
sync with focus.

**Multi-key prefix handling (PRODUCT #8) requires a matcher API
change.** The current `Matcher::match_keystrokes` returns `None` and
clears its pending state on a mismatch
(`crates/warpui_core/src/keymap/matcher.rs:258`+); buffered prefix
keys are dropped silently. PRODUCT #8 demands the readline / ZLE
behavior of replaying buffered keys when a multi-key sequence is
abandoned by a non-matching keystroke. Concrete change:

- The matcher's per-call return type becomes:

  ```rust
  pub enum MatchOutcome<'a> {
      Match(&'a Binding),
      Pending,                       // prefix matched, awaiting more
      AbandonedPrefix(SmallVec<[Keystroke; 4]>, Keystroke),
                                     // prefix did not extend; replay
                                     // these keys then handle the
                                     // current key normally
  }
  ```
- The dispatcher handles `AbandonedPrefix` by feeding each replayed
  keystroke through the matcher with pending state cleared, then
  feeding the current keystroke last. Any of those replayed keys may
  themselves trigger a (single-key) binding; the new prefix
  accumulator is empty until something matches a multi-key prefix
  again.
- The change is internal to `warpui_core`. Callers that don't care
  about the new variant (every existing keymap) use a thin helper
  `match_or_replay()` that flattens `AbandonedPrefix` back into the
  old "single key, no match, drop pending" semantics — preserving
  current behavior for surfaces that don't want replay.

**Ambiguity timeout (PRODUCT #8).** When the accumulated keys both
match a complete binding *and* prefix a longer one, the matcher
returns `Pending` and the dispatcher arms a 500 ms timer. If the
timer fires before another key arrives, the dispatcher tells the
matcher to commit the shorter binding (`Matcher::commit_pending()`,
new) and dispatches that action. If a key arrives first that
extends the prefix, the timer is canceled and matching continues. If
a key arrives that does *not* extend the prefix, the matcher returns
`AbandonedPrefix` and the dispatcher follows the replay path (above).
The timer lives on the dispatcher side so the matcher itself stays
synchronous; this also keeps the timeout out of the matcher's pure
match logic. Pure-prefix accumulation (no complete-binding ambiguity)
arms no timer — Warp waits indefinitely for the next key, matching
readline / ZLE.

**Focus-loss abandonment (PRODUCT #8).** Window blur, modal-overlay
open, and tab switch each call `Matcher::abandon_pending()`, which
returns the buffered keys for the dispatcher to drop on the floor
(no replay, since the input editor no longer has focus to receive
them).

### 5. Settings, feature flag, debug surface

- New boolean setting in `app/src/terminal/keys_settings.rs` via
  `define_settings_group!`: `honor_shell_bindkeys` (default `true`)
  with `toml_path: "terminal.input.honor_shell_bindkeys"`. The matcher
  short-circuits the `BindingOrigin::Contextual` tier when this is off (PRODUCT
  #24). Because re-queries are shell-side (bootstrap + `precmd`
  driven), turning the toggle back on does not actively re-query — it
  resumes matching against the most recent table the bootstrap emitted,
  and any change since then will arrive on the next `precmd`. PRODUCT
  #24 is updated to reflect this (toggling off restores defaults
  immediately; toggling on resumes from the cached table and picks up
  changes on the next prompt).
- New `FeatureFlag::HonorShellBindkeys` in
  `crates/warp_features/src/lib.rs` so we can stage rollout
  (default off → dogfood → preview → stable). Resolves PRODUCT
  open-question #23.
- Read-only debug view (PRODUCT #25): a small panel under the
  Keybindings settings section that lists the active tab's
  `ShellBindings` as `key → widget (status)` rows. Status is derived
  by walking the matcher precedence chain. No new persistence.
- Telemetry events in
  `app/src/server/telemetry/events.rs`:
  - `HonorShellBindkeysToggled { enabled: bool }`
  - `ShellBindkeysQueryFailed { shell_type, reason }`
  - `UnsupportedShellBindkeyWidget { shell_type, widget_name }` — the
    `widget_name` field is sent verbatim only when it appears in the
    shell-vocabulary allowlist (the well-known ZLE/readline/fish
    widget names enumerated in PRODUCT #10). Names outside the
    allowlist (user-defined functions, plugin-private widgets) are
    redacted to the literal string `user-defined`. Key contents and
    binding bodies are never sent. The allowlist lives in
    `crates/warp_terminal/src/shell/bindings.rs` so it is the same
    source of truth used by the parser.
  - `ShellBindkeysApplied { shell_type, honored_count,
    unsupported_count, external_count }` once per tab on first apply.
  - `ShellWidgetPassthroughInvoked { shell_type, widget_name,
    completed_ok }` per pass-through invocation. `widget_name` is
    redacted via the same allowlist policy.

### 6. External widget pass-through dispatch

Pass-through is the v1 mechanism for Category C widgets — atuin,
fzf, zsh-vi-mode, `edit-command-line`, and any other user-defined
shell-function widget — and is also the foundation for inline-plugin
support (§7).

#### Architectural premise

A submitted shell command runs *outside* an active ZLE / readline
context, so `BUFFER`, `CURSOR`, and `zle <widget>` are invalid in
that context. Earlier drafts proposed a `warp_invoke_widget` helper
invoked as a shell command; that approach does not work on zsh and
is partially broken on bash for the same reason. v1 abandons that
shape entirely.

**v1 model: native-keystroke dispatch.** The shell's line editor is
the source of truth for `$BUFFER` (zsh), `$READLINE_LINE` (bash),
and `commandline` (fish). Warp drives it by injecting bytes into
the PTY at the prompt and consumes its state via a bootstrap-
installed hook that emits the buffer + plugin overlays back over
DCS after every keystroke. Widgets dispatch natively from inside
ZLE / readline / fish-line-editor — exactly the context they were
written for.

Concretely:

- All printable keystrokes are written to the PTY via
  `Message::Input`. ZLE / readline / fish-line-editor receive them
  the same way they would in any other terminal and run their
  `self-insert` widget (including plugin-installed wrappers like
  `_zsh_autosuggest_self-insert`, `fast-syntax-highlight`, fish
  abbr-expansion).
- A bootstrap-installed `zle-line-pre-redraw` hook (zsh), per-key
  wrapper widget (bash, see #6.3), or `fish_postexec`-style hook
  (fish, see #6.3) emits a `WarpBufferState` DCS payload after
  each keystroke carrying the new buffer, cursor, vi-mode (if
  any), autosuggest text (if any), and syntax-highlight regions.
- Warp maintains a *mirror* of `$BUFFER` for its own features
  (autocomplete, AI correction, etc.). The mirror is updated from
  the DCS payload, never written-to outside of being a passive
  reflection of shell state. This keeps Warp's editor features
  working without making Warp the source of truth — important
  because shell plugins are allowed to mutate `$BUFFER`
  asynchronously (e.g. fish abbreviation expansion fires on
  space).
- When the matcher resolves a keystroke to
  `ShellWidget::External(name)` (Category C), Warp does nothing
  special at dispatch time — the keystroke is just injected like
  any other byte. ZLE / readline / fish-line-editor dispatches the
  bound widget from its own keymap, in its own valid context.
  Atuin opens its TUI, fzf opens its picker, etc. The existing
  alt-screen plumbing handles the TUI's render.
- When the widget completes (sets `$BUFFER` and returns), the
  `zle-line-pre-redraw` hook fires again and emits the new buffer
  via DCS. Warp updates its mirror and re-renders the input editor.

The widget name **never crosses the app→shell boundary as a
command argument**. The keystroke does. ZLE's keymap turns that
keystroke into a widget dispatch, in-shell. This resolves both the
critical ZLE-context concern and the security concern about
widget-name encoding (there's no widget name to encode).

#### 6.1 Per-keystroke flow (printable + Category A + Category C)

1. User presses key K with current Warp buffer `B`.
2. Matcher resolves K against the active keymap (§4). Match
   `ShellWidget::External(_)`, `Action(_)`, or `Insert(_)` —
   dispatch is the same shape for all three: inject the byte
   sequence for K into the PTY.
3. Shell's line editor processes K natively. `self-insert` wrappers
   fire; bound widget dispatches; plugins render their overlays.
4. Bootstrap hook fires after the keystroke and emits
   `WarpBufferState { buffer, cursor, vi_mode?, autosuggest?,
   highlights? }` over DCS.
5. Warp consumes the DCS, updates its mirror buffer, renders the
   input editor with Warp's block-UI chrome plus the plugin
   overlays from the report.

Latency per keystroke is one PTY round-trip plus one DCS parse —
the same shape as Warp's existing shell→app pings (prompt
detection, exit-code report). The §7 latency budget is the gate.

#### 6.2 Buffer-sync at session boundaries

When the user opens a new tab, the shell starts with `$BUFFER=""`
and Warp's editor is empty. They are in sync trivially.

When the user pastes a multi-character string into Warp's editor,
Warp writes the bytes into the PTY. Shell's bracketed-paste
handling collapses the `self-insert` wrapper bursts (autosuggest
intentionally does not re-fire mid-paste), so the DCS report fires
once at paste completion. Warp's mirror catches up.

When the user switches between tabs or focuses Warp from another
app, no resync is needed — `$BUFFER` is held in the shell's
memory across focus changes.

#### 6.3 Shell-specific bootstrap

**zsh** (`bundled/bootstrap/zsh.sh`):

```sh
_warp_emit_buffer_state() {
    local payload
    payload=$(_warp_encode_buffer_state)  # see #6.4
    printf '\eP+%s|buffer-state|%s\e\\' "$WARP_BOOTSTRAP_NONCE" "$payload"
}
# Install once at bootstrap, after user's zshrc has loaded so
# plugin-installed widgets are present.
zle -N zle-line-pre-redraw _warp_emit_buffer_state
```

The hook runs in proper ZLE context after each redraw (which
follows each keystroke and widget dispatch). It reads `$BUFFER`,
`$CURSOR`, `$KEYMAP` (for vi-mode), and any plugin-published
state (e.g. `$POSTDISPLAY` for zsh-autosuggestions, the
`region_highlight` array for zsh-syntax-highlighting).

**bash** (`bundled/bootstrap/bash_body.sh`): bash readline does not
expose a per-keystroke hook equivalent to `zle-line-pre-redraw`.
Two options, picked per binding category:

- For `bind -x` widgets (atuin, fzf): wrap the user's `bind -x`
  body in a function that emits `WarpBufferState` after the body
  returns. Implemented by post-processing `bind -X` output at
  bootstrap and re-binding each `bind -x` key to a Warp wrapper
  that calls the original body then emits.
- For everything else (native readline functions + `self-insert`):
  the only programmatic surface is `PROMPT_COMMAND` (fires after
  the line is accepted, not per-keystroke) and `bind -x` (fires
  for explicitly-bound keys, not all keystrokes). Vanilla bash
  has no per-keystroke hook. v1 ships with `bind -x` coverage —
  which is what atuin/fzf actually use — and accepts that
  inline-plugin support for vanilla bash is limited to what
  `bind -x` wrappers can publish. Users running `blesh` get full
  inline-plugin support because blesh provides per-keystroke
  hooks.

The bash gap is honest and documented; it does not block v1
because the motivating cases (atuin, fzf) all use `bind -x`.

**fish** (`bundled/bootstrap/fish.sh`): fish provides
`fish_postexec` (after command runs) and lets bindings publish
arbitrary events. The bootstrap wraps each user-bound function in
a small emitter and installs a `bind --erase` + `bind` rebind to
reach every key. Per-keystroke buffer state is read via
`commandline` and emitted with the same DCS shape.

#### 6.4 Security and encoding

The shell→app DCS direction is the only place plugin-influenced
data crosses a trust boundary. Rules:

- **Payload framing.** `WarpBufferState` payloads use the
  existing DCS envelope with `WARP_BOOTSTRAP_NONCE` (zsh/bash) or
  the fish tempfile-nonce equivalent. Missing/wrong nonce → drop
  silently (same path as every other shell→app DCS).
- **Payload encoding.** Buffer content and autosuggest text are
  hex-encoded by the shell-side emitter (`_warp_encode_buffer_state`
  in zsh, equivalents in bash/fish). The app-side parser decodes
  hex back into bytes and treats the result as opaque UTF-8 (with
  invalid-sequence replacement). No structured parsing of buffer
  bytes; they are display data only.
- **Numeric fields.** `cursor`, `vi_mode_id`, `highlight_start`,
  `highlight_end` are parsed as ASCII decimal with strict bounds
  validation (cursor ∈ [0, buffer.len()], etc.). Out-of-bounds →
  drop the report and emit a one-time diagnostic.
- **Widget-name fields.** A `last_dispatched_widget` label may
  appear in reports for telemetry. App-side validates against the
  set of widget names discovered by §1 (`bindkey -L` / `bind -p` /
  `bind` output). Unknown name → drop the label, keep the rest of
  the report. Widget-name labels are never used in command
  construction — telemetry only.
- **App→shell direction carries only bytes.** Warp writes
  keystroke bytes into the PTY via `Message::Input`. There is no
  shell-command construction with shell-reported data, so no
  encoding/escaping question exists at the app→shell boundary.
  This is the structural fix for the security concern raised
  against earlier drafts.

#### 6.5 Cancellation, timeout, accept-line, tab close

- **Widget cancel** (atuin Esc, fzf Ctrl-C): the widget returns,
  `$BUFFER` is whatever it was set to (typically unchanged), the
  hook emits the report, Warp's mirror reflects the unchanged
  buffer. No special handling.
- **Widget calls `accept-line`**: ZLE processes accept-line, the
  command runs as a block, the next prompt's `$BUFFER` is empty
  and the hook reports it. Warp's editor clears for the next
  prompt. Same flow as a normal Enter.
- **Widget hangs**: the user can press Ctrl-C to interrupt
  (Ctrl-C reaches the PTY normally because Warp injected the
  original bound keystroke, putting the shell in widget-active
  state where Ctrl-C is the natural interrupt). If Ctrl-C is
  itself remapped, the shell-level escape is `Ctrl-\` (SIGQUIT)
  which is unmappable. No 60-second timeout machinery is needed —
  the same recovery path as any hung TUI applies.
- **Tab close mid-widget**: standard tab teardown kills the PTY;
  no special handling needed.

#### 6.6 What this does NOT require

- No new shell-command helper crossing app→shell. Removed.
- No out-of-context mutation of `$BUFFER` / `$CURSOR`. The hook
  reads them inside a ZLE-active path.
- No tempfile or named-pipe side channel for buffer sync.
- No widget-name escaping at the app→shell boundary — there is no
  app→shell command construction at all.

### 7. Continuous inline-plugin rendering

PRODUCT #11.6 commits to honoring plugins that hook every keystroke
— zsh-autosuggestions, syntax highlighting, fish abbreviations,
zsh-vi-mode's per-mode cursor shapes. The v1 architecture is the
same per-keystroke injection + DCS-report model defined in §6.

#### 7.1 v1 architecture (committed)

- Every printable keystroke is injected into the PTY (§6.1).
- ZLE / readline / fish-line-editor runs `self-insert` plus all
  plugin-installed wrappers. Plugins update their visible state
  exactly as they do in a native terminal.
- The bootstrap `zle-line-pre-redraw` hook (zsh), `bind -x`
  wrappers (bash with blesh: full per-keystroke; vanilla bash:
  only at bound keys), or fish equivalent emits
  `WarpBufferState` carrying buffer, cursor, vi-mode, autosuggest
  text, and syntax-highlight regions over DCS.
- Warp renders its input editor from the DCS-reported state plus
  Warp's block-UI chrome. The mirror buffer drives Warp's own
  features (autocomplete suggestions, AI correction) but the
  shell's reported buffer is authoritative for display.

This is one mechanism for both Category C dispatch (§6) and
inline plugins. Earlier drafts described three candidates and
deferred the choice; v1 picks this and commits.

#### 7.2 Why this and not the alternatives

- **Not "Warp owns the buffer, queries plugins async"** (the
  previous Candidate C): plugin-aware glue is brittle (adapters
  per plugin) and doesn't generalize to plugins we don't know
  about. Per-keystroke injection has uniform coverage.
- **Not "shell renders the prompt area, Warp lifts the ANSI"**
  (the previous Candidate B): requires parsing arbitrary ANSI
  emitted by plugins into Warp's rendering primitives, which is
  open-ended. DCS-reported structured state (buffer + overlays
  as discrete fields) is bounded and parseable.
- **Not "Warp owns keystrokes locally and only sync at Category
  C"**: plugins don't fire because chars never reach ZLE between
  keystrokes. Loses inline-plugin support — violates PRODUCT
  #11.6.

#### 7.3 Latency budget and fallback

Per-keystroke round-trip cost: one `Message::Input` write, one
shell read + widget execution + hook emit, one DCS parse, one
render. On developer hardware Warp's existing shell→app DCS
round-trips (prompt detection, exit-code) measure in the 1–5 ms
range. Plugin work adds shell-side compute (autosuggest history
lookup, syntax-highlight tokenize) typically 1–10 ms depending on
plugin.

**Budget: p95 keystroke-to-render < 30 ms** on the slowest
realistic stack (zsh + oh-my-zsh + atuin init + fzf init +
zsh-autosuggestions + zsh-syntax-highlighting + powerlevel10k).
Measured by the integration test in the validation section.

**Fallback if budget is exceeded:** Warp-local keystroke handling
for printable characters, with batched sync into the shell only
at Category C dispatch boundaries and at Enter. This preserves
Category C (atuin/fzf still work) but loses inline plugins for
the stacks where latency is bad. The fallback is gated by a
config setting `shell_keybindings.injection_mode` ∈ `{full,
batched}`, defaulting to `full`. Users on slow setups can flip
to `batched` to recover typing latency at the cost of plugin
fidelity; PRODUCT #11.6 invariants degrade explicitly via a
diagnostic.

#### 7.4 What this is NOT

- Not "use PS1 mode for everything" — Warp's block UI is
  preserved; only the input area's buffer is shell-driven.
- Not "reimplement the plugins natively in Warp" — plugins run
  in-shell as their authors wrote them.

### Open questions carried from PRODUCT.md

- **#11 (Category A widgets without Warp equivalent)** — v1 marks
  them `Unsupported` and falls through to Warp's default. This open
  question is now scoped to Category A only (`redisplay`,
  `quoted-insert` edge cases, etc.). Category C external
  shell-function widgets — atuin, fzf, custom user widgets,
  `edit-command-line` — are honored via pass-through (§6) and never
  land here.
- **#13 (vi-mode signal)** — vi mode is tracked by an in-app state
  machine, not by polling the shell. Reading the shell's mode only at
  `precmd` would miss every transition that fires inside the input
  editor (Esc → command, `i` → insert, `v` → visual, etc.) because no
  prompt hook runs between those keystrokes. Concretely:

  - `active_keymap: KeymapMode` lives on each tab's `Shell` struct
    (see Proposed Changes #2).
  - **Initial state and resync** come from the shell. The bootstrap
    payload includes the current mode (zsh `$KEYMAP`, bash
    `bind -v | grep editing-mode`, fish `$fish_bind_mode`); each
    `Precmd` payload also includes the mode and is treated as
    authoritative — if it disagrees with the in-app state, the
    in-app state is corrected to the shell's value, since the shell
    just observed whichever sequence of widgets actually executed.
  - **Transitions between prompts** are driven by the dispatched
    widget. The widget dispatcher maintains a small transition table:
    `vi-cmd-mode` / Esc → `ViCommand`, `vi-insert` /
    `vi-add-next` / `vi-add-eol` / `vi-substitute` /
    `vi-change-whole-line` → `ViInsert`, `vi-replace` → `ViReplace`,
    `vi-visual` → `ViVisual`, `accept-line` → reset to shell-reported
    mode at next prompt. The dispatcher updates `active_keymap`
    synchronously *before* the next keystroke is matched, so the
    next keystroke resolves against the new keymap.
  - This is the only feasible model: any per-keystroke shell roundtrip
    would require an invisible-exec primitive (we don't have one) or
    block on the PTY (violates PRODUCT #26).
- **#22 (AI prompt input)** — v1: not honored. The matcher's tab-scoped
  `BindingOrigin::Contextual` tier only activates on tabs whose focus is the shell
  command input editor, not on the AI prompt input.
- **#22.5 (classifier interaction)** — engaged only when #22 is
  opted on. The agent input editor wraps the binding matcher and
  the inline-plugin renderer in a `ClassifierGate`:
  - `ClassifierGate` holds the last raw classifier label per
    keystroke and a hysteretic `EffectiveMode { Shell, NL,
    LockedShell, LockedNL }`. Effective mode transitions
    Shell↔NL only after N=4 consecutive characters in the new
    raw label, and only after a ~80 ms quiet window — short
    enough to feel responsive, long enough to suppress
    per-keystroke flicker.
  - **External-widget dispatch is gated *outside* the
    classifier.** The matcher resolves the key against
    Category C bindings *before* consulting `EffectiveMode`. If
    a Category C match is found, dispatch proceeds (§6) and the
    classifier is bypassed entirely for that keystroke. PRODUCT
    #22.5(a) is implemented as a "Category C bypass" in the
    matcher rather than as state in the gate.
  - **Inline-plugin renderer reads `EffectiveMode` only.** The
    §7 renderer activates only when `EffectiveMode ∈ { Shell,
    LockedShell }`. On transition to NL (or LockedNL), it
    schedules a single-frame clear of any previously painted
    suggestion / highlight regions — no fade. On transition to
    Shell, it requests a fresh render against the current
    buffer.
  - **Lock action.** A new command `agent-input.lock-mode` toggles
    `LockedShell` / `LockedNL` / Auto for the current buffer.
    Bind it to a default keystroke (TBD; suggest
    `Ctrl-Shift-L`) and surface a chip in the input editor.
    Lock state resets at the next agent turn boundary.
  - **Telemetry.** Emit a counter `agent_input.classifier.flip`
    tagged with `{raw → effective}` and `agent_input.bind_dispatched_in_nl`
    counting Category C dispatches that happened while
    `EffectiveMode == NL` (the bypass cases). The latter is the
    metric that tells us how often the classifier would have
    mis-suppressed a legitimate binding press; if it's nonzero
    and growing, the classifier needs retraining or the
    hysteresis needs tuning.
  - **Why a gate and not a flag in the matcher.** Keeping
    `ClassifierGate` as a wrapper means the same matcher and
    renderer code paths work in both the shell-command input
    (no gate) and the agent input (gate present). The gate is
    the single place where #22.5's hysteresis / debounce / lock
    rules live; matcher and renderer stay classifier-unaware.
- **#23 (rollout)** — gated by `FeatureFlag::HonorShellBindkeys` (above).

## Risks and mitigations

- **Bootstrap script size and shell start latency.** The query adds a
  burst of work at shell start. Mitigation: dump in a single
  invocation per keymap, drop output through DCS without invoking
  external binaries, and benchmark on the slowest of our supported
  shells. Budget: < 30 ms added to shell start; if a real shell blows
  this we move that keymap behind on-demand fetch.
- **Plugin / framework interactions** (oh-my-zsh, prezto, fzf widgets,
  zsh-vi-mode). These rebind heavily and often dynamically. The hash
  re-query in `Precmd` (#1) catches any rebind that's settled before
  a prompt redraws. Vi-mode plugins that swap keymaps reactively are
  tracked through the `KEYMAP` payload field.
- **Widget coverage gaps.** Many widgets have no Warp equivalent
  initially. The `Unsupported(name)` fallthrough plus telemetry on
  hit count tells us which to prioritize.
- **Privacy.** Telemetry never includes key contents or widget bodies.
  Widget names are sent verbatim only when in the shell-vocabulary
  allowlist; user-defined or otherwise unknown names are redacted to
  the bucket `user-defined` (see Proposed changes #5).
- **DCS spoofing.** Arbitrary process output containing a DCS sequence
  could otherwise rewrite local key handling. Mitigated by the per-tab
  nonce gate, size cap, and strict schema validation described in
  Proposed changes #1.
- **Bootstrap parsing fragility.** `bindkey -L`, `bind -p`, and fish
  `bind` outputs are stable but quoting differs. Each parser has a
  property-test fixture set covering edge cases (escapes, multi-byte,
  bound to nothing, named widgets).
- **Bash per-keystroke hook coverage.** Vanilla readline has no
  per-keystroke hook equivalent to `zle-line-pre-redraw`. v1
  emits `WarpBufferState` from `bind -x` wrapper bodies — which
  covers atuin, fzf, and every other user-bound `bind -x` widget
  — and at `PROMPT_COMMAND` time. Vanilla-bash users without
  blesh therefore get Category C dispatch (works correctly via
  native key injection) but no inline-plugin overlays between
  bound keystrokes. blesh users get full coverage because blesh
  installs its own per-keystroke hook surface. This is an honest
  v1 gap: documented in PRODUCT #11.6 failure-mode wording and
  the per-shell rollout notes.

## Testing and validation

Tests are organized to map to numbered PRODUCT invariants. Use
`rust-unit-tests` for new crate-level coverage and
`warp-integration-test` for end-to-end flows.

- **Bootstrap parsers** — unit tests in
  `crates/warp_terminal/src/shell/bindings.rs` per shell, covering
  fixtures generated from real `bindkey -L` / `bind -p` / `bind`
  output. Asserts widget normalization. Covers PRODUCT #2, #9, #10,
  multi-key sequences for #8.
- **Matcher precedence** — unit tests in
  `crates/warpui_core/src/keymap/matcher.rs` that assert resolution
  order across fixed / editable / shell tiers. Covers PRODUCT #14, #15.
- **Tab independence** — unit test that two `Shell` instances carry
  independent `bindings`; matching one tab's keystroke does not
  consult another tab's shell bindings. Covers PRODUCT #5, #17.
- **Lifecycle** — integration test (`crates/integration`) that boots a
  zsh shell with a known rc file declaring `bindkey '^X^E' kill-line`,
  starts a Warp tab, types `^X^E`, asserts the buffer was killed.
  Repeat for bash and fish with shell-appropriate equivalents. Covers
  PRODUCT #1, #2, #7.
- **Dynamic rebind** — integration test that types
  `bindkey '^X^E' beginning-of-line` at the prompt, presses Enter,
  then `^X^E` on the next prompt and asserts the new behavior. Covers
  PRODUCT #4.
- **Vi mode** — integration test that runs `bindkey -v`, switches to
  command mode, presses `gg`, asserts cursor at buffer start. Covers
  PRODUCT #13.
- **Category A unsupported fallthrough** — integration test binding
  a key to a Category A widget Warp does not implement (e.g.
  `redisplay`); assert Warp default fires on that key and a
  telemetry event is recorded. Covers PRODUCT #11, #16. (Note:
  user-defined shell-function widgets are tested separately under
  the pass-through tests below — they do not fall through.)
- **Conflict precedence with user Warp keybinding** — set a Warp
  keybinding for `^A`, also have shell `bindkey '^A' kill-whole-line`,
  assert Warp keybinding wins. Covers PRODUCT #14 #1.
- **Shell start failure** — integration test where the bootstrap
  errors mid-script: bindings are absent, default keymap applies, no
  crash. Covers PRODUCT #3, #28.
- **Pre-bootstrap keystroke** — type before the `Bootstrapped` payload
  arrives; assert the keystroke is handled with Warp defaults and not
  buffered. Covers PRODUCT #26.
- **Setting toggle** — flip `honor_shell_bindkeys` off mid-session;
  assert shell bindings stop applying without restart and Warp's
  default keymap takes over. Flip on; assert (a) the most recently
  cached binding table from each tab resumes immediately (no fresh
  query is issued from the toggle), and (b) the next `precmd` payload
  on each tab refreshes that table if anything changed. Covers PRODUCT
  #24.
- **External widget pass-through (atuin)** — integration test that
  boots zsh with atuin installed (`eval "$(atuin init zsh)"`) and a
  seeded history database, presses Ctrl-R from Warp's input editor
  (with "kub" in the buffer), waits for atuin's TUI, types a search
  refinement, presses Enter on a result, and asserts the selected
  command lands in Warp's input editor and the buffer is the
  expected text. Covers PRODUCT #11.5 happy path on zsh. Repeat for
  bash (with `eval "$(atuin init bash)"`) and fish.
- **External widget pass-through (fzf)** — same shape as the atuin
  test: install fzf, source `key-bindings.zsh`/`key-bindings.bash`/
  `key-bindings.fish`, press Ctrl-R, drive the TUI, assert the
  command is in Warp's editor on exit. Repeat per shell. Covers
  PRODUCT #11.5 with a different widget that uses a different TUI
  binary.
- **Pass-through cancel** — integration test pressing Ctrl-R for
  atuin then Esc (cancel without selecting). Asserts (a) Warp's
  input editor restored to the pre-invocation content, (b) no new
  block in scrollback, (c) cursor at the same position the user
  had pre-invocation. Covers PRODUCT #11.5 cancel path.
- **Pass-through with widget calling `accept-line`** —
  integration test where the widget submits the command directly
  (an atuin config with `enter_accept = true`). Asserts the command
  ran (block in scrollback), Warp's editor is empty, and the user
  is at a fresh prompt. Covers PRODUCT #11.5 accept-line path.
- **Pass-through Ctrl-C interrupt** — integration test that
  triggers a Category C widget (atuin), then sends Ctrl-C before
  the widget completes. Assert: the widget exits, the next DCS
  `WarpBufferState` carries the pre-invocation buffer (since the
  widget didn't mutate `$BUFFER`), Warp's editor reflects it.
  Covers PRODUCT #11.5 failure mode under the §6.5 model.
- **DCS payload encoding** — unit test feeding crafted
  `WarpBufferState` payloads through the parser: valid hex
  buffer + numeric cursor decode correctly; invalid hex →
  drop + diagnostic; cursor > buffer.len() → drop + diagnostic;
  missing/wrong nonce → drop silently; unknown
  `last_dispatched_widget` → strip label, keep rest. Covers §6.4.
- **Widget-name validation** — unit test asserting that
  `last_dispatched_widget` labels are matched against the
  discovered widget set before use in telemetry, and that no
  app→shell code path passes shell-reported widget names into
  any string interpolation, command argv, or eval context.
  Covers the security concern raised against earlier drafts.
- **External widget detection** — unit test that
  `crates/warp_terminal/src/shell/bindings.rs` correctly classifies
  zsh `bindkey '^R' atuin-search` as `External("atuin-search")`,
  bash `bind -X` output for atuin's `__atuin_history` as
  `External(...)`, fish `bind \cr __atuin_search` as
  `External("__atuin_search")`. Plus negative cases — built-ins
  stay `Action(...)`.
- **zsh-autosuggestions inline rendering** — integration test
  with zsh-autosuggestions installed and a seeded history. Type
  `kub` and assert (a) Warp's input editor shows `kub` in the
  active style and a dimmed completion (e.g. `ectl get pods`)
  after the cursor, (b) pressing Right arrow accepts the full
  suggestion and the buffer becomes the accepted command, (c)
  pressing Alt-F (when bound to word-accept) accepts only the next
  word. Covers PRODUCT #11.6 inline-suggestion + acceptance.
- **zsh-syntax-highlighting** — integration test with
  zsh-syntax-highlighting installed. Type `lsx` and assert that
  `lsx` is rendered in the "command not found" style (red by
  default). Type `ls -l` and assert `ls` is in the valid-command
  style and `-l` is in the option style. Type a quoted string and
  assert quote matching. Covers PRODUCT #11.6 syntax highlighting.
- **fish abbreviations** — integration test in fish with `abbr -a
  gco 'git checkout'` set. Type `gco` then space; assert the buffer
  becomes `git checkout `. Type `gco` then enter; assert the
  expanded command runs. Covers PRODUCT #11.6 fish abbr expansion.
- **zsh-vi-mode cursor shape** — integration test with zsh-vi-mode
  installed. Switch to command mode (Esc) and assert the cursor
  shape rendered in Warp's input editor matches zsh-vi-mode's
  configured command-mode shape (block by default). Switch to
  insert mode (i) and assert the configured insert shape (beam).
  Covers PRODUCT #11.6 vi-mode indicators.
- **Inline plugin latency budget** — performance test on
  representative hardware (developer laptop) measuring keystroke-
  to-render time for the slowest realistic stack (zsh + oh-my-zsh
  + atuin init + fzf init + zsh-autosuggestions +
  zsh-syntax-highlighting + powerlevel10k). Asserts p95 keystroke
  latency under §7.1's per-keystroke injection model stays under
  30 ms. Failing test forces the `injection_mode = batched`
  fallback (§7.3) for the affected configuration.
- **Inline plugin failure mode** — integration test that injects a
  malformed ANSI sequence into the plugin output stream (simulate
  a plugin emitting an unsupported escape). Assert: render
  degrades to plain text, no crash, one diagnostic emitted, prompt
  remains usable. Covers PRODUCT #11.6 failure mode.
- **Classifier flicker hysteresis** — unit test on
  `ClassifierGate` feeding a synthetic stream of raw labels
  (Shell, Shell, NL, Shell, Shell, Shell, Shell) and asserting
  that `EffectiveMode` stays Shell throughout (single-keystroke
  NL spike is suppressed). Companion test with sustained NL
  (NL, NL, NL, NL, NL) confirms the transition fires after the
  Nth consecutive label + quiet window. Covers PRODUCT #22.5(b).
- **Classifier-independent bound key** — integration test in
  agent input with #22 opted on, `Ctrl-R` bound to atuin, and the
  classifier rigged to return NL for the current buffer. Press
  `Ctrl-R`. Assert atuin opens regardless of classifier state.
  Covers PRODUCT #22.5(a).
- **Inline-plugin clear on NL transition** — integration test in
  agent input. Type a buffer that classifies as Shell, observe
  autosuggest dimmed text rendered. Append text that flips the
  hysteretic state to NL. Assert dimmed text disappears in a
  single frame (no fade, no partial paint). Covers PRODUCT
  #22.5(c).
- **Mode lock** — integration test that invokes
  `agent-input.lock-mode` while in NL state, then types a clearly-
  shell-ish buffer. Assert inline plugins activate (the lock
  forces Shell) and the lock chip is visible. Pressing the lock
  key again returns to Auto; advancing to the next agent turn
  resets the lock. Covers PRODUCT #22.5(d).
- **Manual** — run Warp against a developer's real zsh+oh-my-zsh +
  atuin + fzf + zsh-autosuggestions + zsh-syntax-highlighting +
  powerlevel10k, a real bash with `~/.inputrc` + `bind -x` widgets +
  atuin + fzf, and a real fish with `~/.config/fish/` bindings +
  atuin + fzf + abbreviations. Capture a short loom walkthrough
  showing each shell's bindings honored, including a real atuin
  search, a real fzf history fuzzy-find, inline autosuggestions
  appearing as the user types, syntax highlighting, and a fish
  abbreviation expanding on space.

## Follow-ups

- Honor remote-shell bindings over SSH (PRODUCT #18).
- Re-query on subshell transitions (PRODUCT #19).
- Optional opt-in: honor shell bindings in the AI prompt input
  (PRODUCT #22). When this lands, the `ClassifierGate` (PRODUCT
  #22.5, tech §"#22.5") ships at the same time — the two are
  inseparable; #22 without the gate ships the flicker bug.
- Extend to PowerShell, nushell, xonsh once the core lands.
