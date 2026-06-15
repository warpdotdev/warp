# Upstream Porting Analysis

This is the subagent re-audit of the upstream candidates listed in `UPSTREAM_PORTING_SUGGESTIONS.md`. The audit used the repo skill `.agents/skills/upstream-change-analysis/SKILL.md`: upstream why first, motive classification second, current Warper code proof third, and no port recommendation unless Warper has a retained local painkiller.

## Decision Rules

- `Port`: current Warper has the affected path, the upstream change fixes a security, correctness, crash, data-loss, dependency, or build reliability problem, and the useful diff does not add hosted/startup/product surface.
- `Port manually`: the painkiller is real, but upstream mixed it with hosted, telemetry, remote, branding, broad refactor, platform, or product-surface changes.
- `Defer`: upstream fixed a real issue, but it is optional, product-significant, startup-sensitive, lower-priority polish, or needs a separate Warper-owned design.
- `Skip`: the path is absent, the platform is outside current Warper targets, the work conflicts with `WARPER-001`, or the motive is lip gloss, recreational product expansion, or churn.

## Hard Current-Code Findings

| Area | Current Warper evidence | Effect |
| --- | --- | --- |
| Diesel | `Cargo.toml:123`, `Cargo.lock:3670`, and `crates/persistence`/`app/src/persistence` Diesel usage. | Diesel GHSA fixes are direct painkillers. |
| `rand`, OpenSSL, `tar` | `Cargo.lock` contains transitive `rand 0.9.1`, `openssl 0.10.78`, and `tar 0.4.45`. | Upstream dependency security bumps apply only for these present packages. |
| `actix-http` | `rg 'actix-http' Cargo.toml Cargo.lock app/Cargo.toml crates/*/Cargo.toml` has no hits. | Upstream `actix-http` GHSA is real but irrelevant to Warper. |
| CoreText | `Cargo.toml:467-470` patches CoreText stack; `crates/warpui/src/platform/mac/fonts.rs` enumerates font descriptors. | CoreText leak fix applies to macOS Warper. |
| iTerm/Kitty images | `app/src/lib.rs:1770-1771`, `app/src/terminal/model/ansi/mod.rs:1042`, and `app/src/terminal/model/grid/ansi_handler.rs:1263,1409` keep image protocols. | Unsafe file download is a painkiller; startup image ordering is lip gloss. |
| OSC 52 | `app/src/terminal/model/grid/ansi_handler.rs:1148` and `app/src/terminal/view.rs:8467-8479` still route clipboard load/store. | Clipboard gating and local UI are painkillers. |
| DCS hooks | `app/src/terminal/model/ansi/dcs_hooks.rs:84`, `handler.rs:239-266`, and `terminal/view.rs:6657-6663` keep lifecycle DCS hooks. | Integrity checks are a local security painkiller, but upstream remote/Windows/bootstrap breadth must be manually trimmed. |
| OpenRouter tools | `app/src/ai/agent/api/openrouter.rs:428-545` exposes local read/search/glob/apply tools. | File-tool security and result-size fixes are current WARPER-005 painkillers. |
| Local rules and skills | `crates/ai/src/project_context/model.rs:341-348`, `app/src/ai/skills/file_watchers/skill_watcher.rs:157-169`, and `app/src/ai/skills/file_watchers/utils.rs:31-46` retain repo rules and skill scans. | Rule race and scan-reduction fixes are painkillers; global rules/startup re-indexing are not. |
| MCP | `app/src/lib.rs:1167-1178`, `app/src/ai/mcp/logs.rs:10`, and `settings_view/edit_page.rs:428,721` retain local MCP managers, logs, and config UI. | Log rotation and path-redaction fixes are painkillers; OAuth/capability/startup changes are deferred. |
| Local control CLI | No `app/src/local_control`, `crates/local_control`, or `warpctrl` source path. | Upstream local control is a new control plane and skipped. |
| CLI-agent plugins/detections | CLI-agent/listener/plugin-manager code exists, but WARPER-005 does not require new child orchestration, new agent detections, or plugin auto-install. | New agent/plugin work is skipped unless a future Warper spec deliberately chooses it. |
| Project Explorer/repo metadata | Local repo tree, watcher, diff validation, and file edit paths remain. | Large local repo stability and OpenRouter file-edit correctness are painkillers; PR/commit-message/UI chip polish mostly defers. |
| Windows | Windows terminal/input code exists, but WARPER-001 through WARPER-005 do not define Windows as a target. | Windows-only upstream fixes are skipped, not “maybe relevant.” |

