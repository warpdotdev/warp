# WARPER-006: upstream local security fixes

## Summary

Warper should selectively port upstream security fixes that protect local terminal use, local file opening, local agent tools, and local dependency hygiene without reintroducing hosted Warp services or cloud-shaped settings.

## Why this matters for Warper

Warper's value is that a user can run a local terminal and a local-first agent without trusting Warp-hosted infrastructure. That makes local execution boundaries more important, not less: branch names, file paths, markdown links, and agent tool arguments come from local repositories and terminal output that may be untrusted. These fixes are relevant only where they protect retained local behavior: local terminal prompts, local markdown/notebook rendering, local file opening, local agent shell tools, and local dependency code that is still in Warper's build. They are not relevant as generic upstream security churn, and they should not be used to justify cloud settings sync, hosted crash reporting, or unsupported platform work.

## Source commits

| Commit | Resolution | Scope |
| --- | --- | --- |
| `4295ec08` | Port manually | Replace display-chip command strings with typed shell commands and shell-specific quoting. |
| `7f0c4dd2` | Port | Harden markdown local-link opening so rendered markdown cannot launch arbitrary local files through OS defaults. |
| `43f4f483` | Port manually | Quote grep and file-glob arguments for shell families Warper actually executes. Do not add unsupported shell-family work in this spec. |
| `861dacea` | Port | Launch external editors on Linux through argv tokenization instead of shell command construction. |
| `0c1e2432` | Port | Strip leading environment assignments before local command blocklist checks. |
| `b6caa957` | Port | Quote paths in local `is_file_path` and `is_git_repository` command predicates. |
| `b1a41d0b` | Port manually | Gate OSC 52 clipboard read/write escape sequences behind a local setting that defaults closed. |
| `164e60e4` | Port manually | Add local settings UI and blocked-operation feedback for OSC 52. |
| `f3b9ce1c` | Port | Disable unsafe iTerm file downloads while retaining inline image support. |
| `32d21d15` | Defer | The DCS spoofing class is real, but the upstream commit touches shell bootstrap startup broadly, including remote and Windows shell paths. Do not port during startup stabilization without a smaller Warper patch. |
| `9d9972cb` | Port manually | Update Diesel. Warper currently pins Diesel 2.3.4 and uses it in local SQLite persistence. |
| `64a0dfbe` | Port manually | Update `rand`. Warper's lockfile currently contains `rand 0.9.1`. |
| `ac091058` | Port manually | Update OpenSSL crates. Warper's lockfile currently contains `openssl 0.10.78`. |
| `cc1ee636` | Port manually | Update `tar`. Warper's lockfile currently contains `tar 0.4.45`. |
| `2f84587a` | Port manually | Update patched `core-text` stack. Warper's macOS build currently patches `core-text` to servo `0bcad1e...`. |

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
9. Dependency security bumps are applied to the verified packages in Warper's dependency graph: Diesel, `rand`, OpenSSL, `tar`, and patched `core-text`.
10. DCS hook integrity is not part of this implementation pass. It requires a smaller Warper-specific patch or a separate security task after startup behavior is stable.
11. If upstream changed cloud, update, crash-upload, remote, Windows, or hosted agent code in the same commit as a useful security fix, Warper ports only the local security slice.

## Validation

- Add or retain tests for shell quoting across the shell families Warper currently supports in retained terminal and agent-tool paths.
- Add regression tests proving environment-prefix command blocklist bypasses fail closed.
- Add markdown-link tests proving unsafe local-file opens are rejected or routed through safe local viewer behavior.
- Add OSC 52 tests covering default deny, explicit allow, blocked banner/action state, and local-only settings persistence.
- Run dependency-audit or equivalent targeted checks for Diesel, `rand`, OpenSSL, `tar`, and `core-text` before and after lockfile changes.
- Run `./script/warper_offline_local_smoke` after porting to confirm no hosted service path was reintroduced.
