# Support Elvish shell — Tech Spec

GitHub issue: https://github.com/warpdotdev/warp/issues/5730
Product spec: `specs/GH5730/product.md`

## Context

Warp models supported shells with a closed enum, `ShellType { Zsh, Bash, Fish, PowerShell }` in `crates/warp_terminal/src/shell/mod.rs:247`. Almost every shell-specific behavior is an exhaustive `match` on that enum (~40 files), so adding a variant makes the compiler enumerate every site that needs an Elvish arm.

The bootstrap flow for a local session:

1. `crates/warp_terminal/src/local_tty/shell.rs` builds the spawn argv per shell. Fish (line ~668) is launched with `--init-command '<init script>'`, which emits the `InitShell` DCS hook carrying the session id.
2. Warp answers `InitShell` by writing the full bootstrap script (`crates/warp_terminal/src/bootstrap.rs` `script_for_shell`, asset `app/assets/bundled/bootstrap/<shell>.sh`) to the PTY. For fish and PowerShell, `app/src/terminal/bootstrap.rs:54` `should_use_rc_file_bootstrap_method` makes the controller write the script to a temp file and `source` it instead (`app/src/terminal/writeable_pty/pty_controller.rs` `source_bootstrap_script`).
3. The bootstrap script installs hooks that emit DCS JSON messages: `Preexec`, `CommandFinished`, `Precmd`, `Bootstrapped`, `InputBuffer`, `Clear` (see `app/assets/bundled/bootstrap/fish.sh` for the reference implementation, ~750 lines).
4. `Bootstrapped` reports aliases, functions, builtins, env var names, history file and shell version; those are parsed by `ShellType::aliases`, `abbreviations`, etc.

Elvish (0.21.0 verified locally) exposes everything needed natively:

| Warp hook | Elvish mechanism |
|---|---|
| `InitShell` | `elvish -rc <file>`: the rc file runs before the first prompt |
| `Preexec` | `edit:after-readline` hooks, called with the submitted line |
| `CommandFinished` | `edit:after-command` hooks, called with a map `{src, duration, error}`; `error` is `$nil` on success, otherwise an exception whose `reason` is `external-cmd-error` with `exit-status` for external commands |
| `Precmd` | `edit:before-readline` hooks |
| prompt markers | wrap `$edit:prompt` / `$edit:rprompt` closures and print OSC 133 A/B around them |
| input buffer report | `$edit:current-command` + a binding in `$edit:insert:binding` |
| hex/DCS output | `print` + external `od`/`tr`, same as fish |

These were verified in a PTY: `after-readline` receives the line, `after-command` fires with `error` → `exit-status 4` for `sh -c 'exit 4'` and `$nil` for `echo`, `before-readline` fires per prompt, and wrapped prompts emit `ESC]133;A BEL … ESC]133;B BEL`.

User rc loading needs care: Elvish has no `source` that injects into the interactive namespace. `eval (slurp < $rc) &on-end={|ns| edit:add-vars (… $ns …)}` does it, and was verified to make `fn` definitions, `var`s and `edit:*` hook additions from the user rc visible at the prompt.

## Proposed changes

### 1. `ShellType::Elvish` (`crates/warp_terminal/src/shell/mod.rs`)

- Add the variant. Let the compiler drive the remaining arms.
- `from_name`: `elvish`, `-elvish`, `*/elvish`.
- `from_markdown_language_spec`: `"elvish" | "elv"`.
- `history_files`: empty vec (bbolt db is not a text file; see non-goals).
- `rc_file_paths`: `~/.config/elvish/rc.elv` (Elvish ≥0.17 default; `$XDG_CONFIG_HOME` is resolved on the shell side).
- `and_combiner`: `"; "` (Elvish aborts the chunk on the first exception, so `;` already short-circuits). `or_combiner` is documented as "run regardless" and returns `" ; "` for bash/zsh/pwsh; Elvish returns the same, but since a failing first command throws, the one caller that relies on it (`app/src/autoupdate/linux.rs:352` `PackageManager::update_command`) gets an Elvish arm that wraps the whole generated command in `sh -c '…'` instead of composing it natively.
- `aliases`: Elvish has no alias concept; return empty. Functions come through `function_names`.
- `abbreviations`: parse `$edit:abbr` / `$edit:small-word-abbr` emitted as `key\tvalue` lines by the bootstrap.
- `ShellFamily` mapping: `Posix` (path escaping and separators are POSIX-like; quoting differences are handled in `escape` below).
- `escape` / quoting (line ~1021): Elvish single-quoted strings escape `'` by doubling it (`''`), not `'\''`. Add an arm.
- `name()`: `"elvish"`. Anything that emits "exec -a"-style or POSIX-only syntax gets an explicit Elvish arm.
- `command_corrections::Shell` has no Elvish; map to `Shell::Fish` (closest semantics for "command not found" corrections) and note in a comment that Elvish-specific corrections are not supported.

### 2. Spawn (`crates/warp_terminal/src/local_tty/shell.rs`)

