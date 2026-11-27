# Tech Spec: Nushell support

**Issue:** [warpdotdev/warp#2038](https://github.com/warpdotdev/warp/issues/2038)
**Product spec:** [`product.md`](product.md)

## Context

Warp's shell integration is keyed by `ShellType` and fans out through shell detection, local shell spawning, bootstrap scripts, command execution, environment-variable serialization, update commands, and UI shell-selection metadata.

Relevant code:

- `crates/warp_terminal/src/shell/mod.rs` — central shell metadata: `ShellType`, executable-name parsing, markdown language spec parsing, history/rcfile locations, command combiners, and alias parsing.
- `app/src/terminal/available_shells.rs` — shell discovery and display names for local shells and Windows executables.
- `app/src/terminal/local_tty/shell.rs` — direct, WSL, and MSYS2 shell starter paths plus shell-specific spawn arguments.
- `app/src/terminal/bootstrap.rs` and `app/assets/bundled/bootstrap/*` — bootstrap asset selection and shell-specific init/body scripts.
- `app/src/terminal/model/session/command_executor/*` and `app/src/terminal/model/terminal_model.rs` — command execution and unsupported-shell handling.
- `app/src/env_vars/mod.rs` — environment-variable initialization/export syntax by shell.
- `app/src/drive/export.rs` and `app/src/drive/index.rs` — Warp Drive environment-variable collection export/copy.
- `app/src/autoupdate/linux.rs` — Linux package-manager update commands that require success-gated control flow before `warp_finish_update`.

Before this change, Nushell could only be represented as the broad POSIX `ShellFamily`, which caused two problems: Warp did not know how to detect/bootstrap `nu`, and paths that converted `ShellFamily::Posix` back to a concrete shell defaulted to Bash syntax.

## Proposed changes

1. Add `ShellType::Nu` as a first-class shell type.
   - Detect `nu`, `-nu`, Unix paths ending in `/nu`, and Windows executable basenames whose whole filename is `nu.exe`.
   - Compare Windows executable basenames case-insensitively for `nu.exe`, while still requiring the whole basename to match so unrelated names such as `menu.exe` and `NuGet.exe` are rejected.
   - Parse markdown language specs `nu` and `nushell` as Nushell.
   - Keep `ShellFamily::from(ShellType::Nu)` as `Posix` only for legacy family-level behavior; every `ShellFamily::Posix` path reachable from `ShellType::Nu` must be inventoried below as safe, Nushell-gated, or unsupported before implementation.
   - Add Nushell history and rcfile locations.
   - Parse Nushell aliases from the structured JSON payload emitted by the bootstrap metadata hook; JSON is the only alias wire format for Nushell.

2. Add local shell discovery and spawn support.
   - Include `nu`/`nu.exe` in shell discovery and display it as "Nushell".
   - Add `/bin/nu` fallback on Unix-like systems.
   - For direct local sessions, invoke Nushell as `nu --login --execute <warp init script>`. The `--login` flag preserves normal login-shell startup, and `--execute` runs Warp's init snippet before leaving the process in an interactive shell.
   - Startup ordering is part of the contract: Nushell completes its normal user startup before Warp's init script runs. That startup includes `env.nu`, `config.nu`, `login.nu` for login shells, `*.nu` files from `$nu.vendor-autoload-dirs`, and `*.nu` files from `$nu.user-autoload-dirs` in Nushell's own documented/runtime order. Warp must not manually replay, skip, or reorder those files; validation should assert they all run before Warp init/bootstrap markers without making Warp responsible for Nushell's internal relative ordering.
   - If a user startup file throws an error before `--execute` runs, Nushell reports that error and Warp treats the shell like any other shell that failed to bootstrap; Warp does not mask or rewrite user config failures.
   - Support WSL and MSYS2 spawning when the detected shell basename maps to `ShellType::Nu`.
   - WSL validation must cover both spawn arguments and fallback behavior: `/usr/bin/nu` should produce a `wsl --distribution <distro> --shell-type standard --exec /usr/bin/nu --login --execute <warp init script>`-style launch, while unsupported basenames should return the existing unsupported-shell fallback instead of being treated as Nushell.
   - MSYS2 validation must cover Nushell-specific spawn args (`--login`) and verify PowerShell remains unsupported for MSYS2 as before.
   - Use basename parsing for WSL detection to avoid substring false positives.

### Nushell literal escaping

Generated Nushell source code must use a Nushell-specific double-quoted string serializer, not JSON string serialization. This serializer is used for temp bootstrap paths, environment-variable constants and names, updater dynamic values, and any other generated Nushell code that embeds data:

- wrap the result in double quotes;
- escape `"` as `\"` and `\` as `\\`;
- escape newline, carriage return, and tab as `\n`, `\r`, and `\t`;
- escape other C0 control characters and DEL as Nushell Unicode escapes of the form `\u{hex}` with 1-6 lowercase hex digits;
- preserve other Unicode scalar values as data;
- never emit JSON-only Unicode escapes such as `\u001b` into generated Nushell source.

JSON serialization is still valid for wire payloads that Nushell emits with `to json -r`, such as the alias metadata payload; it is not the source-code escaping contract for generated Nushell commands.

3. Add Nushell bootstrap assets.
   - `nu_init_shell.nu` sends the initial `InitShell` hook and establishes `WARP_SESSION_ID`.
   - `nu_body.nu` installs Nushell functions/hooks for `Preexec`, `Precmd`, `CommandFinished`, `Bootstrapped`, `Clear`, `InputBuffer`, `FinishUpdate`, prompt-mode toggles, PATH append, and initial working directory handling.
   - `nu.nu` includes the body script through the existing bundled asset mechanism.
   - Split startup/bootstrap delivery deliberately:
     1. User startup files run first as part of the login Nushell process.
     2. The early Warp init script is `nu_init_shell.nu`. Direct Unix/macOS sessions, direct Windows `nu.exe` sessions, and WSL sessions pass the rendered script contents as the single argument to `--execute`: argv is `[<selected nu path>, "--login", "--execute", <rendered nu_init_shell.nu contents>]` for direct sessions, and `["wsl", "--distribution", <distro>, "--shell-type", "standard", "--exec", <wsl nu path>, "--login", "--execute", <rendered nu_init_shell.nu contents>]` for WSL. The rendered command string must be the script body itself, with build placeholders such as `@@USING_CON_PTY_BOOLEAN@@` substituted before process spawn; it is not a path and it does not dynamically construct a `source` command. Nushell remains interactive after this init snippet runs.
     3. MSYS2 Nushell sessions use the temp-source fallback: argv is `[<selected msys2 nu path>, "--login"]`, then Warp sources a secure temp file whose first rendered statements are the `nu_init_shell.nu` contents followed by the `nu.nu`/`nu_body.nu` contents. This keeps MSYS2 on the existing slow-PTY-safe RC-file bootstrap path.
     4. For direct Unix/macOS, direct Windows, and WSL sessions, the larger Warp body bootstrap is not embedded in the `--execute` argument. Warp writes the rendered `nu.nu`/`nu_body.nu` body to the same temp-file source bootstrap path used for shells that should not receive large bootstrap payloads directly through the PTY, then sends exactly one source command after the process is running: `source <temp_bootstrap_path_literal>\n`. The path literal must be produced by the Nushell literal serializer above, so spaces, quotes, backslashes, brackets, control characters, and other path characters cannot change the command structure. Use `source`, not `source-env`, because the body defines Nushell functions and updates `$env`/`$env.config`.
     5. If another platform-specific launcher cannot carry `--execute` safely, the implementation must use the same MSYS2-style temp-source fallback. If temp-file bootstrap setup fails, Warp may fall back to the existing bracketed-paste/bootstrap writer, but it must preserve the same ordering and literal-path escaping.
   - Do not modify the user's `env.nu`, `config.nu`, or `login.nu`; the temp source file is owned by the Warp session and cleaned up like other RC-file bootstrap artifacts.
   - Create temp bootstrap files securely because they are executable shell code: use exclusive creation rather than opening a preexisting path, refuse symlinks, write owner-only permissions before sourcing (`0600` on Unix-like filesystems and a current-user-only ACL/equivalent on Windows where applicable), and avoid reusing paths across sessions.
   - Bootstrap must merge with user configuration rather than replacing it: prepend Warp's `pre_execution` and `pre_prompt` hooks ahead of existing hook lists, append Warp keybindings to existing keybindings, and store the user's original prompt closures/strings before changing prompt indicators.
   - Warp-managed prompt mode sets Nushell prompt indicators to empty strings and emits Warp prompt escape sequences. Honor-user-prompt mode restores the stored user prompt indicators and calls the stored `PROMPT_COMMAND`/`PROMPT_COMMAND_RIGHT` values when they are closures or strings.
   - Disable Nushell's built-in OSC 133/633 shell integration flags in `$env.config.shell_integration` while Warp's integration is active to avoid duplicate prompt markers.

4. Add Nushell command-execution behavior.
   - Use Nushell's `--no-config-file` flag where command executors need isolated command execution that should not re-run user config.
   - Preserve command escaping rules separately from Bash/Zsh/Fish where needed.
   - Route unsupported local child/subshell flows through the existing unsupported-shell handling instead of pretending Nushell is Bash.

5. Add Nushell environment-variable serialization.
   - Constants serialize as Nushell double-quoted string literals assigned to `$env`, using the Nushell literal serializer above. This makes quotes, backslashes, newlines, control characters, `$()`, semicolons, and other shell-significant characters data rather than executable Nushell syntax.
   - Bare environment names are allowed only when they match `[A-Za-z_][A-Za-z0-9_]*`. Any other name serializes through the same Nushell literal escaping and is emitted through a Nushell-safe quoted environment key or equivalent `load-env`/record update path.
   - Command-backed values are intentionally executable Nushell expressions emitted as `(<command>)`. These commands come from Warp's environment-variable command model and are not escaped as data; callers must treat them as trusted command snippets.
   - Secret-backed values are intentionally executable command substitutions around the external secret-manager retrieval command. The secret reference/manager configuration is the trust boundary, and the command output becomes the environment value.
   - Command-backed and secret-backed values may be serialized only after an explicit user action such as applying, copying, or exporting an environment-variable collection, and only for content the current user owns or has explicitly chosen to trust. Actual command execution happens only when the user applies the collection to the active shell or runs the generated text. Drive sync, preview, import, shared collection browsing, and metadata rendering must treat those snippets as inert data and must not evaluate them.
   - Shared or imported Drive environment-variable collections that contain command-backed values must keep using the existing permission/confirmation boundary before execution. If the implementation cannot prove ownership/trust or explicit user intent for a path, that path must render the command text for review or route to unsupported handling instead of executing it automatically.
   - Secret-backed generated commands and their output are redaction boundaries. Drive may persist the existing structured secret reference, but it must not persist the rendered secret-manager command or command stdout in Drive metadata. Telemetry, debug logs, bootstrap/apply failure output, and UI error summaries must omit or redact the rendered secret-manager command, stdout, and any stderr that may include secret values.
   - Warp Drive export and copy paths carry `ShellType` from the active terminal session so Nushell sessions emit Nushell syntax.

6. Add Nushell-safe update command construction.
   - Keep generic `and_combiner()` behavior unchanged for existing shells.
   - Special-case Linux package-manager update commands for `ShellType::Nu` so package-manager commands and `warp_finish_update` run inside one outer Nushell block with the exact shape `try { <package-manager sequence>; warp_finish_update <update_id> } catch {|err| print $err.rendered }`.
   - Every dynamic fragment interpolated into the Nushell updater block must be rendered before command construction as a Nushell string literal or bound local variable using the Nushell literal serializer above. This includes package names, repository names, update ids, cache/temp paths, backup paths, and generated config paths. Do not concatenate raw dynamic strings into external command text or into the `warp_finish_update` call.
   - The preferred Nushell shape is to bind dynamic values once, for example `let warp_update_id = <escaped string literal>` and `let warp_package_name = <escaped string literal>`, then pass those variables as arguments to external commands and `warp_finish_update`. Package-manager selection and subcommand names are enum-controlled by Warp; only data arguments may come from dynamic values.
   - Prevent option injection in addition to shell injection. For each dynamic value passed to `sudo`, package managers, or file commands, the implementation must either insert an option terminator (`--`) before dynamic positional arguments when the target command supports it, or reject/normalize option-looking values such as leading-dash package names, repository names, update ids, and paths before constructing the block. Do not rely on string escaping alone to make `-foo` safe for an external command.
   - Tests must include malicious-looking dynamic values containing spaces, quotes, semicolons, `$()`, newlines, and leading dashes, and assert those values remain a single Nushell string/argument without introducing extra statements, moving `warp_finish_update` outside the success gate, or being interpreted as options by generated `sudo`/package-manager/file commands.
   - The contract depends on Nushell `0.109.x` behavior: a non-zero external command exit inside `try` raises an error, jumps to `catch`, and skips the remaining statements in that `try` block. This is the success gate that prevents `warp_finish_update` from running after a failed package-manager command.
   - Apt sequence: optionally run `try { warp_handle_dist_upgrade <repo> } catch { null }` first. That inner failure is intentionally ignored to preserve the existing dist-upgrade best-effort behavior. Then run `sudo apt update; sudo apt install <package>`. Failure in either sudo command aborts the outer `try` before finish.
   - Yum/Dnf/Zypper sequence: run the single package-manager upgrade/update command inside the outer `try`; any non-zero exit aborts before finish.
   - Pacman sequence: optional key setup commands, optional repository backup/config commands, and `sudo pacman -Sy <package>` all run as ordered statements inside the outer `try`; any non-zero exit in setup or install aborts before finish. Commands that must bypass Nushell built-ins use the external-command caret form such as `^mkdir` and `^cp`.

7. Scope remote/subshell follow-up work explicitly.
   - Remote SSH/session warpification and Nushell subshell bootstrap remain unsupported in this first iteration.
   - Future work can add those paths once the local shell contract is stable.

## `ShellFamily::Posix` and generated-command inventory

Nushell remains in `ShellFamily::Posix` only where the existing family-level API represents broad platform/path behavior. It is not permission to emit Bash/POSIX shell syntax for a Nushell session. Before the implementation PR lands, every `ShellFamily::Posix` path that can be reached by `ShellType::Nu` must be classified and tested as one of the following:

| Area | Contract for Nushell |
| --- | --- |
| Path escaping and editor/completion/path display (`warp_util::path::ShellFamily`; editor/input/completer, terminal view, URI, CLI install, slash command, MCP settings, and "open in Warp" call sites) | Safe only for data escaping/unescaping of paths and text. These paths must not append command separators, control flow, exports, or command substitutions. Regression tests should include Nushell path strings with spaces, quotes, and backslashes. |
| Environment-variable serialization (`app/src/env_vars/mod.rs`) | Nushell-gated. Dispatch on `ShellType::Nu` before falling back to `ShellFamily::Posix`; constants use the Nushell literal serializer, command/secret values use explicit Nushell command substitutions, and non-identifier names are quoted as Nushell environment keys. |
| Warp Drive environment-variable export/copy (`app/src/drive/export.rs`, `app/src/drive/index.rs`) | Nushell-gated. Carry the active terminal `ShellType` rather than reducing to `ShellFamily`; if only a family is available, Nushell export/copy is unsupported instead of defaulting to Bash syntax. |
| External secret command snippets used by env vars (`ExternalSecret::get_secret_extraction_command` and callers) | Nushell-gated and security-sensitive. Do not add POSIX-only prefixes such as Bash's leading backslash for Nushell, and do not execute snippets outside the explicit user-action trust boundary above. |
| Local shell startup/bootstrap (`app/src/terminal/local_tty/shell.rs`, `app/src/terminal/bootstrap.rs`) | Nushell-gated. Direct/WSL/MSYS2 launch arguments, temp-file bootstrap sourcing, and fallback behavior must be asserted for `ShellType::Nu`; unsupported local child/subshell flows route through the existing unsupported-shell handling. |
| Command executors (`app/src/terminal/model/session/command_executor/*`) | Nushell-gated. Use Nushell flags such as `--no-config-file` and Nushell quoting where isolated execution is needed; otherwise mark the executor path unsupported for Nushell rather than running Bash through `ShellFamily::Posix`. |
| Linux updater command construction (`app/src/autoupdate/linux.rs`) | Nushell-gated. The implementation must not use generic POSIX `&&` for Nushell. Package-manager sequences use the Nushell `try { ...; warp_finish_update ... } catch { ... }` contract below. |

Generated success-gated command inventory:

- The current implementation has one `ShellType::and_combiner()` consumer: `app/src/autoupdate/linux.rs`. The Nushell implementation must branch before that generic combiner is used and must assert the generated update command contains no POSIX `&&`.
- `ShellType::and_combiner()` may keep its existing Bash/Zsh/Fish/PowerShell behavior for non-Nushell shells. If a future implementation adds a new generated-command call site that is reachable from `ShellType::Nu`, that call site must add a Nushell-specific branch and tests, or explicitly route Nushell to unsupported-shell handling.

## Nushell metadata collection contract

Behavior 8 is satisfied by the `Bootstrapped` hook payload sent from `nu_body.nu`. The contract is concrete and testable:

- `shell`: literal `"nu"`; sent as a string equal to `nu`.
- `shell_version`: source is `version | get version`; sent as a string that parses as the running Nushell version.
- `histfile`: source is `$env.config.history.file_format?` plus `$nu.history-path`; sent as a string path only when history format is `plaintext`, otherwise an empty string. Tests should assert the plaintext case reports Nushell's platform-derived history path.
- `config_path`: source is `$nu.config-path? | default ""`; sent as a string path equal to Nushell's platform-derived `config.nu` path when available.
- `env_path`: source is `$nu.env-path? | default ""`; sent as a string path equal to Nushell's platform-derived `env.nu` path when available.
- `loginshell_path`: source is `$nu.loginshell-path? | default ""`; sent as a string path equal to Nushell's platform-derived `login.nu` path when available.
- `home_dir`: source probes Nushell's runtime home-directory fields and then the environment: `$nu.home-path? | default ($nu.home-dir? | default ($env.HOME? | default ""))`. The local Nushell `0.109.x` line exposes `$nu.home-path`, while some documentation and future builds may expose `$nu.home-dir`; tests should assert the value matches whichever field the running Nushell provides.
- `path`: source is `$env.PATH` normalized by `warp_path_string`; sent as a platform path string joined with `(char esep)` when `$env.PATH` is a list, or the raw string when it is already a string.
- `editor`: source is `$env.EDITOR? | default ""`; sent as the editor string or empty.
- `aliases`: source is `scope aliases | each {|alias| { name: $alias.name, expansion: ($alias.expansion? | default "") } } | to json -r`; sent as a JSON array of `{ name, expansion }` objects. The parser must preserve expansions containing tabs, newlines, quotes, or semicolons.
- `function_names`: source is `scope commands | where type == "custom" | get name | uniq | str join (char nl)`; sent as newline-separated unique custom command names.
- `builtins`: source is `scope commands | where type == "built-in" | get name | uniq | str join (char nl)`; sent as newline-separated unique built-in command names.
- `keywords`: source is `scope commands | where type == "keyword" | get name | uniq | str join (char nl)`; sent as newline-separated unique keyword names.
- `env_var_names`: source is `$env | columns | str join (char nl)`; sent as newline-separated environment-variable names without values.
- `os_category`: source is `$nu.os-info.name?` mapped to `MacOS`, `Linux`, `Windows`, or empty; tests should assert it matches the host category or empty for unknown OS names.
- `linux_distribution`: source is parsed `NAME=` from `/etc/os-release` or `/usr/lib/os-release` on Linux; sent as a string and expected to be non-empty on standard Linux images when os-release exists.
- `wsl_name`: source is `$env.WSL_DISTRO_NAME? | default ""`; sent as the WSL distro name or empty.
- `shell_path`: source is `$nu.current-exe`; sent as a string path equal to the running Nushell executable.
- `vi_mode_enabled`: source is `$env.config.edit_mode? == "vi"`; sent as `"1"` for vi mode or empty otherwise.

Automated metadata tests should run a rendered bootstrap script under a controlled Nushell environment that defines at least one alias, one alias whose expansion contains a tab/newline, one custom command, one test environment variable, and deterministic PATH/EDITOR values. The test should capture the `Bootstrapped` JSON payload, decode it, and assert the field formats above, including platform-derived `$nu.*-path` values when available.

## Compatibility and reproducible Nushell provisioning

The minimum supported Nushell version is `0.109.0`. The bootstrap and tests rely on the following Nushell behavior available in that line and newer versions:

- `nu --login --execute <commands>` starts a login shell, runs the command string after user startup files, and remains interactive.
- `$env.config.hooks.pre_execution`, `$env.config.hooks.pre_prompt`, prompt closures, and keybinding records can be updated with `upsert`.
- `scope commands`, `scope aliases`, `to json -r`, `encode hex`, `hide-env`, and `commandline` host commands are available with the syntax used by the bootstrap.
- External command failures inside `try { ... } catch { ... }` jump to `catch` and prevent later statements in that `try` block from running; the updater success-gating tests rely on this behavior.

Older Nushell versions are best-effort. If an older installed `nu` lacks required flags, hooks, or syntax, Warp should surface the normal shell bootstrap/startup failure rather than falling back to Bash syntax. Future compatibility expansion should add version-specific smoke tests before lowering the minimum.

For reproducible CI/reviewer validation, tests must not rely on an author-local `nu` from a user profile. Provision Nushell through a pinned Nix input, for example `nix shell github:NixOS/nixpkgs/<repo-flake-lock-rev>#nushell -c nu --version`, or add Nushell to the repo's CI/devshell package set and assert `version >= 0.109.0` before running smoke tests. If a non-Nix environment lacks `nu`, Nushell smoke/interactive tests should skip with an explicit "Nushell binary not available" reason rather than silently passing.

## Testing and validation

Product behavior coverage:

- Behavior 1, 2, and 5: unit tests in `crates/warp_terminal/src/shell/mod_tests.rs` and `app/src/terminal/local_tty/shell_tests.rs` verify `nu`, `-nu`, `/usr/bin/nu`, Windows `nu.exe`/`NU.EXE` case-insensitive basename parsing, WSL `nu` basename parsing, WSL Nushell launch arguments, direct Windows Nushell launch arguments, MSYS2 Nushell launch arguments and temp-source fallback, and false positives such as `menu.exe`, `NuGet.exe`, and `/usr/bin/menu.exe`.
- Behavior 3 and 14: existing shell-discovery tests are extended so Nushell appears with the known shell types without regressing other shells.
- Behavior 4, 6, 7, 8, 9, and 10: bootstrap unit coverage verifies Nushell asset selection; rendered-script smoke tests verify that the init/body scripts parse and run under the supported local `nu` binary after build placeholders are replaced; startup-order validation uses a controlled Nushell config home with `env.nu`, `config.nu`, `login.nu`, vendor autoload, and user autoload marker scripts and asserts every marker runs before Warp's init/bootstrap markers; temp-file bootstrap tests verify secure temp file creation and the exact `source <temp_bootstrap_path_literal>` command for paths containing spaces, quotes, backslashes, brackets, and control characters; metadata payload tests should cover structured alias rows, delimiter-containing alias expansions, newline-separated command-name lists for custom/built-in/keyword commands, newline-separated environment-variable names, normalized PATH, `$nu.current-exe` shell path, `$nu.home-path`/`$nu.home-dir` home-directory probing, and platform-derived `$nu.history-path`/`$nu.config-path`/`$nu.env-path`/`$nu.loginshell-path` values.
- Behavior 11 and 12: `app/src/env_vars/mod.rs` tests verify Nushell initialization/export syntax, command substitution, quoted environment-variable names, and the Nushell literal serializer for quotes, backslashes, newlines, tabs, carriage returns, `\u{0}`, `\u{1b}`, DEL, `$()`, and semicolons; Drive export tests cover the `ShellType` API change; Drive/shared/imported collection tests verify command-backed and secret-backed values are inert during sync, preview, import, and browsing, require explicit user action before generated command text can be applied to the active shell, and redact rendered secret-manager commands/stdout from metadata, telemetry, logs, and apply/bootstrap failure output.
- Behavior 13: `app/src/autoupdate/linux_test.rs` verifies the Nushell update command gates `warp_finish_update` behind the package-manager command sequence, emits the expected outer `try { ... } catch {|err| print $err.rendered }` shape with a bound error parameter, preserves best-effort dist-upgrade handling for Apt, does not emit POSIX `&&`, safely quotes/binds dynamic package, repository, update-id, cache, and path values before inserting them into privileged command blocks, including control characters that JSON would encode as invalid Nushell `\u00xx` escapes, and prevents leading-dash option injection with `--` or command-specific validation/rejection.
- Behavior 15: unsupported child/subshell paths intentionally keep using existing unsupported-shell behavior for Nushell.
- POSIX-family inventory: add regression coverage or an explicit unsupported-shell assertion for every Nushell-reachable entry in the `ShellFamily::Posix` and generated-command inventory above. The implementation PR should update this inventory if `rg 'ShellFamily::from|shell_family\\(|ShellFamily::Posix|and_combiner\\(' app crates` finds a new concrete syntax path reachable from `ShellType::Nu`.

Planned repo-root validation commands for the implementation PR:

```bash
cargo fmt --all -- --check
cargo test -p warp env_vars::tests --lib
cargo test -p warp drive::export::tests --lib
cargo test -p warp_terminal test_from_name --lib
cargo test -p warp_terminal test_nu_parse_aliases --lib
cargo test -p warp_terminal test_nu_string_literal_escaping --lib
cargo test -p warp test_nu_startup_order_includes_autoload_dirs --lib
cargo test -p warp test_nu_temp_bootstrap_source_escapes_path_literal --lib
cargo test -p warp test_nu_temp_bootstrap_file_is_created_securely --lib
cargo test -p warp test_nu_update_command_gates_finish_update_on_success --lib
cargo test -p warp test_nu_update_command_uses_nu_dist_upgrade_handler --lib
cargo test -p warp test_nu_update_command_escapes_dynamic_fragments --lib
cargo test -p warp test_nu_update_command_prevents_option_injection --lib
cargo clippy -p warp -p warp_terminal --all-targets --tests -- -D warnings
```

When using the companion Nix devshell locally, run those commands from the repository root through a relative devshell invocation such as `nix develop ../warp#default -c bash -lc '<command>'`; CI should use its normal pinned environment.

Rendered-script smoke validation:

```bash
nu -n --no-std-lib rendered-nu_init_shell.nu
WARP_BOOTSTRAPPED= WARP_SESSION_ID=12345 WARP_INITIAL_WORKING_DIR="$PWD" nu -n --no-std-lib rendered-nu_body.nu
```

Both smoke commands should exit successfully after replacing build placeholders such as `@@USING_CON_PTY_BOOLEAN@@` with a concrete boolean. These smoke checks prove script syntax and basic runtime evaluation, but they do not prove the interactive PTY lifecycle by themselves.

Interactive lifecycle acceptance validation:

1. Build/run Warp from the local checkout with the shared `../warp` flake development environment.
2. Select Nushell or set the default shell path to the local `nu` binary.
3. Open a new session and verify it remains interactive after the Warp init script runs.
4. Run `pwd`, `cd`, `echo`, `false`, and a command after `false`; verify block boundaries, working directory reporting, prompt redraw, and non-zero exit-code reporting.
5. Toggle between Warp prompt mode and honoring the user prompt; verify the original prompt is restored in honor-user-prompt mode and Warp block markers continue to work.
6. Verify custom user `pre_execution`/`pre_prompt` hooks and keybindings still run after Warp prepends/appends its integration entries.
7. On Windows, validate WSL Nushell launch arguments and unsupported-shell fallback behavior with a WSL distro whose `$SHELL` is `nu`; validate MSYS2 Nushell launch arguments when MSYS2 is available.
8. Export/copy a Warp Drive environment-variable collection while the active session is Nushell and verify the generated text is valid Nushell `$env` assignment syntax.
9. On Linux package builds, inspect the generated update command and verify `warp_finish_update` is not emitted after a failing package-manager command.

## Risks and mitigations

- **Nushell syntax changes over time.** Nushell is still evolving, so the bootstrap sticks to stable, simple constructs where possible and is covered by smoke tests against the local `nu` binary.
- **POSIX-family assumptions.** Nushell remains `ShellFamily::Posix` for legacy escaping/family APIs, but code that needs concrete syntax now carries `ShellType` through the flow.
- **False-positive detection.** Basename checks and regression tests reduce the risk from the short executable name `nu`.
- **Partial first iteration.** Remote and subshell flows are explicit non-goals so unsupported paths fail predictably instead of producing incorrect Bash/POSIX behavior.

## Follow-ups

- Add remote SSH/session warpification support for Nushell.
- Add Nushell subshell bootstrap support.
- Investigate deeper integration with Nushell-native completions, plugins, LSP, or MCP once local shell support is stable.
