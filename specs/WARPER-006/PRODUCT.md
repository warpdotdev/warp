# WARPER-006: upstream local security fixes

## Summary

Warper should selectively port upstream security fixes that protect local terminal use, local file opening, local agent tools, and local dependency hygiene without reintroducing hosted Warp services or cloud-shaped settings.

## Why this matters for Warper

Warper's value is that a user can run a local terminal and a local-first agent without trusting Warp-hosted infrastructure. That makes local execution boundaries more important, not less: branch names, file paths, markdown links, and agent tool arguments come from local repositories and terminal output that may be untrusted. These fixes are relevant only where they protect retained local behavior: local terminal prompts, local markdown/notebook rendering, local file opening, local agent shell tools, and local dependency code that is still in Warper's build. They are not relevant as generic upstream security churn, and they should not be used to justify cloud settings sync, hosted crash reporting, or unsupported platform work.

## Source commits

| Commit | Upstream why | Motive | Current Warper evidence | Resolution | Manual-port exclusions |
| --- | --- | --- | --- | --- | --- |
| `4295ec08` | Public PR not resolvable; diff/tests replace display-chip shell strings after command-injection exposure. | Painkiller | `app/src/context_chips/display_chip.rs:1552` and `:1717` build local shell actions from cwd/git/node data. | Port manually | Keep typed/quoted local chip commands only. |
| `7f0c4dd2` | Public PR not resolvable; diff/tests harden markdown local-link opening. | Painkiller | `app/src/notebooks/link.rs:260` and `:271` still dispatch local markdown links. | Port | None. |
| `43f4f483` | Public PR not resolvable; diff/tests quote grep and file-glob shell arguments. | Painkiller | `openrouter.rs:462`, `openrouter.rs:483`, `grep.rs:375`, and `file_glob.rs:226` expose/build shell-backed local search. | Port manually | Quote only shell families Warper actually executes. |
| `861dacea` | Public PR not resolvable; diff/tests remove shell construction from Linux external editor launch. | Painkiller | `app/src/util/file/external_editor/linux.rs:450` and `:515` build local editor launches. | Port | None. |
| `0c1e2432` | Public PR not resolvable; diff/tests strip leading env assignments after denylist bypass. | Painkiller | `openrouter.rs:407` exposes shell execution; `permissions.rs:848` checks local commands. | Port | None. |
| `b6caa957` | Public PR not resolvable; diff/tests escape `is_file_path` and `is_git_repository` paths. | Painkiller | `execute.rs:1141` and `:1156` interpolate paths into shell predicates used by local tools. | Port | None. |
| `b1a41d0b` + `164e60e4` | Public PRs not resolvable; diffs gate OSC 52 clipboard read/write and add blocked-setting UX after terminal-output clipboard exposure. | Painkiller | `grid/ansi_handler.rs:1148` and `terminal/view.rs:8467` still route local clipboard load/store. | Port manually | Implement local default-deny setting and local banner only; no cloud settings sync. |
| `f3b9ce1c` | Public PR not resolvable; diff/tests disable non-inline iTerm file downloads. | Painkiller | `terminal_model.rs:2868` writes non-inline iTerm payloads into cwd. | Port | None. |
| `32d21d15` | Public PR not resolvable; diff/tests add DCS hook integrity after spoofable lifecycle hooks. | Painkiller | `dcs_hooks.rs:84`, `handler.rs:239`, and `terminal/view.rs:6657` keep DCS lifecycle/bootstrap hooks. | Port manually | Local integrity mechanism only; omit remote, Windows, shared-session, and broad bootstrap churn. |
| `c697c8f5` | Public PR not resolvable; diff/tests escape restored-conversation `cd` paths. | Painkiller | `app/src/terminal/view/load_ai_conversation.rs:268` restores local conversations by issuing `cd`. | Port manually | Keep local path escaping here; hosted/non-local conversation absence remains owned by WARPER-001. |
| `88c344e2` | Public PR not resolvable; diff/tests fix SSH command injection. | Painkiller | `app/src/lib.rs:1103` registers SSH paths; `app/src/terminal/ssh/util.rs:232` builds shell commands. | Port manually | Shell escaping only; no remote-server installation, sharing, or hosted remote management. |
| `9d9972cb` | PR `#10263` updates Diesel for GHSA/RUSTSEC SQLite UTF-8 corruption. | Painkiller | `Cargo.toml:123` pins Diesel; `Cargo.lock:3670` contains it. | Port | None. |
| `64a0dfbe` | PR `#10060` updates transitive `rand 0.9.1` for `GHSA-cq8v-f236-94qc`. | Painkiller | `Cargo.toml:201` declares `rand`; `Cargo.lock:9630` contains transitive `rand 0.9.1`. | Port | None. |
| `ac091058` | PR `#10513` updates OpenSSL after an output-buffer overflow fix and abort fixes. | Painkiller | `Cargo.lock:8444` and `:8476` contain `openssl 0.10.78`. | Port | None. |
| `cc1ee636` | PR `#12090` updates `tar` for PAX header desync GHSA. | Painkiller | `Cargo.lock:11862` contains `tar 0.4.45`. | Port | None. |
| `2f84587a` | PR `#9665` fixes a CoreText font descriptor leak. | Painkiller | `Cargo.toml:467` patches CoreText; `crates/warpui/src/platform/mac/fonts.rs:82` enumerates descriptors. | Port | None. |

