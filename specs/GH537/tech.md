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

**`schema_version` policy.** The bootstrap script is shipped from
the app to the PTY at runtime (`bundled/bootstrap/*.sh` are
embedded in the binary and written into the shell's init path on
session start), so the bootstrap version and the app's expected
schema are always pinned together — version skew should be
impossible in steady state. `schema_version` validation is
defense-in-depth, covering: stale bootstrap scripts persisted on
disk from a partial install, rc-file injection of a hand-crafted
`ShellBindings` DCS frame (caught upstream by the nonce check
but the version check is a second layer), and dev-branch
mismatches where a developer is running an old app against a new
bootstrap. Schema-version bumps follow the existing
`DProtoHook` convention: a new version is only introduced when
field semantics change incompatibly; additive fields use
`#[serde(default)]` and do not bump. Any future bump ships
alongside an app release; the new bootstrap is always co-shipped.

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
- `Action(InputAction)` — every other Category A built-in widget. In
  `batched` mode the dispatcher fires the mapped `InputAction`
  directly against `InputBufferModel` (the PRODUCT #10 "no shell
  roundtrip" fast path). In `full` mode the dispatcher injects the
  bound keystroke into the PTY so plugin wrappers around the
  underlying widget continue to see it; the corresponding mutation
  to Warp's mirror lands when the next `WarpBufferState` arrives.
  See §6.1 for the per-mode dispatch flow.
- `External(widget_name: String)` — Category C external shell-function
  widgets (atuin, fzf, custom user widgets, plugin widgets,
  `edit-command-line`, etc.). The dispatcher routes to the inject
  path described in §6 (with the buffer prefix-sync detour in
  `batched` mode, §6.1).
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

**Contextual prefix-index (`full`-mode prerequisite).** When the
matcher is in `full` mode (the input editor's terminal layer sets
this via a new `Matcher::set_inject_mode(InjectMode::Full)` API),
the matcher needs to distinguish three cases for an arriving key
K:

1. K commits or extends a Reserved/Editable/Fixed sequence — own
   the prefix locally.
2. K is single-key match against the Contextual tier (atuin's
   `Ctrl-R`, etc.) and no local tier extends it — commit to
   inject immediately, do not buffer.
3. K both extends a local-tier sequence *and* is a prefix of
   some Contextual sequence — buffer locally, but on
   `AbandonedPrefix` route the replay through the inject path
   (not back through the matcher), so ZLE's keymap takes over
   the rest of the sequence.

The "is K a Contextual prefix?" check requires a
`contextual_prefixes: HashSet<SmallVec<[Keystroke; 4]>>` on the
matcher, rebuilt every time `set_contextual()` is called. Each
Contextual `Binding`'s key sequence contributes every non-empty
prefix to the set. The matcher queries
`contextual_prefixes.contains(&buffered_keys)` at every
keystroke to decide between cases 2 and 3 above. In `batched`
mode the index is unused (Contextual bindings dispatch through
the regular `MatchOutcome` path) but is still maintained so a
mid-session mode flip (§7.3) does not require rebuilding it.

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
returns the buffered keys for the dispatcher to replay through
normal handling *while the editor still has focus*, then releases
focus. Any single-key bindings from the replayed keys fire; none
re-enter prefix accumulation until the replay finishes. This
matches PRODUCT #8's "no keystroke is ever silently dropped" rule
and the replay path called out for focus loss. On refocus,
accumulation starts fresh.

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

**v1 model: dispatch widgets where their context is valid.** The
unifying rule is that no widget — Category A, B, or C — is ever
invoked outside an active ZLE / readline / fish-line-editor
context. Bytes are the only thing that crosses the app→shell
boundary; the shell's own keymap and dispatch machinery turns
them into widget invocations from inside the context those
widgets were written for. Two operating modes implement this rule
differently based on shell capability — see §6.1 for the
per-keystroke flow and §7.3 for the per-shell defaults.

- **`full` mode** (zsh; bash with blesh): every keystroke is
  written to the PTY via `Message::Input`. ZLE / readline runs
  `self-insert` plus plugin wrappers (`_zsh_autosuggest_self-
  insert`, `fast-syntax-highlight`, fish abbr-expansion under
  any future fish per-keystroke hook). A bootstrap-installed
  per-keystroke hook — `add-zle-hook-widget zle-line-pre-redraw`
  on zsh, blesh's per-key surface on bash — emits a
  `WarpBufferState` DCS payload carrying the new buffer, cursor,
  vi-mode, autosuggest text, and syntax-highlight regions. Warp
  reconciles its mirror against the report (§6.1's reconciliation
  paragraph); for printable `SelfInsert` Warp speculatively
  renders the mirror locally to hide the round-trip.
- **`batched` mode** (vanilla bash; fish in v1): Warp owns the
  mirror locally during typing. Category A and B widgets
  dispatch natively in `InputBufferModel`; Category C widgets
  inject the bound keystroke after first syncing the mirror to
  the shell via the literal-paste path (§6.2.5: bracketed-paste
  markers around a nonced `__warp_paste_sentinel` prefix, so
  newlines and control bytes in the mirror don't dispatch
  widgets mid-sync). At widget exit and at Enter, a bootstrap-
  installed hook (bash's `bind -x` wrappers plus
  `_warp_bracketed_paste`; fish's wrapped `__fish_paste` and
  per-bound-key wrappers, plus `fish_postexec` / `fish_prompt`
  as coarser fallbacks) emits a `WarpBufferState` payload and
  Warp resyncs.
- For `External(name)` (Category C) the dispatch shape is the
  same in both modes: the keystroke is injected, ZLE / readline /
  fish-line-editor dispatches the bound widget from its own
  keymap in its own valid context, atuin opens its TUI, fzf
  opens its picker, the existing alt-screen plumbing handles the
  render. The only difference is whether the buffer pre-fill
  came implicitly from prior keystrokes (`full` mode) or from an
  explicit mirror-to-shell sync (`batched` mode).

The widget name **never crosses the app→shell boundary as a
command argument**. The keystroke does. ZLE's keymap turns that
keystroke into a widget dispatch, in-shell. This resolves both the
critical ZLE-context concern and the security concern about
widget-name encoding (there's no widget name to encode).

#### 6.1 Per-keystroke flow

Dispatch depends on the tab's `injection_mode` (§7.3), set at
bootstrap based on shell capability and inline-plugin detection.
Two modes — `full` and `batched` — differ in who owns the buffer
during typing.

**Step 0 — reserved-key bypass (both modes).** If K matches one of
the shell's reserved infrastructure keys (PRODUCT #14: zsh `^P` /
`\ei` / `\ep` / `\ew`; bash `\C-p` / `\ei` / `\ep` / `\ew`; fish
`\cP` / `\ep` / `\ew` / `\ei`), Warp handles it locally and
returns; the byte is never written to the PTY. This step runs
before the matcher and before any inject path.

**`full` mode** — inline plugins active. Defaults: zsh, bash with
blesh detected. The shell owns `$BUFFER`; Warp mirrors it.

1. Matcher walks the full PRODUCT #14 precedence ladder, but each
   tier dispatches differently in this mode:
   - **Reserved** match → Warp handles locally, no inject. Step 0
     already covered this; included here for completeness.
   - **Editable** match (user-customized Warp binding) → Warp
     dispatches locally, no inject.
   - **Contextual** match (shell binding *other than `SelfInsert`*
     — see below) → Warp injects the byte; ZLE / readline
     dispatches the bound widget. Plugin wrappers and the bound
     widget fire in shell context.
   - **Fixed** match (Warp default; the shell-reported binding for
     this key is `SelfInsert` so the shell asserts no intent) →
     Warp dispatches locally, no inject. PRODUCT #29 holds: Warp
     defaults that the user has not shadowed via shell binding or
     editable override continue to fire.
   - **Else** (printable, no Fixed match) → inject, ZLE
     self-inserts and plugin wrappers fire.

   The Contextual tier only contains shell bindings whose widget
   is non-`SelfInsert` (Category A / B / C). Keys for which
   `bindkey -L` / `bind -p` / fish `bind` reports `self-insert`
   are *not* recorded as Contextual matches — they fall through
   to Fixed (Warp default if any) and then to the inject-and-
   self-insert path. This is how a key the shell leaves at
   default `self-insert` can still trigger a Warp Fixed action
   (PRODUCT #14 tier 4) before defaulting to character insertion
   (tier 5).
2. For `SelfInsert` of a printable K (the Else case above), Warp
   speculatively appends K to its mirror and renders one frame
   immediately, hiding the PTY round-trip. Speculation operates
   on complete UTF-8 codepoints, not raw bytes — a multi-byte
   codepoint (e.g. `é` = `0xC3 0xA9`) is buffered at the input
   layer until the full codepoint is in hand and then speculated
   as one unit; the mirror never briefly contains a partial
   codepoint or a replacement character. Pastes that arrive as
   multiple printable characters in one input event speculate the
   whole burst at once. For `Action(_)` or `External(_)` injected
   to ZLE, no speculation: the pre-K buffer remains rendered
   until the DCS report arrives. Category A keystrokes go
   through the shell in this mode so plugin wrappers around
   `self-insert` and the kill-ring continue to see them;
   `$BUFFER` stays authoritative.
3. For tiers that inject (Contextual non-`SelfInsert` and the
   printable Else case), Warp writes the byte sequence for K
   into the PTY. The shell's line editor runs `self-insert` plus
   plugin wrappers, or dispatches the externally-bound widget in
   its own keymap.
4. The bootstrap hook fires after the keystroke and emits
   `WarpBufferState { buffer, cursor, vi_mode?, autosuggest?,
   highlights? }` over DCS.
5. Warp reconciles the mirror with the reported buffer in one
   frame (no fade). If the speculative render disagrees with the
   shell-reported state — a plugin wrapper rewrote `$BUFFER`,
   autosuggest accepted on a key that also self-inserts, fish
   abbr expanded the typed token — the corrected state is what
   the user sees; the speculative frame is overwritten before
   most users perceive it. Plugin overlays from the report
   (autosuggest dimmed text, syntax-highlight regions) render
   alongside the buffer.

**`batched` mode** — inline plugins inactive. Defaults: vanilla
bash, fish (see §6.3 fish for the structural reason). Warp owns
the mirror; the shell receives the buffer at sync boundaries.

1. Matcher runs the full precedence chain (§4): Reserved →
   Editable → Contextual → Fixed.
2. On `SelfInsert` or `Action(_)` match, Warp dispatches natively
   in `InputBufferModel` — no PTY round-trip, satisfying PRODUCT
   #10's "no shell roundtrip" guarantee. Warp's mirror is
   authoritative for the duration of the typed line.
3. On `External(_)` match, Warp first syncs `$BUFFER` to the
   shell via the literal-paste path described in §6.2.5, then
   injects the bound keystroke. ZLE / readline / fish-line-editor
   dispatches the widget with `$BUFFER` correctly pre-populated.
   On widget exit the post-dispatch hook emits a
   `WarpBufferState` payload and Warp resyncs the mirror to
   whatever the widget left behind.
4. On Enter, Warp uses the same literal-paste path (§6.2.5) to
   set `$BUFFER` to the mirror, then injects `\n`; the shell
   runs `accept-line` natively and the block model fires from
   the existing `Preexec` / `Precmd` hooks.
5. **Fish-specific space-trigger sync (PRODUCT #11.6
   abbreviations).** Fish's defining `abbr` feature expands a
   typed token into its full form when the user presses space.
   Native fish runs this inside its line editor when space
   arrives; in `batched` mode space would otherwise be a local
   `SelfInsert` and never reach fish. To preserve the PRODUCT
   #11.6 invariant on fish, the `batched`-mode dispatcher
   treats space (`U+0020`) as an additional sync boundary on
   fish tabs only. When the user presses space, Warp does
   *not* append it to the mirror locally; instead it syncs the
   pre-space mirror to `commandline` via the literal-paste
   path and then injects one literal space byte. Fish's
   abbreviation engine fires on that space, possibly rewriting
   `commandline`; the post-space `WarpBufferState` reports the
   (post-expansion, plus-space) buffer and Warp resyncs its
   mirror — which now contains both the space and any
   expansion fish performed. Cost: one round-trip per space
   keystroke on fish tabs only — measured at the same range
   as `full` mode's per-keystroke round-trips (1–5 ms on
   developer hardware), well under PRODUCT #29's perceptible-
   lag bar. zsh and bash do not need this carve-out (zsh runs
   in `full` mode where every keystroke already reaches the
   shell; vanilla bash doesn't have an abbr-on-space feature
   to honor).

**Prefix accumulation ownership (PRODUCT #8).** Each prefix is
owned by exactly one matcher; the two matchers never compete for
the same prefix.

- In both modes, multi-key bindings on the `Reserved`,
  `Editable`, or `Fixed` tier accumulate in Warp's matcher with
  `MatchOutcome::{Match, Pending, AbandonedPrefix}` (§4). While
  Warp accumulates, the buffered bytes are *not* injected — the
  shell does not see them until the sequence resolves.
- In `full` mode, multi-key bindings on the `Contextual` tier
  (shell bindings) accumulate inside ZLE / readline. The shell's
  own pure-prefix wait, 500 ms ambiguity timeout, and replay-on-
  abandon behavior is what the user sees once the bytes reach
  it. Warp's matcher in `full` mode does not buffer for
  Contextual-only prefixes — single-key Contextual matches
  inject immediately; for ambiguous cases where a key prefixes
  both a Warp local-tier sequence and a Contextual sequence,
  Warp's matcher accumulates locally and the AbandonedPrefix
  path forwards the buffered keys through the inject path so
  the shell's own matcher can take over.
- In `batched` mode, multi-key bindings on the `Contextual`
  tier accumulate in Warp's matcher (because shell bindings are
  dispatched natively); Warp's matcher implements PRODUCT #8
  rules for those sequences directly.

The single invariant: a key the user types either (a) commits a
Warp-side dispatch (Reserved/Editable/Fixed/Contextual-in-batched)
without injecting, or (b) reaches the shell as a literal byte,
where ZLE's keymap is the authority. Never both.

Latency per keystroke: one PTY round-trip plus one DCS parse in
`full` mode, masked by speculation for the common `SelfInsert`
case; zero round-trips in `batched` mode. The §7.3 latency
budget applies to `full` mode only.

#### 6.2 Buffer-sync at session boundaries

When the user opens a new tab, the shell starts with `$BUFFER=""`
and Warp's editor is empty. They are in sync trivially.

When the user pastes a multi-character string into Warp's editor
in `full` mode, Warp writes the bytes into the PTY wrapped in
the bracketed-paste markers the terminal already supports. The
shell's bracketed-paste handling collapses the `self-insert`
wrapper bursts (autosuggest intentionally does not re-fire mid-
paste), so the DCS report fires once at paste completion. Warp's
mirror catches up. In `batched` mode, paste lands directly in
Warp's mirror; the mirror syncs to the shell at the next sync
boundary (Category C dispatch or Enter), so paste is zero-
round-trip.

When the user switches between tabs or focuses Warp from another
app, no resync is needed — `$BUFFER` is held in the shell's
memory across focus changes (`full` mode), and Warp's mirror is
held in the app's memory (`batched` mode).

#### 6.2.5 Literal mirror-to-shell sync (`batched` mode)

In `batched` mode, Category C dispatch (§6.1 step 3), Enter
(§6.1 step 4), and fish's space-trigger sync (§6.1 step 5)
all require Warp to install its locally-held mirror into the
shell's `$BUFFER` before any bound keystroke fires. Injecting
the mirror byte-for-byte through `self-insert` is unsafe: the
mirror can contain newlines (would fire `accept-line`
mid-sync) and control characters bound to widgets (would
dispatch them instead of inserting).

The sync path therefore does **not** route mirror bytes through
the keymap. It uses bracketed paste:

- **Bracketed paste for buffer content.** Warp wraps the mirror
  bytes in DEC-mode bracketed-paste markers
  (`\e[200~` … `\e[201~`). zsh, bash (with
  `enable-bracketed-paste`, on by default in modern readline),
  and fish all treat the contents as literal text — no widget
  dispatch fires, newlines do not trigger `accept-line`, control
  characters insert as literal bytes. Plugins that hook
  `bracketed-paste-magic` (zsh) remain composable because the
  paste delimiters fire the paste-handler widget, not
  `self-insert`. The bootstrap installs a sentinel paste handler
  that suppresses its own emit during a Warp-driven paste so the
  post-paste `WarpBufferState` fires once (not once per byte).
  The handler distinguishes Warp-driven pastes from user-driven
  ones via an in-payload marker, *not* a separate signaling
  channel.

  **Sentinel format.** Warp emits
  `\e[200~<sentinel><bytes>\e[201~` for Warp-driven syncs. The
  sentinel is `__WARP_PASTE_<nonce>__` where `<nonce>` is the
  first 8 hex characters of `WARP_BOOTSTRAP_NONCE` (captured at
  bootstrap). The bootstrap stores the sentinel in a shell-local
  `__warp_paste_sentinel` constant and its length in
  `__warp_paste_sentinel_len`; handlers compare against these
  rather than hardcoding either, so a future nonce-length
  change doesn't require touching handler bodies. The per-
  session nonce closes the collision window — a non-bootstrap
  process cannot guess the nonce, the same trust boundary as
  §1's other DCS payloads.

  **Handler behavior.** Each shell's paste handler reads the
  full pasted content (all three shells deliver it as a single
  string), checks whether the leading bytes match
  `__warp_paste_sentinel` (the sentinel length is computed once
  at bootstrap and stored as `__warp_paste_sentinel_len`; all
  three handlers use that, so a future nonce-length change
  doesn't require touching the handler bodies). If yes: strip
  the sentinel, write the remainder to `$BUFFER` /
  `$READLINE_LINE` / `commandline` directly, emit one final
  `WarpBufferState`. If no (user paste from the OS clipboard,
  or any other source that doesn't carry the nonced sentinel):
  run the shell's standard paste path unchanged.

  The bootstrap asserts `__warp_paste_sentinel_len > 0` after
  computing the sentinel and refuses to install the paste
  handlers if the assertion fails — a zero-length sentinel
  would make every paste silently match the empty prefix and
  fall through to the user-paste branch, leaving Warp-driven
  pastes with their sentinel bytes still in the buffer. On
  assertion failure the bootstrap emits a diagnostic and the
  tab falls back to no-`batched`-mode-Cat-C dispatch (same
  path as the bash `enable-bracketed-paste off` case below).

  **Per-shell installation.**
  - **zsh.** Save the existing `bracketed-paste` widget under
    a new name and install a Warp wrapper that diffs `$BUFFER`
    around the delegation so it can locate the inserted slice
    regardless of where the cursor was pre-paste:
    ```sh
    zle -A bracketed-paste _warp_orig_bracketed_paste
    _warp_bracketed_paste() {
        local pre_cursor=$CURSOR
        zle _warp_orig_bracketed_paste
        # The original widget inserted the pasted content at
        # pre_cursor and advanced CURSOR past it. The inserted
        # slice is $BUFFER[pre_cursor .. CURSOR).
        local inserted=${BUFFER:$pre_cursor:$((CURSOR - pre_cursor))}
        if [[ ${inserted:0:$__warp_paste_sentinel_len} == "$__warp_paste_sentinel" ]]; then
            # Strip the sentinel from the inserted slice and
            # rewrite $BUFFER. Move CURSOR back by sentinel_len
            # so it lands at end of the (stripped) insert.
            BUFFER=${BUFFER:0:$pre_cursor}${inserted:$__warp_paste_sentinel_len}${BUFFER:$CURSOR}
            CURSOR=$((CURSOR - __warp_paste_sentinel_len))
            _warp_emit_buffer_state
        fi
    }
    zle -N bracketed-paste _warp_bracketed_paste
    ```
    The pre/post-cursor diff is the standard zsh idiom for
    "what did the inner widget insert"; without it the
    sentinel-strip would only be correct when `$BUFFER` was
    empty pre-paste.

    The bootstrap loads after user zshrc by design (§1), so
    Warp's wrapper sits outermost in any pre-existing widget
    chain. Plugin-installed wrappers (`bracketed-paste-magic`,
    etc.) run as part of `_warp_orig_bracketed_paste` when
    Warp delegates to the saved original.
  - **bash.** readline has no separate paste-stage filter, so
    the bootstrap binds the bracketed-paste start sentinel to
    a `bind -x` handler that reads byte-by-byte until the
    5-byte end sentinel arrives:
    ```sh
    _warp_bracketed_paste() {
        # Accumulate bytes until the matching `\e[201~`
        # terminator arrives. `read -N 1` reads one byte at a
        # time without interpreting delimiters; the 5-byte
        # terminator is matched against a sliding window of
        # the last 5 accumulated bytes. `read -d` is *not*
        # used: it takes a single-character delimiter and
        # would terminate at the first `\e` inside the
        # pasted content.
        local content='' byte
        while IFS= read -r -N 1 byte; do
            content+=$byte
            [[ ${content: -5} == $'\e[201~' ]] && break
        done
        content=${content%$'\e[201~'}
        if [[ ${content:0:$__warp_paste_sentinel_len} == "$__warp_paste_sentinel" ]]; then
            # Warp-sync: the mirror is the complete buffer
            # state, so land the cursor at end. READLINE_LINE
            # is empty in batched-mode sync (Warp owns typing),
            # so += is effectively assign.
            READLINE_LINE+=${content:$__warp_paste_sentinel_len}
            READLINE_POINT=${#READLINE_LINE}
            _warp_emit_buffer_state
        else
            # User paste from OS clipboard: insert at cursor
            # and advance past the inserted content, matching
            # default bash bracketed-paste behavior.
            READLINE_LINE="${READLINE_LINE:0:$READLINE_POINT}$content${READLINE_LINE:$READLINE_POINT}"
            READLINE_POINT=$((READLINE_POINT + ${#content}))
        fi
    }
    bind -x '"\e[200~": _warp_bracketed_paste'
    ```
    The boundary-handling concerns from the OSC discussion
    above don't apply here because the handler runs inside a
    paste-mode sequence (PTY input is a contiguous burst
    framed by `\e[200~` / `\e[201~`), not during free typing
    where ambiguous-prefix timeouts would matter.
  - **fish.** Wrap fish's internal bracketed-paste path so
    the handler runs *after* fish has accumulated the pasted
    content (binding `\e[200~` directly would fire the
    handler at paste-start, before any content was
    accumulated). Recent fish exposes `__fish_paste` as the
    accumulation entry point:
    ```fish
    functions --copy __fish_paste _warp_orig_fish_paste
    function __fish_paste
        # The pasted content arrives as $argv[1] (fish's
        # standard paste calling convention). Strip the Warp
        # sentinel if present and `--replace` $commandline
        # wholesale (Warp's mirror is the complete buffer
        # state in `batched`-mode sync); otherwise delegate
        # to the saved original so user pastes from the OS
        # clipboard behave as default fish (insert at cursor).
        set -l content $argv[1]
        if string match -q "$__warp_paste_sentinel*" -- $content
            commandline --replace -- (string sub --start (math $__warp_paste_sentinel_len + 1) -- $content)
            _warp_emit_buffer_state
        else
            _warp_orig_fish_paste $content
        end
    end
    ```
    `functions --copy` preserves the original definition
    composably; later user code that further wraps
    `__fish_paste` still calls Warp's wrapper, which calls
    the saved original. The bootstrap probes for both
    `__fish_paste` and any documented successor name a future
    fish release introduces, binding whichever is present and
    emitting a diagnostic when neither is available.

  This approach avoids an app→shell signaling primitive that
  the shells don't natively support (binding OSC prefixes via
  `bindkey` / `bind -x` for free-typing-time use is fragile
  under PTY-read boundaries and prefix-timeout rules);
  encoding the marker inside the paste content sidesteps that
  entirely.
- **Bracketed-paste capability is a v1 requirement on bash.**
  Warp can emit the `\e[200~` / `\e[201~` markers regardless of
  the readline setting, but the reason we need
  `enable-bracketed-paste on` is what happens to the bytes
  *between* the markers: with the setting off, readline
  dispatches each byte through the normal keymap, so a newline
  in the pasted content fires `accept-line` mid-sync and a
  control character fires its bound widget. The setting being
  on switches readline into a paste-mode read that the `bind
  -x` handler above captures contiguously. The bash bootstrap
  detects the setting at startup by parsing `bind -v` output
  for `set enable-bracketed-paste on` and re-detects on every
  `precmd` re-snapshot (the user can flip the setting mid-
  session via `bind 'set enable-bracketed-paste off'`; the
  same `precmd` that re-snapshots `bind -X` for Category C
  also re-reads `bind -v` for this flag). When the setting is
  off (user has it disabled in `~/.inputrc` or in an older
  readline, or flips it mid-session), the bootstrap emits a
  one-time-per-state-change diagnostic *and a user-visible
  toast on the affected tab* — "Category C bindings disabled
  on this tab — bracketed-paste was turned off; re-enable with
  `bind 'set enable-bracketed-paste on'`". Warp suppresses
  Category C dispatch from the block-mode editor on that tab —
  the binding still parses and appears in the debug view as
  `unsupported (bracketed-paste disabled)`, but pressing the
  bound key falls through to Warp's default for that key
  (PRODUCT #11/#16 fallthrough applies). When the setting
  flips back on, a symmetric toast ("Category C bindings
  re-enabled — bracketed-paste is on again") fires and Cat C
  dispatch resumes from the next keystroke. zsh and fish
  always have bracketed-paste available in supported versions
  and need no detection. Lifting the bash requirement is a
  follow-up gated on an explicit per-byte literal-insert
  primitive landing in readline.

**Cursor position is end-of-buffer (v1).** After the paste the
cursor sits at end-of-buffer. Warp does not reposition it in v1
— positioning via `\e[D` would have to navigate the user's
active keymap (different bindings under emacs vs vi-cmd vs
vi-insert; user-rebinds of `backward-char` further muddy it)
and the timing of `\e` in vi command mode interacts with
mode-switch detection. Category C widgets that the v1
motivating cases use (atuin, fzf, `edit-command-line`) care
about buffer content, not cursor position — atuin opens a TUI
on the current buffer, fzf does the same — so end-of-buffer is
correct for the cases this PR targets. The mirror's recorded
cursor is restored to Warp's editor after the widget exits, so
the user perceives the position they had pre-dispatch.
Tracked as follow-up for a literal cursor-set primitive once
one exists.

The literal-paste path is used in `batched` mode only —
in `full` mode the shell already owns `$BUFFER` continuously
from per-keystroke injection, no resync needed.

#### 6.3 Shell-specific bootstrap

**zsh** (`bundled/bootstrap/zsh.sh`):

```sh
_warp_emit_buffer_state() {
    local payload
    payload=$(_warp_encode_buffer_state)  # see #6.4
    printf '\eP+%s|buffer-state|%s\e\\' "$WARP_BOOTSTRAP_NONCE" "$payload"
}
# Install once at bootstrap, after the user's zshrc has loaded so
# plugin-installed widgets are present. `add-zle-hook-widget`
# composes — it appends to the existing widget chain instead of
# replacing whatever zsh-autosuggestions / zsh-syntax-highlighting
# / prezto already installed at `zle-line-pre-redraw`. Using
# `zle -N` here would silently uninstall those plugins' hooks and
# break the inline-rendering support §7 is supposed to deliver.
autoload -Uz add-zle-hook-widget
add-zle-hook-widget zle-line-pre-redraw _warp_emit_buffer_state
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
  that calls the original body then emits. **Wrapper
  construction safety** (the body is user-controlled shell code):
  - The original body string and the key string are *parsed*
    from `bind -X` output, not concatenated into shell source.
    The parser handles `bind -X`'s documented quoting (outer
    double quotes around the key, `\C-`/`\M-`/`\e` escapes,
    backslash-escaped quotes) and emits a structured
    `{key_bytes: Vec<u8>, body: String}` pair.
  - **Pre-wrap validation.** Before any wrapping happens, the
    bootstrap iterates the parsed `bind -X` output (one
    `{key_bytes, body}` pair per discovered binding) and
    validates each body. The `continue` in the example below
    sits inside that outer loop: a rejected binding skips its
    wrapper installation and falls through to the next entry,
    leaving the original `bind -x` intact. The check has two
    parts:

    1. Length cap: `${#body} > 64 KiB` is rejected with a
       diagnostic. Bounds the bootstrap memory footprint and
       catches pathological generators *before* paying the
       `bash -n` fork+exec cost.
    2. Build the validation source via `printf -v`, then parse
       it with `bash -n`:
       ```bash
       printf -v __validate_src '_warp_validate_body() { %s\n}\n' "$body"
       if ! bash -n -c "$__validate_src" 2>/dev/null; then
           # reject this binding; original `bind -x` stays in
           # place; no Warp wrapper is installed; a diagnostic
           # logs the rejected `bind -x` (key only — body is
           # never logged).
           continue
       fi
       ```
       `printf -v` writes the constructed source into
       `__validate_src` literally — `$body` is *not* re-parsed
       as a shell command by the outer bash. The constructed
       source is then passed to `bash -n` as a single quoted
       argument. A non-zero exit means the body has unbalanced
       structure (extra `}`, dangling heredoc terminator,
       unclosed quoted string, etc.) that would let it escape
       the wrapper. `bash -n` is the entire structural-escape
       defense: any body that survives this check cannot
       break out of the surrounding `__warp_wrap_NNN() { … }`
       at execution time, because bash has already proven the
       braces and other block structure are balanced.

    **Validation cost.** Each `bash -n` spawn costs one
    fork+exec — measured at ~5-15 ms on typical hardware.
    With a heavy `bind -x` stack (atuin + fzf + plugin-manager
    wrappers, ~30 entries) this adds 150-450 ms to bootstrap.
    The validation loop runs sequentially after the binding
    dump, so it's measurable but doesn't block the first
    keystroke (PRODUCT #26: a late-arriving binding table just
    means earlier keystrokes use Warp defaults). If a real
    shell stack pushes the wall-clock overhead over a
    perceptible threshold, the implementation can amortize via
    a long-lived validator subshell that reads bodies on stdin
    and emits per-body verdicts on its stdout — folding the
    per-spawn cost into one spawn at bootstrap. The protocol
    is implementation-private (a small read-validate-respond
    loop running under `bash --noprofile --norc`); the
    spec leaves the wire format unspecified since it doesn't
    cross the trust boundary.

    Bodies that pass validation are *structurally guaranteed*
    to execute inside the wrapper's function body and nowhere
    else. Semantic concerns (a body that contains `return`,
    `exec`, or writes to `__warp_*` variables) are *not*
    caught — but they aren't wrapper-escape vectors either.
    A bare `return` returns from the wrapper itself, skipping
    `_warp_emit_post` (symptom: missing buffer-state report
    for that keystroke, not a security issue). `exec`
    replaces the bash process running the wrapper (almost
    never intended; the user already accepted this in their
    `bind -x`). Writes to `__warp_*` collide with the
    bootstrap's private namespace — a footgun, but the user
    chose those names. None of these escape the wrapper at
    the shell level, so v1 leaves them as documentation
    rather than rejection. (Adding a forbidden-token check
    would require either string-matching with false-positive
    risk inside string literals/comments, or a custom bash
    statement parser; not worth the complexity in v1.)

  - **Installation.** Validated bodies are installed via
    `source <(printf …)` (process substitution + `source`),
    not through string-concatenated `eval`. The body parses
    when `source` reads the function declaration, in exactly
    the same lexical context the original `bind -x`
    declaration parsed it. Conceptually:
    ```bash
    # $body is the literal parsed source from bind -X,
    # already validated above.
    # $wrapper_name is bootstrap-generated and contains no
    # user data.
    source <(printf '%s() {\n_warp_emit_pre\n%s\n_warp_emit_post\n}\n' \
        "$wrapper_name" "$body")

    # bind -x expects a single argument of the form
    # '"keyseq": shell-command' — quote-marks around the
    # keyseq are part of the format, not a shell quoting.
    # The argument is built explicitly:
    printf -v __bind_arg '"%s": %s' "$keyseq" "$wrapper_name"
    bind -x "$__bind_arg"
    ```
    The safety property is "validated body parses once at
    bootstrap in the same lexical context the original
    `bind -x` parsed it" — not "no `eval`". `source <(…)` is
    `eval`-equivalent by design, but combined with the
    pre-wrap validation above the input is structurally
    constrained so that the body cannot escape the wrapper
    function or execute outside the key-press call site. The
    wrapper does *not* re-evaluate the body at invoke time and
    does *not* re-interpolate it into any further string.
  - `$keyseq` is the readable form of the key sequence
    (`\C-r`, `\M-x`, etc.) as it appears in `bind -X` output
    after the parser has normalized escapes. It is composed of
    a documented set of tokens, not arbitrary user bytes, so
    no further escaping is required when building
    `__bind_arg`. Keys whose `bind -X` representation falls
    outside the documented token set are dropped with a
    diagnostic (the original `bind -x` remains in effect; only
    the Warp wrapper is skipped).
  - On re-snapshot at `precmd` (when the user has run another
    `bind -x` since the last snapshot), wrappers for keys that
    have disappeared are removed via `bind -r`; wrappers for
    keys whose body has changed are re-installed with a fresh
    wrapper name (the old wrapper is unset). No body string is
    ever mutated in-place.
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

**fish** (`bundled/bootstrap/fish.sh`): fish has no per-keystroke
hook equivalent to `zle-line-pre-redraw`. The available hooks
fire at coarser boundaries: `fish_postexec` after a command is
accepted and run, `fish_prompt` before each new prompt. Neither
fires per keystroke. Fish therefore defaults to `batched` mode
(§7.3) in v1:

- Category C dispatch is supported via per-bound-key wrapping
  exactly analogous to bash `bind -x`. The bootstrap post-
  processes the discovered fish bindings and rewrites each
  Category C bind so that a Warp emitter fires before and after
  the user's function. Wrapper construction follows the same
  safety model as bash:
  - The original binding's right-hand side is parsed from
    `bind` output into a structured `{key: String, body:
    String}` pair (fish bindings have a documented format —
    `bind \cR foo; bar` where everything after the key token
    is the command list).
  - **Pre-wrap validation** mirrors the bash safety model in
    §6.3 bash: before any wrapping, the bootstrap validates
    that `$body` is a self-contained fish statement that
    cannot escape a function-body context. The check runs in
    two steps:

    1. Length cap: `string length -- $body` greater than
       64 KiB rejects the binding with a diagnostic, before
       paying the `fish --no-execute` spawn cost (same
       cheap-check-first ordering as §6.3 bash).
    2. Pipe the validation source to `fish --no-execute` on
       stdin — fish reads it as a single contiguous script,
       with no command-substitution newline splitting:
       ```fish
       if not printf 'function _warp_validate_body\n%s\nend\n' $body \
           | fish --no-execute 2>/dev/null
           # reject this binding; original `bind` stays in
           # place; no Warp wrapper is installed; a diagnostic
           # logs the rejected `bind` (key only — body is
           # never logged).
           continue
       end
       ```

    A non-zero exit means the body has unbalanced structure
    (extra `end`, dangling block terminator, unclosed string,
    etc.) that would let it escape the wrapper. `fish
    --no-execute` is the entire structural-escape defense,
    same role as `bash -n` in §6.3 bash. Validated bodies are
    *structurally guaranteed* to execute inside the
    wrapper's function and nowhere else; semantic checks
    (forbidden tokens, namespace collisions) are not
    performed, for the same reasons §6.3 bash documents. The
    same long-lived-validator amortization path from §6.3
    bash applies (fish doesn't have `coproc` builtin but the
    equivalent via a backgrounded `fish --no-execute`
    reading from a FIFO is straightforward).
  - **Installation.** Wrapper installation feeds the
    function declaration to fish through `source (printf
    … | psub)` (the fish analogue of bash's process-
    substitution `source` from §6.3 bash). The body parses
    once when `source` reads it, in the same lexical context
    the original `bind` parsed it:
    ```fish
    # $body is the literal parsed source from `bind`,
    # already validated above.
    # $wrapper_name is bootstrap-generated and contains no
    # user data.
    source (printf 'function %s\n_warp_emit_pre\n%s\n_warp_emit_post\nend\n' \
        $wrapper_name $body | psub)

    # Rebind the key to call the wrapper. Key tokens
    # (`\cR`, `\eX`, etc.) come from the validated set
    # described below.
    bind $key $wrapper_name
    ```
    The safety property mirrors bash: validated body parses
    once at bootstrap in the same lexical context the
    original `bind` parsed it, and the validation step
    structurally guarantees the body can't escape the wrapper
    function. Fish has no `bind -x`-style direct primitive
    that takes a function-name-plus-keyseq; `source` of a
    process substitution is the chosen install path.
  - Keys from `bind` output are validated against fish's key
    syntax before being passed back to `bind`; any byte that
    doesn't round-trip through fish's parser is dropped with
    a diagnostic (the binding remains unwrapped, so the user's
    original behavior is preserved — Cat C dispatch from Warp
    just doesn't emit overlays for that key).
  - Wrapper function names (`__warp_wrap_NNN`) are bootstrap-
    generated and contain no user data.
  - On re-snapshot at the next `precmd`/event, removed
    bindings have their wrappers `functions --erase`d; changed
    bodies get a fresh wrapper name. No body string is ever
    mutated in-place.

  This composes with binds the user adds after bootstrap: each
  `precmd` re-snapshots `bind` output and re-wraps any new
  Category C entries before emitting the next `ShellBindings`
  payload. No `bind --erase`-everywhere is required.
- `WarpBufferState` payloads emit at the wrap boundaries above,
  at `fish_postexec`, and at `fish_prompt` — never per
  keystroke. Buffer content is read via `commandline`.
- fish abbreviations (`abbr`) are detected at the
  `WarpBufferState` sync that fires when the user presses space
  or enter — the expansion is visible after the sync, not per
  keystroke. PRODUCT #11.6's "fish abbreviations expand"
  invariant is met (the expanded text lands before the command
  is submitted); the visible-during-typing animation the
  invariant does not require is not delivered.
- Fish's built-in syntax highlighting and autosuggestions are
  rendered by fish itself when Warp is in PS1 mode; the
  block-UI input editor in v1 mirrors Warp's local buffer and
  does not show fish-native inline highlighting. Inline-plugin
  parity for fish is a follow-up tracked below, contingent on a
  per-keystroke fish hook landing upstream or on a curated
  bind-every-printable-key shim that the implementation can
  measure against the §7.3 latency budget.

#### 6.4 Security and encoding

The shell→app DCS direction is the only place plugin-influenced
data crosses a trust boundary. `WarpBufferState` payloads
arrive over the PTY just like every other shell-integration DCS
hook, which means any process the user runs can write the same
byte stream. The defenses below mirror the `ShellBindings`
payload protection in §1 — per-tab nonce gating, pre-decode
size cap, schema validation, post-decode bounds — so a
malicious or careless process can't spoof the overlay channel
to mutate Warp's editor.

**Validation order, single rule.** Each `WarpBufferState`
payload runs through three strictly ordered phases on the
receive side. Failures at phases 1-2 and *structural* failures
at phase 3 (out-of-bounds cursor, overlong buffer, malformed
numeric fields) discard the entire payload — there is no
"drop one entry and keep the rest" branch. The one exception
is the `last_dispatched_widget` telemetry label at phase 3,
which is strip-on-unknown rather than whole-payload reject,
since the label is not used in any structural decision.

1. **Pre-decode byte cap.** The DCS frame's hex-decoded byte
   length is checked against a 64 KiB total cap for
   `WarpBufferState` *before* JSON deserialization runs.
   Frames exceeding the cap are dropped at the framing layer
   and logged; no allocation for parsed structures happens.
   (`ShellBindings` carries a binding table and gets the
   256 KiB cap from §1; `WarpBufferState` carries one buffer
   plus overlay vectors and gets a tighter cap.)
2. **Schema decode (with nonce check).** JSON is decoded into
   the `WarpBufferState` struct via a single `serde_json` pass.
   The decoded struct's `nonce` field is compared against the
   tab's expected `WARP_BOOTSTRAP_NONCE` (zsh/bash) or fish
   tempfile-nonce value. Missing/mismatched nonce, field type
   mismatch, unknown `schema_version`, or any malformed sub-
   field discards the entire payload silently (same path as
   every other shell→app DCS spoof attempt — no oracle for an
   attacker). The nonce is part of the JSON payload (per §1's
   `ShellBindings` shape `{ shell, keymaps, active_keymap,
   schema_version, nonce }`) rather than a DCS envelope
   header, so the check happens here rather than as a
   separate pre-decode phase.
3. **Post-decode bounds.** After successful decode + nonce
   verification:
   - `buffer` content (hex-decoded by the shell-side emitter
     `_warp_encode_buffer_state` in zsh, equivalents in
     bash/fish; app-side decodes hex back into bytes and
     treats the result as opaque UTF-8 with invalid-sequence
     replacement) is bounded to 32 KiB. Buffer bytes are
     display data only — no structured parsing.
   - Numeric fields (`cursor`, `vi_mode_id`,
     `highlight_start`, `highlight_end`) are parsed as ASCII
     decimal with strict bounds: `cursor ∈ [0, buffer.len()]`,
     each highlight range `[start, end]` satisfies
     `0 ≤ start ≤ end ≤ buffer.len()`, total highlight count
     ≤ 256.
   - `autosuggest` text bounded to 8 KiB; same opaque-UTF-8
     treatment as buffer.
   - A `last_dispatched_widget` label, if present, is
     validated against the widget-name set discovered by §1
     (`bindkey -L` / `bind -p` / `bind` output). Unknown name
     → strip the label, keep the rest of the report. Widget-
     name labels are never used in command construction —
     telemetry only. This is the one strip-rather-than-reject
     case the rule statement above carves out.
   - Any *structural* bound violation (buffer overlong,
     cursor out of range, highlight range invalid, autosuggest
     overlong, highlight-count overflow) discards the entire
     payload and emits a one-time-per-session diagnostic.

**Why these particular caps.** 32 KiB is two orders of
magnitude above the longest plausible interactive command line
(observed p99 ≈ 4 KiB for shell-tool corpora) and bounds the
work the renderer does. 8 KiB autosuggest covers any realistic
zsh-autosuggestions hit (the suggestion is at most one shell
history entry). 256 highlight regions covers
zsh-syntax-highlighting's worst case on a 32 KiB buffer plus
headroom.

**App→shell direction carries only bytes.** Warp writes
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
zsh-vi-mode's per-mode cursor shapes — on shells whose line editor
exposes a per-keystroke hook. The v1 architecture is the
per-keystroke injection + DCS-report model defined in §6.1 `full`
mode; capability for that mode is determined per shell at
bootstrap (§7.3).

#### 7.1 v1 architecture (committed)

The architecture below is what runs when `injection_mode = full`
on a tab. §7.3 lists which shells default to `full` in v1.

- Every printable keystroke is injected into the PTY (§6.1
  `full` mode).
- ZLE / readline runs `self-insert` plus all plugin-installed
  wrappers. Plugins update their visible state exactly as they
  do in a native terminal.
- The bootstrap `zle-line-pre-redraw` hook (zsh, installed via
  `add-zle-hook-widget` so plugin-installed hooks compose) or
  blesh's per-keystroke hook surface (bash) emits a
  `WarpBufferState` payload carrying buffer, cursor, vi-mode,
  autosuggest text, and syntax-highlight regions over DCS.
- Warp renders its input editor from the DCS-reported state
  plus Warp's block-UI chrome and the speculative-render frame
  for the in-flight keystroke (§6.1 reconciliation). The
  shell's reported buffer is authoritative for display.

Shells whose default is `batched` (vanilla bash, fish in v1)
do not run this path — they deliver Category C bindings
natively but cannot supply per-keystroke overlays. See §6.3
fish and §7.3 for the structural reasons.

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
Measured by the integration test in the validation section. The
budget applies to `full` mode only.

**Mode is per-shell, defaulted by structural capability — not a
universal `full` default.** `injection_mode ∈ {full, batched}` is
computed at bootstrap time and stored on the tab:

- **zsh** → `full`. `add-zle-hook-widget zle-line-pre-redraw`
  delivers per-keystroke `WarpBufferState`.
- **bash with blesh detected** → `full`. blesh installs the
  per-keystroke hook surface readline lacks.
- **vanilla bash (no blesh)** → `batched`. Readline has no
  per-keystroke hook (§6.3 bash). `full` cannot deliver PRODUCT
  #11.6 invariants on this shell at any latency; the user-facing
  setting is read-only in this case.
- **fish** → `batched` in v1 (§6.3 fish). Inline-plugin parity
  follow-up gates the eventual `full` default.

**User opt-down on capable shells.** On zsh or bash-with-blesh,
users on slow setups can flip the setting to `batched` to recover
typing latency at the cost of inline plugin fidelity. PRODUCT
#11.6 invariants degrade explicitly via a one-time-per-session
diagnostic at tab start that names the plugins affected.

**Structural-incapability diagnostic.** On vanilla bash and fish
(both default to `batched` for structural reasons), a one-time-
per-session diagnostic at tab start names the limitation:
"inline-plugin overlays are not available on this shell in v1;
Category C bindings (atuin, fzf, custom widgets) work normally."
The diagnostic is suppressed when the user has explicitly
chosen `batched` on a capable shell — the opt-down case above.

**Mid-session capability re-evaluation.** Structural capability
is fixed per shell with one exception: bash's capability flips
when the user installs or removes blesh mid-session. zsh always
has `add-zle-hook-widget` and fish always lacks a per-keystroke
hook in v1, so plugin installs on those shells (autosuggest,
syntax-highlighting, etc.) become inline plugins *within* the
existing mode rather than triggering a mode change.

The `precmd` re-snapshot (§1's hash-driven re-query) re-runs the
structural-capability test on every snapshot and emits the
current capability alongside the `ShellBindings` payload:

- On zsh and fish the result is constant and is informational
  only; the app does not flip the mode.
- On bash, when blesh is detected for the first time (the user
  ran `source ble.sh` since the last snapshot — detected via
  `[[ -n "${_ble_version:-}" ]]`, since blesh exports
  `_ble_version` on init and unsets it on unload), the
  capability flips from `batched`-only to both-available.
  Symmetrically, if blesh is unloaded mid-session
  (`_ble_version` is absent on a snapshot where it was present
  previously), capability falls back to `batched`-only. On the
  unload transition the bootstrap traverses the installed
  `__warp_wrap_NNN` wrappers and either re-installs them
  through the plain-readline `bind -x` path (Cat C keys, which
  have a plain-readline equivalent) or `unset -f`s them (the
  per-keystroke hooks installed only because blesh provided
  the surface — these have no plain-readline equivalent and
  must not stick around).

When the bash capability flip happens:

- If the user has *no* explicit `injection_mode` preference for
  the tab, the app flips `injection_mode` to track the new
  capability and emits a one-time diagnostic naming the change
  ("blesh detected — enabling inline-plugin rendering for this
  tab; §7.3 latency budget applies", or the symmetric falling-
  back-to-batched form).
- If the user has an explicit `batched` preference on record
  (the only preference that's meaningful pre-blesh on vanilla
  bash), the app does *not* auto-flip but emits a one-time
  "blesh detected — inline-plugin rendering is now available;
  clear the per-tab `injection_mode` setting to enable"
  affordance. This surfaces that the explicit choice is now
  non-trivial without overriding the user.
- If the user has an explicit `full` preference (only valid
  after blesh was first detected), and blesh later unloads,
  the app falls back to `batched` for that tab and emits a
  diagnostic; the explicit `full` preference is retained so
  if blesh is reloaded the user is auto-flipped back to it.

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
    Default binding `Ctrl-Alt-L` (rarely bound natively in
    shells, terminal emulators, or common IDE extensions; avoids
    the macOS/Linux `Ctrl-Shift-L` clash with terminal "clear
    selection" / line-select shortcuts). User-rebindable via
    the editable Warp keymap. Surfaces a chip in the input
    editor. Lock state resets at the next agent turn boundary.
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
- **Literal mirror sync — round-trip** — integration test in
  vanilla bash (`batched` mode). Place mirror = `"ls -la\nrm -f"`
  (contains a newline that would fire `accept-line` if synced
  byte-by-byte). Trigger Cat C dispatch. Assert `$READLINE_LINE`
  in the shell after sync equals the mirror exactly, no
  `accept-line` fired, and the post-sync `WarpBufferState`
  reports the same content. Covers §6.2.5 happy path on bash.
  Repeat for fish (`commandline --replace` path).
- **Literal mirror sync — sentinel collision** — unit test:
  craft a user paste that begins with literal `__WARP_PASTE_`
  but has a different (or absent) nonce. Feed through each
  shell's paste handler. Assert the sentinel check fails (the
  nonce doesn't match `__warp_paste_sentinel`) and the handler
  falls through to the user-paste branch — content lands at
  `$READLINE_POINT` / `commandline` cursor with no
  `WarpBufferState` emitted. Covers §6.2.5 nonce-vs-collision
  invariant.
- **bash bracketed-paste-disabled fallback** — integration test
  that starts a bash tab with `set enable-bracketed-paste off`
  in `~/.inputrc`. Assert: bootstrap emits the user-visible
  toast, debug view shows each Cat C binding as `unsupported
  (bracketed-paste disabled)`, pressing the bound key falls
  through to Warp's default. Then run `bind 'set enable-
  bracketed-paste on'` at the prompt; on the next `precmd`,
  assert the symmetric toast fires and Cat C dispatch resumes
  from the next keystroke. Covers §6.2.5 bash detection +
  toggle.
- **fish `__fish_paste` composition** — integration test in fish
  that installs a user wrapper around `__fish_paste` *after*
  Warp's bootstrap (simulating a plugin loaded at runtime).
  Assert: a Warp-driven sync still reaches Warp's wrapper (via
  the user's wrapper calling Warp's, which calls the saved
  original), and the buffer ends up correct. Covers §6.2.5
  fish composability.
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
  latency under §7.3's per-keystroke injection model stays under
  30 ms. Failing test forces the `injection_mode = batched`
  fallback (§7.3) for the affected configuration.
- **Inline plugin failure mode** — integration test that injects a
  malformed ANSI sequence into the plugin output stream (simulate
  a plugin emitting an unsupported escape). Assert: render
  degrades to plain text, no crash, one diagnostic emitted, prompt
  remains usable. Covers PRODUCT #11.6 failure mode.
### Follow-up validation (with #22 opt-in)

The tests in this subsection cover PRODUCT #22 (AI prompt
input opt-in for shell bindings) and PRODUCT #22.5 (classifier
interaction). Both are tracked as a follow-up (TECH §"#22") —
v1 of this PR does *not* ship the AI-prompt path, so these
tests are not in scope for the v1 implementation. They are
listed here so that when the #22 follow-up lands, the test
plan is ready and the `ClassifierGate` machinery has
deterministic coverage.

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
- Promote fish to `full` mode (§6.3 fish, §7.3). Two viable
  paths: (a) upstream a per-keystroke fish hook akin to
  `zle-line-pre-redraw` once one exists, (b) ship a curated
  bind-every-printable-key shim that wraps `self-insert` for
  the ASCII printable range plus common navigation keys, gated
  on the §7.3 latency budget. Path (b) is a v1.x candidate;
  path (a) is open-ended.
- Vanilla bash `full`-mode path conditional on blesh becoming
  installable on demand (the v1 dependency is "blesh detected
  at bootstrap"; an opt-in installer flow would let more users
  reach `full` without distro changes).
- Extend to PowerShell, nushell, xonsh once the core lands.
