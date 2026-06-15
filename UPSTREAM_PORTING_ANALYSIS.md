# Upstream Porting Analysis

This is the corrected audit for commits listed in `UPSTREAM_PORTING_SUGGESTIONS.md`. The earlier version over-weighted upstream intent; this version uses Warper's current code and `WARPER-001` through `WARPER-005` as gates: port only retained local terminal/security/build behavior, defer optional local-agent and MCP work until it is on Warper's actual critical path, and skip cloud, hosted, Windows-only, and startup-expanding feature work.

## Decision Rules

- `Port`: current Warper code has the affected path today, the upstream change fixes security/correctness/build reliability, and the change does not add a cloud/startup surface.
- `Port manually`: same as `Port`, but upstream mixed the fix with hosted, telemetry, remote, branding, or broad refactor work.
- `Defer`: code exists or could be useful, but it is optional, product-significant, or not needed for the current Warper baseline.
- `Skip`: the dependency or product path is absent, outside current Warper targets, or conflicts with `WARPER-001`.

## Hard Evidence From Current Warper

| Area | Current Warper evidence | Effect |
| --- | --- | --- |
| Diesel | `Cargo.toml:123` pins `diesel = "2.3.4"`; `crates/persistence/src/model.rs`, `schema.rs`, migrations, and `app/src/persistence/*` use Diesel. | Diesel security updates are directly relevant. No conditional wording. |
| `rand` | `Cargo.toml:201` asks for `rand = "0.8.2"`; `Cargo.lock` contains `rand 0.8.5` and `rand 0.9.1`. | The upstream `rand 0.9.4` fix is relevant to the lockfile graph. |
| OpenSSL | `Cargo.lock` contains `openssl 0.10.78`. | The upstream `0.10.79` bump applies. |
| `tar` | `Cargo.lock` contains `tar 0.4.45`. | The upstream `0.4.46` bump applies. |
| `actix-http` | No `actix-http` package in `Cargo.toml` or `Cargo.lock`. | The upstream `actix-http` bump is not a Warper fix. |
| CoreText | `Cargo.toml:467-470` patches `core-foundation`, `core-graphics`, and `core-text` to servo `0bcad1e...`; `Cargo.lock` uses git `core-text 21.0.0`. | The macOS font leak bump applies to the Warper macOS build. |
| iTerm/Kitty images | `app/src/lib.rs:1770` and `1778` register `ITermImages` and `KittyImages`; `app/src/terminal/model/ansi/mod.rs:1042-1070` parses iTerm image OSC; `app/src/terminal/model/kitty.rs` implements Kitty graphics. | Warper renders terminal images, but startup inline-image ordering is a cosmetic terminal feature unless a Warper bug report makes it critical. |
| iTerm file download | `app/src/terminal/model/terminal_model.rs:2868-2879` writes non-inline iTerm image data to the active cwd with `save_as_file`. | Upstream disabling iTerm file download is a real local security fix. |
| OSC 52 clipboard | `app/src/terminal/model/ansi/mod.rs:965-966` routes clipboard load/store; `app/src/terminal/view.rs:8467-8478` writes/reads the local clipboard. | Gating OSC 52 is a real local terminal security fix. |
| DCS hooks | `app/src/terminal/model/ansi/handler.rs:239-266` exposes prompt/bootstrap hooks; `app/src/terminal/view.rs:6657` uses the init-shell DCS hook. | The spoofing class is real, but upstream commit `32d21d15` is large and startup-sensitive. Defer to a focused security task after baseline startup is stable. |
| OpenRouter local tools | `app/src/ai/agent/api/openrouter.rs:402-625` exposes local `read_files`, `grep`, `file_glob_v2`, `search_codebase`, `apply_file_diffs`, `read_skill`, and `ask_user_question`; `convert_from.rs:625-652` maps local tool calls. | Grep/glob/read/apply-diff hardening is on the current Warper path, not hypothetical. |
| MCP startup surface | `app/src/lib.rs:1167-1178` registers `FileMCPWatcher`, `FileBasedMCPManager`, and `TemplatableMCPServerManager` during app startup. | MCP work must be conservative. Fixes that reduce risk are candidates; new OAuth/plugin/gallery/interop surface is deferred. |
| MCP integer coercion | `app/src/ai/blocklist/action_model/execute/call_mcp_tool.rs:107-117` already has shallow integer coercion before local MCP tool execution. | Recursive coercion is a correctness bugfix for existing MCP, but it is not a startup priority. Defer unless MCP tool calls are exposed in the current OpenRouter slice. |
| MCP logs | `app/src/ai/mcp/logs.rs:10-12` writes per-server logs via `simple_logger`. | Size rotation is useful if MCP stays enabled; port only if it does not add startup credential/network behavior. |
| MCP OAuth | `specs/WARPER-002` says optional MCP credentials must not trigger launch prompts; `templatable_manager/oauth.rs:287-307` still has OAuth flow and Warp-branded copy. | OAuth timing fixes are not baseline. Defer until Warper MCP credential behavior is audited. |
| CLI agents | `app/src/lib.rs:1143` registers `CLIAgentSessionsModel` at startup; `terminal/view.rs:9488-9504` intercepts CLI-agent notifications. | Adding more agents/plugins/status protocols expands startup/product surface. Reject unless tied to a current Warper bug. |
| Project Explorer | `app/src/code/file_tree/view.rs:71-73` shows retained local Project Explorer copy. | Performance/crash fixes are useful; hidden-file toggles and feature expansion are not urgent. |
| Markdown/Mermaid | `Cargo.toml:169` pins `mermaid_to_svg`; `crates/editor/src/content/text.rs:733` and `app/src/ai/agent/util.rs:13` use Mermaid detection. | Renderer dependency/security/correctness fixes may apply; new notebook UX is not baseline. |
| Linux packaging | `app/channels/oss/dev.warper.Warper.desktop:10` uses `Exec=warp-oss %U`; `script/linux/bundle_appimage:61` rewrites AppImage Exec to `warp`. | Upstream `.desktop` Exec fixes need Warper-specific review, not blind porting. |
| Build deps | `script/presubmit:52-53` runs `clang-format`; `script/linux/install_build_deps` currently installs `libfontconfig1-dev` but not `clang-format`. | Linux bootstrap dependency fixes are relevant. |

