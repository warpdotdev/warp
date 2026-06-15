# WARPER-006: stop-ship local security ports

## Summary

Warper should port only the upstream security fixes that protect retained local execution boundaries from command execution, local file overwrite, clipboard exposure, spoofed terminal lifecycle hooks, or restore-time shell injection. Dependency bumps, memory leaks, and general hardening stay out until a targeted pass proves Warper will fail without them.

## Why this matters for Warper

Warper's local-first promise collapses if untrusted repository names, file paths, markdown, terminal output, or saved conversation paths can execute commands or mutate local files. These are not "cybersecurity" decorations. They are the line between a local terminal/agent the user can run and one that can be driven by hostile local data.

## Source commits

| Commit | Upstream why | Current Warper evidence | Resolution |
| --- | --- | --- | --- |
| `4295ec08` | PR `#25398` is not publicly resolvable; commit body names display-chip RCE `GHSA-hgvx-4xvm-39pw`. | `app/src/context_chips/display_chip.rs:1713` manually quotes `cd`; `:1717` emits raw `git checkout {branch_name}`. | Port manually. |
| `43f4f483` | PR `#25351` is not publicly resolvable; title/diff fix command injection in code search tools. | `app/src/ai/blocklist/action_model/execute.rs:1141` and `:1156` interpolate paths into shell predicates. | Port manually. |
| `0c1e2432` | PR `#25258` says env-prefixed commands bypassed denylist matching. | `app/src/ai/blocklist/permissions.rs:831` decides autoexecution and `:852` applies the denylist. | Port. |
| `b6caa957` | PR `#26138` says Agent Mode `is_file_path` and `is_git_repository` shell checks did not quote untrusted paths. | `execute.rs:1141` and `:1156` build those checks from caller paths. | Port. |
| `7f0c4dd2` | PR `#25353` is not publicly resolvable; commit body says markdown should emit `OpenFileWithTarget` only for trusted known-extension targets. | `app/src/notebooks/link.rs:348` emits local open events from markdown links. | Port manually. |
| `b1a41d0b`, `164e60e4` | PR `#25339` cites OSC 52 clipboard exposure `GHSA-wgqj-4c26-7c4g`; PR `#25625` adds user-visible blocked-operation handling. | `app/src/terminal/model/grid/ansi_handler.rs:1159` sends clipboard stores, `:1175` sends clipboard loads, and `app/src/terminal/view.rs:8467-8471` touches the local clipboard. | Port manually. |
| `f3b9ce1c` | PR `#25261` says non-inline iTerm file payloads could overwrite cwd files. | `app/src/terminal/model/terminal_model.rs:2846-2916` keeps iTerm image/payload handling. | Port. |
| `32d21d15` | PR `#25395` says arbitrary PTY output could spoof DCS lifecycle hooks. | Warper keeps DCS lifecycle/bootstrap hooks in `app/src/terminal/model/ansi/dcs_hooks.rs` and `app/src/terminal/view.rs:6657`. | Port manually. |
| `c697c8f5` | PR `#25383` cites restore `cd` advisory `GHSA-8659-m852-gmfx`. | `app/src/terminal/view/load_ai_conversation.rs:271-273` runs `cd "{path}"`. | Port manually. |
| `861dacea` | PR `#25365` says Linux external editor launch used `sh -c` with untrusted field-code expansions. | `app/src/util/file/external_editor/linux.rs:99` builds editor commands from desktop field codes and `:118` invokes `sh`. | Port. |

## Behavior

1. Shell commands built from repository names, branch names, file paths, tool arguments, or saved paths treat those values as data.
2. Local file/repo predicate checks quote path arguments for the shell family they execute.
3. Denylist checks normalize leading environment assignments before deciding whether a command is blocked.
4. Markdown local links use the upstream trust gate before emitting `OpenFileWithTarget`.
5. OSC 52 clipboard access is denied by default and can only be enabled through local settings.
6. Non-inline iTerm file payloads are ignored; inline images remain supported.
7. DCS lifecycle hooks require the local integrity proof from the ported implementation.
8. Conversation restore escapes local `cd` paths before execution.
9. Linux external editor launch uses structured argv handling rather than `sh -c` for file paths.

## Validation

- Add regression tests for malicious branch names, file paths, and repo paths.
- Add tests proving env-prefixed denied commands still fail closed.
- Add markdown-link tests for unsafe local targets.
- Add OSC 52 default-deny and blocked-feedback tests.
- Add iTerm non-inline payload tests proving cwd files are not written.
- Add DCS spoofing tests for raw PTY output.
- Add restore tests for paths containing shell metacharacters.
- Add Linux external editor tests for filenames containing shell metacharacters.