Launch as `exec '<path>' -rc '<init file>'`, where the init file is the one-line init script written to a temp file (the rc-file mechanism already exists for fish/pwsh). `local_command_executor.rs` gets `-norc` for non-interactive generator commands (parallel to fish's `--no-config`) and no login flag (Elvish has none).

### 3. Bootstrap assets (`app/assets/bundled/bootstrap/`)

- `elvish_init_shell.elv`: emits `InitShell` (hex-encoded JSON in DCS, same framing as `fish_init_shell.sh`), then sources the user rc via the `eval … &on-end` pattern above so user state lands in the interactive namespace before Warp's hooks are appended.
- `elvish.elv`: the bootstrap body, a port of `fish.sh` limited to what the product spec requires: `warp_send_json_message`, `Preexec` / `CommandFinished` / `Precmd` hooks, prompt wrapping with honor-PS1 toggle functions, `Bootstrapped` (functions via `keys $edit:` + user `fn`s from `edit:add-vars`, builtins via `keys $builtin:`, env var names via `env` builtin, `$version`), `InputBuffer` binding, `clear` override. Exit status is derived in the `after-command` hook: `$nil` → 0, `external-cmd-error` → `exit-status`, anything else → 1.
- Register both in `script_for_shell`, `init_shell_script_for_shell` and `raw_init_shell_script_for_shell` (`crates/warp_terminal/src/bootstrap.rs:54,84,178`). `load_and_escape_script` joins lines with `;`, which is valid Elvish, but the Elvish arm must use the `''` quote escape instead of `'\''`.

### 4. RC-file bootstrap (`app/src/terminal/bootstrap.rs:54`)

Add `shell_type == ShellType::Elvish` to `should_use_rc_file_bootstrap_method`. Add an Elvish arm in `PtyController::source_bootstrap_script` that writes ` eval (slurp < '<path>')` (leading space keeps it out of Elvish history, same trick as fish).

### 5. Remaining exhaustive matches

Driven by the compiler. Known sites and intended behavior:

- `app/src/terminal/available_shells.rs:126,699`: display name "Elvish", add `(ShellType::Elvish, "elvish")` to the non-Windows list only.
- `app/src/env_vars/mod.rs:52,186`: env var export as `set-env FOO bar` and inline prefix as `env FOO=bar cmd`.
- `app/src/context_chips/builtins.rs`: Elvish arms reuse the POSIX `SH_COMMAND` wrapped in `sh -c`, same as fish does today.
- `app/src/terminal/local_shell/mod.rs:112,272,302`: PATH capture via `print $E:PATH`, flags `-c` (no `-i -l`; `elvish -c` does not read `rc.elv`, so use `elvish -c 'eval (slurp < ~/.config/elvish/rc.elv); print …'`).
- `app/src/terminal/model/session.rs:805,1025`: env var names newline-separated; POSIX path separators.
- `app/src/terminal/warpify/mod.rs:32`: `None` (subshell warpify is a non-goal).
- `app/src/terminal/view.rs:746` shell-widget apply mode: `Replace`.
- `app/src/terminal/input.rs:8747`: Elvish supports newlines in commands, so it takes the non-fish path.
- `crates/warp_terminal/src/shell/mod.rs:167` input-report keybinding: `ESC i`, bound in the bootstrap via `set edit:insert:binding[Alt-i] = …`.
- `SSH` / subshell paths (`warpify`, `ssh` helper scripts): Elvish arms return "unsupported" so the existing fallback UI is shown.

### 6. Version gate

Checking after bootstrap is too late to fall back cleanly, so the gate runs where the startup shell is resolved: when building the `ShellStarter` for an Elvish path, run `elvish -version` once (cached per path, alongside the existing PATH-capture probe in `app/src/terminal/local_shell/mod.rs`). Below 0.17 the shell is treated like any other unsupported login shell and goes through `ShellStarterSource::Fallback` with `UnsupportedShell` telemetry (`app/src/terminal/local_tty/terminal_manager.rs:1045`), satisfying behavior #12.

## Testing and validation

Unit tests (next to existing `*_tests.rs`):

- `shell/mod_tests.rs`: `from_name` for `elvish`, `-elvish`, `/opt/homebrew/bin/elvish`; negative `/bin/elvish/foo`. `rc_file_paths` for Elvish. Elvish quote escaping (`it's` → `'it''s'`). Abbreviation parsing.
- `bootstrap_tests.rs`: `script_for_shell(Elvish)` loads, contains no `#include` leftovers; `init_shell_script_for_shell(Elvish)` substitutes the session id placeholder.
- `app/src/terminal/bootstrap.rs` tests: `should_use_rc_file_bootstrap_method(Elvish, Local)` is true.
- `env_vars` tests: Elvish export strings.

Integration (`crates/integration`, GUI framework, gated on `elvish` being on `PATH` in CI; add `elvish` to the macOS and Linux CI images):

- Bootstrap completes and reaches the input editor (behaviors 1–3).
- `fn greet { echo hi }` in a temp `rc.elv`, then `greet` produces a block with output `hi` (4, 5).
- `sh -c 'exit 4'` block has exit code 4; `fail boom` block has exit code 1 and shows the exception (5, 6).
- `cd /tmp` updates the block's pwd (5).
- Honor PS1 toggle swaps prompts without restart (7).
- `clear` empties the block list (8).

Manual:

- macOS and Linux, Elvish 0.21: run through behaviors 1–13 with a real `rc.elv` that sets a custom prompt, uses `use` on a module, and defines `edit:after-command` hooks of its own (to confirm Warp's hooks append rather than replace).
- Regression pass on zsh, bash, fish (14).

## Risks

- **User hooks that replace instead of append.** A `rc.elv` doing `set edit:after-command = [...]` (without `$@edit:after-command`) would drop Warp's hook if run after ours. Mitigation: Warp installs its hooks after the user rc is evaluated (step 3 ordering).
- **Prompt closures that print directly.** Elvish prompts may write to stdout instead of returning styled text; wrapping still works because OSC markers are printed in the same stream, verified with a plain `print`-based prompt.
- **`eval … &on-end` namespace merge.** `edit:add-vars` rejects names with `:`; module namespaces imported with `use` inside the rc are exposed as `name:` vars and pass through. Covered by the manual `use` test.

## Follow-ups

- SSH / subshell warpification for Elvish.
- Native completion passthrough via `edit:complete-filename` / arg-completers.
- History import from Elvish's bbolt db (would need a reader or `edit:command-history` dump at bootstrap).