## Security And Dependencies

| Commit | What upstream changed | Warper decision | Why |
| --- | --- | --- | --- |
| `4295ec08` | Replaced display-chip shell strings with typed commands and quoting tests. | Port manually | Context/display chips remain local terminal UI; this is local command execution risk, not cloud functionality. |
| `7f0c4dd2` | Hardened markdown local-link opening. | Port | Markdown rendering remains local; unsafe local file/link dispatch is a local security boundary. |
| `43f4f483` | Quoted grep and file-glob command arguments. | Port manually | OpenRouter currently exposes `grep` and `file_glob_v2`; this is on Warper's current local-tool path. |
| `861dacea` | Used argv tokenization for Linux external editor launch. | Port | External editor launch is retained local file UX; shell command construction from paths is unsafe. |
| `b1a41d0b` | Gated OSC 52 clipboard access behind a setting. | Port manually | Current terminal output can read/write local clipboard through OSC 52. Gate it locally, default closed. |
| `164e60e4` | Added OSC 52 settings UI and blocked banner. | Port manually | Required UX for the OSC 52 gate; strip cloud settings sync and upstream copy. |
| `32d21d15` | Added DCS hook integrity checks across bootstrap scripts and terminal internals. | Defer | The spoofing class is real, but the commit touches startup bootstrap broadly, including remote and Windows shell paths. Do not port during startup stabilization without a smaller Warper patch. |
| `0c1e2432` | Stripped env assignments before command blocklist checks. | Port | OpenRouter uses local shell-tool execution; env-prefix bypass is a current local permission bug. |
| `f3b9ce1c` | Disabled unsafe iTerm file download while keeping inline files/images. | Port | Current Warper writes non-inline iTerm data into cwd. That is a concrete local file-write risk. |
| `b6caa957` | Escaped `is_file_path` and `is_git_repository` predicate paths. | Port | File and repo predicates remain local agent/tool plumbing; path injection matters. |
| `9d9972cb` | Bumped Diesel for GHSA. | Port | Diesel 2.3.4 is present and used in persistence. |
| `64a0dfbe` | Bumped `rand` for GHSA. | Port | `rand 0.9.1` is present in `Cargo.lock`; update the lockfile graph rather than copying unrelated upstream churn. |
| `c68b9775` | Bumped `actix-http`. | Skip | Warper has no `actix-http` package. |
| `ac091058` | Bumped OpenSSL crates. | Port | `openssl 0.10.78` is present. |
| `cc1ee636` | Bumped `tar`. | Port | `tar 0.4.45` is present. |
| `2f84587a` | Bumped patched CoreText stack. | Port | Warper macOS build uses the affected patched CoreText stack. |