## Port And Port-Manually Rows

| Commit | Upstream why | Motive | Current Warper path | Decision | Warper rationale |
| --- | --- | --- | --- | --- | --- |
| `9d9972cb` | PR `#10263` updates Diesel for GHSA/RUSTSEC SQLite UTF-8 corruption. | Painkiller | Diesel is pinned and used in local SQLite persistence. | Port | Retained local persistence should not carry a known vulnerable Diesel version. |
| `64a0dfbe` | PR `#10060` updates transitive `rand 0.9.1` for `GHSA-cq8v-f236-94qc`. | Painkiller | `Cargo.lock` contains transitive `rand 0.9.1`. | Port | Present vulnerable graph, not hypothetical dependency hygiene. |
| `ac091058` | PR `#10513` updates OpenSSL after release notes called out an output-buffer overflow fix and abort fixes. | Painkiller | `Cargo.lock` contains `openssl 0.10.78`. | Port | Present native TLS dependency. |
| `cc1ee636` | PR `#12090` updates `tar` for PAX header desync GHSA. | Painkiller | `Cargo.lock` contains `tar 0.4.45`. | Port | Present archive parser dependency. |
| `2f84587a` | PR `#9665` fixes CoreText font descriptor leak. | Painkiller | Patched CoreText stack and font descriptor enumeration are present. | Port | Retained macOS app should not leak while enumerating fonts. |
| `4295ec08` | Private security diff replaces display-chip shell strings with typed commands. | Painkiller | Display chips still generate shell actions from repo data. | Port manually | Port typed/quoted local commands only. |
| `7f0c4dd2` | Private security diff hardens markdown open-link handling. | Painkiller | Local markdown/notebook link dispatch remains. | Port | Prevent unsafe local file/link launch from rendered markdown. |
| `43f4f483` | Private security diff quotes grep/glob shell arguments. | Painkiller | OpenRouter exposes `grep` and `file_glob_v2`; executors build shell commands. | Port manually | Fix command injection in retained local tools only. |
| `861dacea` | Private security diff removes shell construction from Linux editor launch. | Painkiller | External editor launch remains. | Port | Local path-to-editor launch must be argv-safe. |
| `0c1e2432` | Private security diff strips env assignments before denylist checks. | Painkiller | OpenRouter exposes shell execution; denylist checks raw commands. | Port | Prevent `FOO=bar rm ...`-style bypasses. |
| `b6caa957` | Private security diff escapes file/repo predicate paths. | Painkiller | `is_file_path`/`is_git_repository` shell predicates are still called by local tools. | Port | Repo-controlled paths must not become shell fragments. |
| `b1a41d0b`, `164e60e4` | Private security diffs gate OSC 52 and add blocked-setting UX. | Painkiller | OSC 52 clipboard load/store remains. | Port manually | Default-deny terminal clipboard access with Warper-local setting and banner. |
| `f3b9ce1c` | Private security diff disables non-inline iTerm file download. | Painkiller | Non-inline iTerm payloads can be written into cwd. | Port | Terminal output must not write arbitrary local files. |
| `32d21d15` | Private security diff authenticates DCS lifecycle hooks. | Painkiller | DCS hooks can mutate session/bootstrap state. | Port manually | Port local integrity checks only; avoid remote/Windows/shared-session breadth. |
| `c697c8f5` | Private security diff escapes restored-conversation `cd` and skips non-local conversations. | Painkiller | Local restore path builds a `cd` from saved paths. | Port manually | Prevent local restore command injection; do not restore hosted conversation semantics. |
| `88c344e2` | Private security diff fixes SSH command injection. | Painkiller | Remote SSH command executor code still exists. | Port manually | Escape shell construction only; no remote product expansion. |
| `ae832ff6`, `0902e973`, `fb3cb0e9`, `388f5dc1` | Public PRs fix zsh grid corruption, noisy shell `PATH` capture, Meta-key bytes, and flat-storage underflow. | Painkiller | Local terminal bootstrap, key encoding, and grid storage remain. | Port or Port manually | Core local terminal behavior and crash resistance. |
| `fc1157e0`, `3ff78d29`, `ab081528`, `6d4201ba` | Public PRs fix macOS IME and X11 IME behavior. | Painkiller | macOS and Linux UI paths remain; X11/Wayland split exists. | Port | Native text input must work on retained desktop targets; X11 only for Linux IME. |
| `802a881e`, `89f61b63`, `48331870`, `5fa22831`, `9f459842`, `43828a6d`, `03ad9ea9`, `e8024b5a`, `0f97ef18`, `3497d184`, `21e70d56`, `5d8507e4`, `bd7202f3`, `a1b76c28` | Public PRs fix local diff, file-tool, repo metadata, watcher, and file-tree failure modes. | Painkiller | OpenRouter file tools and local Project Explorer/repo metadata remain. | Port or Port manually | WARPER-005 needs reliable local file reads/edits/search and bounded local repo metadata. |
| `5146a5bf`, `b48ece2e`, `ac4225c1` | Public PRs fix rule watcher race and duplicate/unrelated skill scans. | Painkiller | Local rules and skill watchers remain. | Port or Port manually | WARPER-005 depends on local skill/rule fidelity without excess filesystem churn. |
| `92069590`, `51c380ce` | Public PRs cap MCP log size and fix add-path secret-redaction bypass. | Painkiller | Local MCP logs and config UI remain. | Port manually | Local disk and local config safety; no OAuth/startup expansion. |
| `65381be1` | PR `#12540` preserves selected-block context on Cmd+Enter new agent conversations. | Painkiller | Current input path can bypass the context-preserving agent-view path. | Port manually | Directly supports WARPER-005 context fidelity. |
| `3f83932c` | PR `#12254` adds format-on-save setting to stop unwanted LSP reformat diffs. | Painkiller | Current local code editor auto-formats unconditionally. | Port | Users need local control over destructive save behavior. |
| `0446a507`, `6eefa4bb`, `e91b5a21`, `1244ffbe` | Public PRs fix dev run target dir, Linux desktop launcher, Linux build deps, and deb repo-source duplication. | Painkiller | Warper scripts/packaging paths have matching local issues. | Port or Port manually | Local build/package reliability without hosted packaging behavior. |

