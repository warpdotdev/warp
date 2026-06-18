# Upstream Porting Analysis

This audit uses the repo skill `.agents/skills/upstream-change-analysis/SKILL.md` with the stricter XP rule: a port recommendation means Warper is unsafe to run, can corrupt local data/state, crashes in normal retained use, or cannot complete a current local build/package path without the change. A retained code path and a real upstream bug are necessary evidence, not sufficient rationale.

## Decision Rules

- `Port`: current Warper has the affected path, the change clears the XP necessity bar, and the useful diff does not add hosted/startup/product surface.
- `Port manually`: the local fix clears the XP necessity bar, but upstream mixed it with hosted, telemetry, remote, branding, platform, or broad refactor work.
- `Defer`: upstream fixed a real issue, but Warper does not die without it today.
- `Skip`: the path is absent, the platform is outside current Warper targets, the work conflicts with `WARPER-001`, or the motive is product expansion/churn.

## Hard Current-Code Findings

| Area | Current Warper evidence | XP effect |
| --- | --- | --- |
| Display chips | `app/src/context_chips/display_chip.rs:1713` hand-quotes `cd`; `:1717` emits raw `git checkout {branch_name}`. | Command execution exposure in retained terminal UI. Port manually. |
| OpenRouter command/search predicates | `app/src/ai/blocklist/action_model/execute.rs:1141` and `:1156` interpolate paths into shell commands. | Command execution exposure in retained local-agent path. Port. |
| Markdown local links | `app/src/notebooks/link.rs:348` emits `OpenFileWithTarget` for local markdown links. | Port only if the upstream trust gate maps to this unsafe local open path; do not port notebook polish. |
| OSC 52 | `app/src/terminal/model/grid/ansi_handler.rs:1159` sends clipboard stores, `:1175` sends clipboard loads, and `app/src/terminal/view.rs:8467-8471` touches the local clipboard. | Arbitrary terminal output reading/writing clipboard is unsafe-to-run. Port local default-deny gate. |
| iTerm file payloads | `app/src/terminal/model/terminal_model.rs:2846-2916` keeps iTerm image receive/complete paths. | Non-inline payloads writing cwd files are local data corruption. Port. |
| DCS hooks | DCS lifecycle/bootstrap hooks remain in `app/src/terminal/model/ansi/dcs_hooks.rs`; `app/src/terminal/view.rs:6657` writes InitShell DCS. | Spoofed lifecycle hooks can mutate terminal/session state. Port local integrity only. |
| Conversation restore `cd` | `app/src/terminal/view/load_ai_conversation.rs:271-273` runs `cd "{path}"`. | Saved paths can become shell commands during restore. Port local escaping. |
| Linux external editor | `app/src/util/file/external_editor/linux.rs:99` builds editor commands from desktop field codes and `:118` invokes `sh`. | File names must not be passed through `sh -c`. Port argv-safe launch. |
| Terminal flat storage | `app/src/terminal/model/grid/grid_handler.rs:310` owns flat storage; `ansi_handler.rs:805`, `:853`, and `:893` clear it. | Underflow after clear/resize/write is normal-use crash/corruption. Port. |
| OpenRouter diff validation | `app/src/ai/agent/api/openrouter.rs:529-530` exposes `apply_file_diffs`, and `app/src/ai/blocklist/action_model/execute/request_file_edits.rs:270-271` applies diffs. | Multiline diff suffix corruption can damage local files. Port. |
| Local code editor save | `app/src/code/local_code_editor.rs:984-1079` formats before saving; `:1542-1543` always calls that path. | Saving can mutate user files unexpectedly. Port format-on-save control. |
| macOS run script | `script/macos/run:28`, `:62`, and `:69` hardcode relative `target` bundle paths. | Current Warper bundle-testing workflow can build one target and launch/sign another. Port target-dir resolution. |
| Linux launcher/package scripts | `app/channels/oss/dev.warper.Warper.desktop:10` says `Exec=warp-oss`; `resources/linux/debian/app/postinst.template:4-5` creates `warp-terminal...`; `script/linux/bundle_deb:105-116` reads absent common repo templates. | Current Linux packaging can generate a launcher/package that does not work. Port manually as Warper packaging, not upstream naming. |
| Rules/skills/MCP | Local paths exist. | Existing audit did not prove current acceptance-test failure. Defer rule/skill/MCP rows. |
| Repo metadata scale | Project Explorer/repo metadata paths remain. | Useful performance/fidelity work, but not stop-ship without a failing current workflow. Defer. |
| Windows | Windows code exists. | Current WARPER specs do not target Windows. Skip. |

## Dependency And CoreText Checks