## Terminal And Input

| Commit | What upstream changed | Warper decision | Why |
| --- | --- | --- | --- |
| `09be9c1f` | Rendered startup inline iTerm/Kitty images before preexec output. | Defer | Warper renders iTerm/Kitty images, but this is about startup image ordering for programs like `fastfetch`. It does not improve Warper startup reliability or local-agent usefulness. |
| `e59c7a49` | Queued terminal response sequences under PTY backpressure. | Skip for now | Public PR body frames this as Windows PTY behavior; current Warper specs do not define Windows as a target. |
| `ae832ff6` | Fixed zsh command grid rendering glitches. | Port | zsh startup/prompt rendering is core local terminal behavior. |
| `0902e973` | Hardened interactive shell `PATH` capture against rc startup output. | Port manually | macOS GUI launch needs shell-derived PATH; noisy rc files are common and can break local tool resolution. Omit IAP/gcloud/hosted pieces. |
| `1df6ff13` | Fixed Shift+Backspace behavior. | Skip for now | Public PR body frames this as Windows Shift+Backspace behavior; no current Windows target. |
| `71edcac8` | Made Page Up/Down scroll output from prompt. | Defer | Local UX bug, but not security, startup, or local-agent critical. |
| `b7dd0ef8` | Fixed selection auto-scroll beyond bounds. | Defer | Local UX correctness; port after baseline stabilization. |
| `fb3cb0e9` | Fixed legacy Meta-key encoding. | Port | Terminal key encoding affects shell/editor correctness and is small. |
| `388f5dc1` | Fixed flat-storage row iterator underflow after clear. | Port | Local terminal crash/corruption fix. |
| `fc1157e0` | Skipped macOS key equivalents while IME composes. | Port | macOS input correctness is a retained desktop target. |
| `3ff78d29` | Prevented Enter from submitting during Japanese IME conversion. | Port | macOS input correctness; avoids accidental prompt submission. |
| `ab081528` | Refreshed IME cursor area after redraw. | Port | macOS input/rendering correctness. |
| `6d4201ba` | Enabled IME on X11. | Port | Warper has active Linux packaging and desktop resources, so X11 IME support is a retained desktop-target fix. |
| `d426c045` | Preserved Windows non-IME typed characters. | Skip | Windows is outside current Warper specs. |
| `03ef4d05` | Fixed Windows non-Latin chord shortcuts. | Skip | Windows is outside current Warper specs. |
| `2992d02e` | Ignored `REG_MULTI_SZ` env values for `CreateProcessW`. | Skip | Windows is outside current Warper specs. |
| `9d635254` | Stopped Sentry minidump server zombie process. | Skip | Sentry upload/autoupdate paths are removed by `WARPER-001`; reconsider only for a local-only crash tool. |