## Defer Rows

| Commit(s) | Upstream why | Motive | Decision reason |
| --- | --- | --- | --- |
| `09be9c1f` | PR `#10478` renders startup iTerm/Kitty images like `fastfetch`. | Lip gloss | Warper has image protocols, but this is visual compatibility, not launch reliability, security, or WARPER-005. |
| `71edcac8`, `b7dd0ef8` | Local terminal scroll/selection ergonomics. | Lip gloss | Real UX fixes, but not recovery/security/startup work. |
| `5bee7a75`, `59e802ea`, `2fe9d43c`, `1175e82f`, `ffe93a5e`, `1d2775ac`, `cb4fe42a` | Git UI state, branch-chip command behavior, commit-message, PR diff, or watcher API fixes. | Painkiller upstream | Current paths exist, but these are outside current OpenRouter file-tool/repo data-plane priorities or need dependency review. |
| `163380dc`, `edfd4149`, `6289aec1` | MCP nested integer coercion, OAuth token timing, and capability querying. | Painkiller upstream | MCP exists, but these touch tool exposure, OAuth/keychain timing, or startup manager behavior; require a narrow MCP spec first. |
| `d2f26ae9`, `ee133f47` | Pane rename keybinding and tab color slash command. | Lip gloss | Local customization, not current Warper pain. |
| `35cb40c3`, `c8d39088`, `912e4540`, `3aa6026c`, `56e8617c`, `606e1653`, `8da83b42` | Mermaid/Markdown correctness or rendering polish. | Lip gloss or Churn | Keep out until a concrete Warper bug/security reason exists. |

## Skip Rows