| Commit | Upstream why | Current Warper evidence | Decision | Warper reason |
| --- | --- | --- | --- | --- |
| `9d9972cb` | PR `#10263` updates Diesel because `GHSA-h5x4-m2qf-r4f2` / `RUSTSEC-2026-0111` says Diesel's SQLite backend can corrupt UTF-8; the PR says patched versions are `2.3.8+`. | `Cargo.toml:123` pins workspace Diesel at `2.3.4`; `cargo tree -i diesel` shows `app`, `persistence`, and `warp_server_client` use it; `app/src/persistence/sqlite.rs:762`, `:1209`, `:1266`, and `:2016` write local app state, project paths/rules, and command history through Diesel. | Port | This is not package-presence reasoning: Warper stores local state in SQLite through the vulnerable backend. A SQLite UTF-8 corruption bug can corrupt local Warper data, which clears the XP data-corruption bar. No new spec is needed for a patch-level dependency bump. |
| `64a0dfbe` | PR `#10060` updates transitive `rand 0.9.1` to `0.9.4` for `GHSA-cq8v-f236-94qc` / `RUSTSEC-2026-0097`; the PR says the issue is low severity, CVSS `0.0`, and requires `rand::rng()` with a custom logger. | Workspace `rand` remains `0.8.6`; `crates/managed_secrets/Cargo.toml:31` pins `rand = "0.9"` for HPKE compatibility; `cargo tree -i rand@0.9.1` shows transitive users including `mockito`, `warp_managed_secrets`, and graphics/ML dependencies. The audit did not find a custom logger path that triggers the advisory. | Defer | The vulnerable package is present, but the upstream trigger is specific and the current audit has no Warper path that hits it. Promote only for a release gate such as `cargo audit`, not as runtime survival work. |
| `ac091058` | PR `#10513` is a Dependabot update from `openssl 0.10.78` to `0.10.79`; release notes include OpenSSL binding fixes such as AES key-wrap output buffer handling. | `Cargo.lock` still contains `openssl`, but both `cargo tree -i openssl` and escalated `cargo tree --target=all --all-features -i openssl` report that no `openssl` package is resolved for the workspace. | Skip | A stale lockfile entry is not Warper code. There is no resolved OpenSSL package to port. |
| `cc1ee636` | PR `#12090` updates `tar 0.4.45` to `0.4.46`; the release notes cite `GHSA-3cv2-h65g-fgmm`, another PAX header desync fix. | `cargo tree -i tar` shows `tar` only through `crates/node_runtime`; `crates/node_runtime/src/lib.rs:205-232` downloads Node.js from `nodejs.org` and extracts the `.tar.gz`; `:282-287` calls `Archive::unpack(dest_dir)` directly. | Defer | Warper has a real remote-archive extraction path, but the current path downloads a pinned Node distribution over HTTPS, not arbitrary user-supplied tarballs. This is release-gate work or downloader-hardening work, not proven current product death. |
| `2f84587a` | PR `#9665` says `CTFontCollection::get_descriptors` leaked descriptor arrays; upstream calls out startup font enumeration, font picker usage, selected-font loading, and fallback chains for CJK/emoji. | `Cargo.toml:467-470` pins `core-foundation-rs` to the older rev; `crates/warpui/src/platform/mac/fonts.rs:82-88` calls `get_descriptors()` while loading all system fonts; `:336-358` calls it when resolving fallback descriptors. | Defer | The leak is real and the current macOS font paths are retained. The audit still has no current Warper memory-growth measurement, crash report, or release blocker showing the app dies without the bump. No implementation spec is justified from this evidence. |

## Port And Port-Manually Rows