## Rules, MCP, Skills, And CLI Agents

| Commit | What upstream changed | Warper decision | Why |
| --- | --- | --- | --- |
| `b5a0d89b` | Added global rulefiles and active rule layering. | Defer | Local global rules fit Warper, but the upstream change includes cloud/settings surface. Do not port until WARPER-005 local skill/rule behavior is the active task. |
| `5146a5bf` | Fixed active rule update race. | Port with WARPER-005 | Active local rules support OpenRouter skill fidelity. Keep it scoped to file-backed rules. |
| `3019671e` | Re-indexed project rules on startup. | Defer | Startup work is sensitive; only port after measuring current rule indexing failure. |
| `b48ece2e` | Avoided duplicate project skill tree scans. | Port | Reduces local filesystem/startup work for retained skills. |
| `ac4225c1` | Avoided project skill refresh scans for unrelated repo updates. | Port manually | Reduces retained local metadata churn; omit remote/server assumptions. |
| `163380dc` | Recursed MCP integer coercion into nested schemas. | Defer | Current MCP coercion is shallow, but OpenRouter does not expose MCP tools today. Port when MCP tool exposure is actually enabled. |
| `92069590` | Size-rotated MCP server logs. | Port manually | MCP server logs are local files; cap disk growth without adding startup credential/network behavior. |
| `edfd4149` | Persisted MCP OAuth token timing. | Defer | OAuth-backed MCP is optional and intersects `WARPER-002` keychain-prompt risk. Audit credential timing first. |
| `6289aec1` | Queried MCP tools/resources independently by capability. | Defer | Useful compatibility work, but not current baseline and not startup-critical. |
| `51c380ce` | Prevented MCP add-path secret redaction bypass. | Port manually | Local MCP config UI exists; secret/path redaction is a retained local safety boundary. |
| `95518310` | Added hosted Codex CLI harness setup. | Skip | Hosted tasks, server metadata, snapshots, and GraphQL are outside Warper. |
| `4dddda60` | Preseeded Codex auth/trust with managed key. | Skip | Silent credential/trust writes conflict with local-first and privacy requirements. |
| `70c725ff` | Resumed Codex conversations from server transcripts. | Skip | Hosted conversation restore is explicitly out of scope. |
| `f85d69aa` | Threaded model IDs through server task metadata. | Skip | Cloud task metadata is not Warper's OpenRouter model selection. |
| `fd0a9d10` | Added local Codex child command support mixed with cloud tasks. | Defer | Do not expand local child-agent orchestration while `WARPER-001`, `WARPER-002`, and `WARPER-005` remain the active gates. Salvage only if a future spec proves Codex child control is needed. |
| `385b2a90` | Re-enabled Claude Code orchestration through hosted task plumbing. | Skip | Hosted task plumbing and orchestration are not Warper's objective. |
| `63fe7285` | Added Codex plugin/status protocol support. | Defer | Plugin/status support expands CLI-agent surface. Need a Warper-owned agent strategy first. |
| `2c38e1fd` | Auto-installed Codex orchestration plugin. | Skip | Remote plugin install and trust bypass are startup product debt. |
| `967a9485` | Recognized Goose CLI agent. | Defer | Cheap detection is still product expansion; no current Warper requirement. |
| `de1ac841` | Recognized Mistral Vibe CLI agent. | Defer | Same as Goose. |
| `a806bfb2` | Recognized Hermes CLI agent. | Defer | Same as Goose. |
| `148e80ce` | Reported harness shutdown to hosted endpoint. | Skip | Hosted/Oz lifecycle plumbing. |
| `69ffea41` | Fixed raw image paste to CLI agents. | Defer | Relevant only after Warper commits to CLI-agent rich input as a supported feature. |
| `65381be1` | Fixed Cmd+Enter starting new agent conversations. | Defer | Touches agent view/conversation routing; no current need to expand this surface. |
| `5a35550d` | Added enter-to-send for queued prompts. | Skip | Prompt queueing is a new interaction model, not a Warper recovery task. |
| `e367c9de` | Allowed terminal command queueing. | Skip | Changes terminal execution semantics; not needed. |
| `19018bf4` | Stored attachments on queued prompt model. | Skip | Depends on skipped queueing feature. |
| `0aee45df` | Fixed queued prompt paper cuts. | Skip | Depends on skipped queueing feature. |