## Goals / Non-goals

- Goal: close local command injection and file-opening vulnerabilities in retained terminal, markdown, editor, and agent-tool paths.
- Goal: make terminal escape sequence handling safer for untrusted output.
- Goal: keep dependency security updates constrained to dependencies present in Warper's current lockfile.
- Goal: keep security settings local-only and Warper-branded.
- Non-goal: port upstream cloud settings sync, hosted telemetry, Sentry, autoupdate, Oz, or Warp account behavior.
- Non-goal: add dependencies that Warper does not currently use only because upstream updated them.
- Non-goal: create unsupported platform or shell-family work as part of this spec.
- Non-goal: remove terminal compatibility features such as inline images when they can be retained safely.

## Behavior

1. Branch names, file paths, version strings, markdown links, context chips, and agent tool inputs are treated as data, not executable shell fragments.
2. Local grep, glob, file-path checks, Git-repository checks, and display-chip actions correctly quote arguments for the shell family they execute in.
3. Environment-variable prefixes cannot bypass command blocklist decisions for AI-issued or safety-checked shell commands.
4. External-editor launches use structured argument passing where the platform supports it. They do not construct shell commands from untrusted file paths.
5. Markdown-rendered local links cannot be used to open or execute arbitrary local files through unsafe OS handler behavior.
6. OSC 52 clipboard access is disabled by default. If a user enables it, that preference is stored locally and never synced to a hosted service.
7. When OSC 52 access is blocked, Warper reports a local, Warper-branded blocked-operation state without prompting login, sync, feedback, or hosted settings flows.
8. Terminal-initiated file download escape sequences that write arbitrary files are disabled. Inline images remain supported where the parser can distinguish them safely.
9. Dependency security bumps are applied to the verified packages in Warper's dependency graph: Diesel, `rand`, OpenSSL, and `tar`.
10. The CoreText patch is updated for the macOS font descriptor leak. This is a leak fix, not a dependency-audit substitute.
11. DCS lifecycle hooks cannot be spoofed by arbitrary PTY output. Warper accepts retained shell/bootstrap hooks only when they carry the local integrity proof required by the ported implementation.
12. Restoring a local conversation cannot execute an unescaped `cd` command built from a saved path. Non-local or hosted conversation semantics remain out of scope.
13. The retained SSH command executor escapes cwd and command fragments as data before execution. This does not add remote-server installation, sharing, or hosted remote management.
14. If upstream changed cloud, update, crash-upload, remote, Windows, or hosted agent code in the same commit as a useful security fix, Warper ports only the local security slice.

## Validation

- Add or retain tests for shell quoting across the shell families Warper currently supports in retained terminal and agent-tool paths.
- Add regression tests proving environment-prefix command blocklist bypasses fail closed.
- Add markdown-link tests proving unsafe local-file opens are rejected or routed through safe local viewer behavior.
- Add OSC 52 tests covering default deny, explicit allow, blocked banner/action state, and local-only settings persistence.
- Add DCS hook tests proving raw PTY output cannot spoof lifecycle/bootstrap hooks.
- Add conversation restore tests for paths containing shell metacharacters.
- Run dependency-audit or equivalent targeted checks for Diesel, `rand`, OpenSSL, and `tar` before and after lockfile changes.
- Add or run macOS font enumeration validation for the CoreText descriptor leak fix.
- Run `./script/warper_offline_local_smoke` after porting to confirm no hosted service path was reintroduced.