| Commit(s) | Upstream why | Motive | Decision reason |
| --- | --- | --- | --- |
| `c68b9775` | `actix-http` request-smuggling GHSA. | Painkiller upstream | Warper does not depend on `actix-http`. |
| `e59c7a49`, `1df6ff13`, `d426c045`, `03ef4d05`, `2992d02e`, `ebedb9fd` | Windows PTY/input/process fixes. | Painkiller upstream | Windows code exists, but current Warper specs do not target Windows. |
| `e566a6ce`, `a7f668ea`, `f6b28f5e`, `c2954dcb` | Firebase/auth/WASM/profile security fixes. | Painkiller upstream | Paths are absent, already amputated, or outside WARPER-001 through WARPER-005. |
| `b5a0d89b`, `3019671e` | Global rules and startup re-indexing. | Recreational drug or Churn | Adds startup/watch/settings surface beyond current repo-rule fidelity need. |
| `95518310`, `4dddda60`, `70c725ff`, `f85d69aa`, `fd0a9d10`, `385b2a90`, `63fe7285`, `2c38e1fd`, `967a9485`, `de1ac841`, `a806bfb2`, `148e80ce`, `69ffea41` | Hosted Codex/Claude/CLI-agent/plugin/detection/rich-input work. | Recreational drug | WARPER-005 needs OpenRouter local context and tools, not upstream child-agent/plugin expansion. |
| `5967abf0`, `aa0a2c21` | Credentialed local control protocol and `warpctrl` command catalog. | Recreational drug | New loopback control plane and automation surface absent from Warper specs. |
| `5a35550d`, `e367c9de`, `19018bf4`, `0aee45df` | Prompt or terminal command queueing. | Lip gloss or Recreational drug | Changes execution semantics and queue UI without solving current Warper pain. |
| `26e81f9d`, `49857685`, `040a7819`, `d09a90ea`, `29394232`, `5fe27354`, `16933d3c`, `f004f417`, `ce73fe07`, `e4695f21`, `49bbe78e`, `30237218`, `d7ecfac5`, `90d214af` | Conversation rename, settings palette, already-present actions, editor display preferences, markdown panels, or remote UX. | Lip gloss | Not needed now; several are already present locally or depend on server/remote surfaces. |
| `3984e67f`, `fc110333`, `f3bfb750`, `98dbf783`, `d3757291`, `662bd737`, `e0535ca2`, `b24fce3d`, `a44fbf16`, `4598f4fb`, `af532bdc`, `7076885b`, `011d9da7` | Grouped tabs, multi-select, cross-window drag, rollout flags, telemetry. | Recreational drug | New product surface and rollout churn. |
| `467daa88`, `e7736435`, `fc1d2ff0`, `d80c0ba9`, `0510ea89`, `a30c03cb`, `3c22e421`, `57f2d4c5`, `5767910b`, `4e0b7c99`, `4ca690be`, `2113a0a3` | Process skills, visual evidence rules, Oz changelog, cloud onboarding, format wrapper, Nix, gcloud setup. | Churn or Recreational drug | Not product runtime painkillers; several conflict with WARPER-001. |

## Specs Created

- `WARPER-006`: retained local security fixes and dependency updates, expanded by the re-audit to include DCS integrity and conversation/SSH command escaping.
- `WARPER-007`: retained terminal/input painkillers, with Windows-only rows explicitly skipped.
- `WARPER-009`: OpenRouter file-tool and local repo metadata painkillers, narrowed away from Git/PR/UI polish.
- `WARPER-010`: local rule/skill/MCP hygiene and one OpenRouter context-fidelity fix. This is not a local-agent expansion spec.

## Specs Not Created

- No spec for `warpctrl` or local control CLI. It is a credentialed control plane absent from the current code and specs.
- No spec for global rules or startup rule re-indexing. The retained pain is repo rule fidelity, not more startup watchers.
- No spec for new CLI-agent detections, plugin auto-install, child orchestration, or prompt/terminal queueing.
- No spec for grouped tabs, tab pinning, cross-window drag, or multi-select feature bundles.
- No spec for Markdown/Mermaid polish or process-skill churn.