| Commit | Upstream why | Motive | Current Warper path | Decision | Warper survival rationale |
| --- | --- | --- | --- | --- | --- |
| `4295ec08` | PR `#25398` is not publicly resolvable; commit body names display-chip RCE `GHSA-hgvx-4xvm-39pw` and says the fix adds shell quoting and restructures chip command construction. | Painkiller | Display-chip command formatting remains in `display_chip.rs`. | Port manually | Malicious repo/branch/path data must not execute shell commands from retained terminal chips. |
| `43f4f483` | PR `#25351` is not publicly resolvable; title/diff target command injection in code search tools. | Painkiller | OpenRouter search/tool predicates build shell commands from paths. | Port manually | Local-agent tools cannot safely run if repo-controlled paths become shell syntax. |
| `0c1e2432` | PR `#25258` says env-prefixed commands bypassed denylist matching. | Painkiller | `app/src/ai/blocklist/permissions.rs:831` decides autoexecution and `:852` applies the denylist. | Port | Command approval/denial is a core safety boundary for WARPER-005. |
| `b6caa957` | PR `#26138` says `is_file_path` and `is_git_repository` interpolated untrusted paths into shell commands. | Painkiller | `execute.rs:1141` and `:1156` still do that. | Port | Path classification must not execute code. |
| `7f0c4dd2` | PR `#25353` is not publicly resolvable; commit body says only trusted known-extension markdown targets should emit `OpenFileWithTarget`. | Painkiller | Notebook markdown links can emit local open events. | Port manually | Unsafe local file/app launch from rendered markdown would make local docs unsafe. Port only the gate. |
| `b1a41d0b`, `164e60e4` | PR `#25339` cites OSC 52 clipboard exposure `GHSA-wgqj-4c26-7c4g`; PR `#25625` adds local setting UI/banner. | Painkiller | OSC 52 clipboard routing is in `ansi_handler.rs:1159`, `:1175`, and `terminal/view.rs:8467-8471`. | Port manually | Arbitrary terminal output must not read/write clipboard by default. |
| `f3b9ce1c` | PR `#25261` says non-inline iTerm file payloads could overwrite cwd files. | Painkiller | iTerm image/payload handling remains. | Port | Terminal output must not write arbitrary local files. |
| `32d21d15` | PR `#25395` says arbitrary PTY output could spoof DCS lifecycle hooks. | Painkiller | DCS lifecycle/bootstrap hooks remain. | Port manually | Raw output must not mutate session/bootstrap state. |
| `c697c8f5` | PR `#25383` cites restore `cd` escaping advisory `GHSA-8659-m852-gmfx`. | Painkiller | Restore path runs `cd "{path}"`. | Port manually | Saved paths must not execute commands during local restore. |
| `861dacea` | PR `#25365` says Linux external editor used `sh -c` with untrusted field-code expansions. | Painkiller | `app/src/util/file/external_editor/linux.rs:99` builds commands and `:118` invokes `sh`. | Port | Opening a local file must not execute shell metacharacters in its name. |
| `388f5dc1` | PR `#12085` fixes flat-storage underflow after clear/resize/write. | Painkiller | `grid_handler.rs:310` owns flat storage and `ansi_handler.rs:805`, `:853`, `:893` clear it. | Port | Normal terminal output must not crash/corrupt the grid. |
| `a1b76c28` | PR `#9623` fixes multiline partial-line suffix preservation in diff validation. | Painkiller | `openrouter.rs:529-530` exposes `apply_file_diffs`; `request_file_edits.rs:270-271` applies diffs. | Port | Agent file edits must not corrupt user files. |
| `3f83932c` | PR `#12254` says always-on LSP format-on-save caused unwanted diffs. | Painkiller | Local code editor always formats before save. | Port | Saving a file must not unexpectedly rewrite user content. |
| `0446a507` | PR `#12313` and issue `#11957` say `script/macos/run` ignored Cargo's target dir and failed after `cargo bundle` wrote outside `./target`. | Painkiller | `script/macos/run` hardcodes relative target paths. | Port | Warper needs explicit target-dir control to test bundles without wrong-target/arch rebuild churn. |
| `6eefa4bb` | PR `#9558` fixes upstream OSS desktop `Exec` not matching installed launcher. | Painkiller | Warper desktop/package naming is inconsistent after rename. | Port manually | Linux packaged Warper must launch from the app menu. |
| `1244ffbe` | PR `#10019` subject says deb packaging avoids duplicate apt sources; body has no details. | Painkiller | Warper deb bundler reads absent common repo templates. | Port manually | Current deb package generation references files that do not exist. |

## Defer Rows

| Commit(s) | Upstream why | Motive | Decision reason |
| --- | --- | --- | --- |
| `64a0dfbe` | Low-severity `rand 0.9.x` custom-logger advisory. | Painkiller upstream | Present transitive package, but no current custom-logger trigger. |
| `cc1ee636` | `tar` PAX header desync. | Painkiller upstream | Real archive parser issue, but current Warper path is pinned Node download over HTTPS, not arbitrary tar input. |
| `2f84587a` | CoreText font descriptor leak. | Painkiller upstream | Current font paths call the leaking function, but no current memory-growth measurement or crash evidence clears the XP bar. |
| `88c344e2` | SSH command injection. | Painkiller upstream | Requires explicit retained-SSH product decision before remote-session work. |
| `ae832ff6`, `0902e973`, `fb3cb0e9`, `fc1157e0`, `3ff78d29`, `ab081528`, `6d4201ba` | Terminal zsh/PATH/key/IME fixes. | Painkiller upstream | Real bugs, but not stop-ship without failing retained-platform smoke tests or user reports. |
| `802a881e`, `89f61b63`, `48331870`, `5fa22831`, `9f459842`, `43828a6d`, `03ad9ea9`, `e8024b5a`, `0f97ef18`, `3497d184`, `21e70d56`, `5d8507e4`, `bd7202f3` | Local repo/file-tree scale and fidelity fixes. | Painkiller upstream | Useful, but not implementation work until a current OpenRouter workflow fails or corrupts state. |
| `5146a5bf`, `b48ece2e`, `ac4225c1`, `92069590`, `51c380ce`, `65381be1` | Rule, skill, MCP, and selected-context fixes. | Painkiller upstream | Needs a failing WARPER-005 acceptance test; otherwise it is not implementation work. |
| `e91b5a21` | Missing Linux bootstrap deps for presubmit. | Painkiller upstream | Fresh Linux convenience, not current app/build survival. |
| `09be9c1f`, `71edcac8`, `b7dd0ef8`, `5bee7a75`, `59e802ea`, `2fe9d43c`, `1175e82f`, `ffe93a5e`, `1d2775ac`, `cb4fe42a`, `163380dc`, `edfd4149`, `6289aec1`, `d2f26ae9`, `ee133f47`, `35cb40c3`, `c8d39088`, `912e4540`, `3aa6026c`, `56e8617c`, `606e1653`, `8da83b42` | Visual compatibility, ergonomics, Git UI, MCP expansion, Markdown/Mermaid polish, or optional customization. | Lip gloss or Churn | Not XP-critical. |