## Repo, File Tools, And Project Explorer

| Commit | What upstream changed | Warper decision | Why |
| --- | --- | --- | --- |
| `802a881e` | Fixed diff handling for untracked directories. | Port | Local Git/diff correctness feeds current local code-review and agent context. |
| `5bee7a75` | Added diff `Detecting` state. | Defer | UI state polish; not urgent unless current false failures reproduce. |
| `89f61b63` | Limited apply-diff results to changed ranges. | Port manually | OpenRouter exposes `apply_file_diffs`; focused results improve current local edit workflows. |
| `48331870` | Capped `get_repo_contents` results and returned errors. | Port manually | Prevents oversized local repo reads. |
| `5fa22831` | Fixed out-of-bounds `read_files` ranges. | Port | OpenRouter exposes `read_files`; current `read_lines_async` omits nonexistent lines. |
| `9f459842` | Fixed symlinked gitignored paths. | Port | Local repo metadata correctness. |
| `59e802ea` | Fixed linked-worktree branch checkout. | Defer | Useful but not core unless current branch checkout bug is reproduced. |
| `2fe9d43c` | Cleared stale Git diff chip/code review button state. | Defer | UI polish; not local-agent critical. |
| `1175e82f` | Fixed branch/diff chip initialization race. | Defer | Same. |
| `ffe93a5e` | Fixed first-commit diff for commit-message generation. | Defer | Commit-message generation is not baseline. |
| `1d2775ac` | Used fork point for PR diffs and commit retrieval. | Defer | PR workflow is not part of WARPER-001 through WARPER-005 baseline. |
| `e4695f21` | Added hidden files toggle to Project Explorer. | Skip | New UX preference; not needed. |
| `43828a6d` | Avoided cloning whole file tree on view update. | Port | Retained Project Explorer can otherwise burn CPU/memory on local repos. |
| `03ad9ea9` | Avoided eager lazy subtree expansion. | Port | Local large-repo performance fix. |
| `e8024b5a` | Honored force-included paths in lazy metadata. | Port with WARPER-005 | Relevant to local skills/rules only if those paths are part of the current agent context flow. |
| `0f97ef18` | Allowed partial repo metadata builds. | Port manually | Large local repos should degrade, not fail. |
| `3497d184` | Stopped watching gitignored directories. | Port manually | Reduces local filesystem load; preserve force-included skill/rule paths. |
| `21e70d56` | Avoided panic on watcher creation failure. | Port | Local stability fix. |
| `cb4fe42a` | Updated filesystem watch filters. | Defer | Needs dependency/API review; do not churn watchers blindly. |
| `54712e5d` | Added pending SSH remote Project Explorer state. | Skip | Remote-session workflow, not Warper baseline. |
| `5d8507e4` | Avoided freshly cloned repo loading dead state. | Port | Retained Project Explorer reliability. |
| `bd7202f3` | Refreshed affected file-tree roots selectively. | Port manually | Local performance/correctness; strip remote motivation. |
| `a1b76c28` | Preserved partial-line suffix in diff validation. | Port | OpenRouter exposes `apply_file_diffs`; diff validator correctness is current. |

## User-Facing Features