## Skip Rows

| Commit(s) | Upstream why | Motive | Decision reason |
| --- | --- | --- | --- |
| `c68b9775` | `actix-http` request-smuggling GHSA. | Painkiller upstream | Warper does not depend on `actix-http`. |
| `e59c7a49`, `1df6ff13`, `d426c045`, `03ef4d05`, `2992d02e`, `ebedb9fd` | Windows PTY/input/process fixes. | Painkiller upstream | Current Warper specs do not target Windows. |
| `e566a6ce`, `a7f668ea`, `f6b28f5e`, `c2954dcb` | Firebase/auth/WASM/profile security fixes. | Painkiller upstream | Paths are absent, already amputated, or outside WARPER-001 through WARPER-005. |
| `b5a0d89b`, `3019671e` | Global rules and startup re-indexing. | Recreational drug or Churn | Adds startup/watch/settings surface beyond proven repo-rule failure. |
| `95518310`, `4dddda60`, `70c725ff`, `f85d69aa`, `fd0a9d10`, `385b2a90`, `63fe7285`, `2c38e1fd`, `967a9485`, `de1ac841`, `a806bfb2`, `148e80ce`, `69ffea41` | Hosted Codex/Claude/CLI-agent/plugin/detection/rich-input work. | Recreational drug | WARPER-005 needs OpenRouter local context and tools, not upstream child-agent/plugin expansion. |
| `5967abf0`, `aa0a2c21` | Credentialed local control protocol and `warpctrl` command catalog. | Recreational drug | New loopback control plane and automation surface absent from Warper specs. |
| `5a35550d`, `e367c9de`, `19018bf4`, `0aee45df` | Prompt or terminal command queueing. | Lip gloss or Recreational drug | Changes execution semantics without solving current Warper survival pain. |
| `26e81f9d`, `49857685`, `040a7819`, `d09a90ea`, `29394232`, `5fe27354`, `16933d3c`, `f004f417`, `ce73fe07`, `e4695f21`, `49bbe78e`, `30237218`, `d7ecfac5`, `90d214af` | Conversation rename, settings palette, already-present actions, editor display preferences, markdown panels, or remote UX. | Lip gloss | Not needed now. |
| `3984e67f`, `fc110333`, `f3bfb750`, `98dbf783`, `d3757291`, `662bd737`, `e0535ca2`, `b24fce3d`, `a44fbf16`, `4598f4fb`, `af532bdc`, `7076885b`, `011d9da7` | Grouped tabs, multi-select, cross-window drag, rollout flags, telemetry. | Recreational drug | New product surface and rollout churn. |
| `467daa88`, `e7736435`, `fc1d2ff0`, `d80c0ba9`, `0510ea89`, `a30c03cb`, `3c22e421`, `57f2d4c5`, `5767910b`, `4e0b7c99`, `4ca690be`, `2113a0a3` | Process skills, visual evidence rules, Oz changelog, cloud onboarding, format wrapper, Nix, gcloud setup. | Churn or Recreational drug | Not runtime survival work; several conflict with WARPER-001. |

## Spec Impact

- `WARPER-006` must cover only unsafe-to-run local security rows from this audit.
- `WARPER-007` must shrink to terminal crash/corruption, currently `388f5dc1`.
- `WARPER-009` must shrink to local file-edit corruption, currently `a1b76c28`.
- Do not keep a WARPER spec for deferred rule/skill/MCP work. Rejected or deferred rows stay in this audit.
- A new build/package spec is justified for `0446a507`, `6eefa4bb`, and `1244ffbe` because they block current local bundle/package paths.