| Commit | What upstream changed | Warper decision | Why |
| --- | --- | --- | --- |
| `5967abf0` | Added local control protocol, CLI, settings, credentials, packaging hooks. | Skip for now | 7,800-line new control plane with credentials and startup registration. Not needed for WARPER-001 through WARPER-005. |
| `aa0a2c21` | Added broad `warpctrl` command catalog and bundled skill. | Skip for now | Depends on skipped control plane and adds a new automation surface. |
| `3f83932c` | Added format-on-save code editor setting. | Skip for now | Useful editor preference, but not recovery/security/startup work. |
| `ce73fe07` | Added configurable code editor line numbers. | Skip for now | Local polish, not needed. |
| `26e81f9d` | Added `/rename-conversation` through server-backed path. | Skip | Server-backed conversation rename conflicts with current local-first shape. |
| `49857685` | Added conversation-list inline rename UI. | Skip | Depends on a local conversation rename product decision. |
| `040a7819` | Added command palette entries for many setting toggles. | Defer | Manual-only later; risk of resurrecting removed settings. |
| `d09a90ea` | Added keybinding for conversation details panel. | Skip | Conversation details panel is not part of current Warper baseline. |
| `d2f26ae9` | Registered Rename Active Pane as keyboard-bindable. | Defer | Small local UX; not urgent. |
| `29394232` | Surfaced Reopen Closed Session in new-session menu. | Defer | Useful but not core. |
| `ee133f47` | Added `/set-tab-color`. | Defer | Local customization; not core. |
| `5fe27354` | Prioritized copying selected input text. | Port | Small local input correctness fix. |
| `16933d3c` | Added Show in Finder to file-link tooltip. | Defer | Local convenience; not core. |
| `f004f417` | Added Clear Blocks context menu action. | Defer | Local UX; not core. |

## Markdown, Mermaid, And Notebooks

| Commit | What upstream changed | Warper decision | Why |
| --- | --- | --- | --- |
| `35cb40c3` | Added Raw/Rendered Mermaid notebook toggle. | Defer | New notebook UX, not baseline. |
| `c8d39088` | Showed Mermaid render failure callout. | Defer | Useful if Mermaid UX is prioritized, but not core recovery work. |
| `49bbe78e` | Opened Mermaid diagrams in lightbox. | Skip for now | New UX. |
| `912e4540` | Refactored visual markdown sizing for fit-width Mermaid. | Defer | Rendering polish. |
| `3aa6026c` | Bumped `mermaid_to_svg`. | Defer | `mermaid_to_svg` is present, but bumping a git renderer without a concrete bug or security reason is churn. |
| `56e8617c` | Bumped Mermaid renderer and changed theme/config behavior. | Defer | Current renderer exists, but theme/config UX is not baseline. |
| `606e1653` | Fixed Markdown ToC anchor navigation. | Defer | Local markdown correctness, lower priority. |
| `8da83b42` | Respected Markdown Viewer preference for Markdown links. | Defer | Local preference correctness, lower priority. |
| `30237218` | Applied Markdown Viewer setting in AI rules/facts panel. | Skip | Specific upstream panel path is not current Warper baseline. |
| `d7ecfac5` | Fixed markdown header alignment. | Skip for now | Pure visual polish. |
| `90d214af` | Fixed markdown rendering on remote SSH sessions. | Skip | Remote-session-heavy and outside local baseline. |

## Larger UI Features

| Commit | What upstream changed | Warper decision | Why |
| --- | --- | --- | --- |
| `3984e67f` | Added cross-window tab drag. | Skip for now | Large UI/windowing feature. |
| `fc110333` | Added tab group flag and entry points. | Skip for now | New product surface. |
| `f3bfb750` | Added vertical tab grouping. | Skip for now | New product surface. |
| `98dbf783` | Added tab group renaming. | Skip for now | Depends on skipped tab groups. |
| `d3757291` | Added horizontal tab group rendering. | Skip for now | Depends on skipped tab groups. |
| `662bd737` | Added vertical tab group drag/drop and telemetry. | Skip | Depends on skipped tab groups and includes telemetry. |
| `e0535ca2` | Added horizontal multi-tab selection. | Skip for now | Depends on grouped-tab/multi-select product decision. |
| `b24fce3d` | Added vertical multi-select grouping actions. | Skip for now | Depends on grouped-tab product decision. |
| `a44fbf16` | Added vertical multi-tab selection. | Skip for now | Depends on grouped-tab product decision. |
| `4598f4fb` | Moved grouped tabs to upstream Preview flag. | Skip | Upstream rollout flag irrelevant. |
| `af532bdc` | Added tab/group pinning actions. | Skip for now | Depends on grouped tabs. |
| `7076885b` | Fixed grouped tab restoration. | Skip for now | Depends on grouped tabs. |
| `011d9da7` | Fixed horizontal tab dragging for groups. | Skip for now | Depends on grouped tabs. |

## Developer Skills And Tooling

| Commit | What upstream changed | Warper decision | Why |
| --- | --- | --- | --- |
| `467daa88` | Added logged-out UI bug reproduction skill. | Defer | Useful process doc, but not product code and easy to write Warper-native later. |
| `e7736435` | Added change-keybinding skill. | Defer | Useful, but not a porting priority. |
| `fc1d2ff0` | Required screenshots/videos for UI-impacting PRs. | Defer | Process improvement, not recovery work. |
| `d80c0ba9` | Hardened visual-evidence review rule. | Defer | Same. |
| `0510ea89` | Required Parallelization section in tech specs. | Skip for now | Process preference; not needed. |
| `a30c03cb` | Added parent-skill prerequisite preambles. | Skip for now | Needs local skill inventory; upstream paths may be wrong. |
| `3c22e421` | Added Oz changelog draft skill and GHA. | Skip | Oz, hosted release process, and API keys. |
| `57f2d4c5` | Added cloud-agent onboarding verification. | Skip | Hosted/cloud onboarding. |
| `6984bc39` | Removed built-in feedback skill. | Already aligned | Warper already targets removal of hosted feedback surfaces. |
| `5767910b` | Added `script/format`. | Defer | Nice convention; not needed unless current formatting workflow is failing. |
| `0446a507` | Resolved Cargo target dir in macOS run script. | Port | Small local dev-run correctness fix. |
| `4e0b7c99` | Added Nix flake for Warp. | Skip for now | Packaging-sized and branding-heavy. |
| `4ca690be` | Fixed Nix build features. | Skip for now | Depends on skipped Nix flake. |
| `6eefa4bb` | Aligned `.desktop` `Exec` with packaged binary. | Port manually | Warper desktop Exec currently needs deliberate `warp-oss` vs `warp` packaging review. |
| `e91b5a21` | Added Linux bootstrap deps. | Port | `script/presubmit` uses `clang-format`; Linux bootstrap should install required local build tools. |
| `1244ffbe` | Avoided duplicate apt source entries. | Skip | Warper does not have the upstream apt repo template path. |
| `2113a0a3` | Replaced deprecated `apt-key` for gcloud install. | Skip | gcloud install path is cloud/test-infra oriented. |

## Specs Created

Only three specs survive the stricter gate:

- `WARPER-006`: local security fixes and dependency updates that map to current Warper code.
- `WARPER-007`: terminal correctness and crash/input fixes that map to current Warper targets.
- `WARPER-009`: OpenRouter local file/repo tool correctness and Project Explorer performance/stability.

## Specs Not Created

No `PRODUCT.md` files were created for these groups because they do not pass the spec gate:

- Rules/MCP/CLI-agent bundle: not a single product target. Later work must split it into narrow, evidence-backed tasks such as skill-scan reduction or MCP log rotation.
- Local control CLI: rejected for now because it is a new credentialed control plane.
- Local editor/conversation/settings UX: mostly polish, and conversation rename needs a separate local persistence design.
- Markdown/Mermaid/notebook rendering: mostly renderer UX; dependency bumps stay in the audit until there is a concrete bug or security reason.
- Developer skills/process tooling: mostly process preferences. The small build-script/package fixes stay in the audit table and do not need a product spec.
