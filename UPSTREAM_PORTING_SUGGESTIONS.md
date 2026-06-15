This document is based on a local Git comparison of this Warper fork against the fetched upstream `warpdotdev/warp` history, direct `git show` inspection of candidate commits, public `gh pr view --repo warpdotdev/warp` metadata where available, and current Warper code searches with `rg`; its purpose is to identify upstream changes worth porting into Warper as a local-only terminal and OpenRouter agent, with upstream motivation kept separate from Warper relevance.

# Upstream Porting Suggestions

## Port First

These are painkillers for retained Warper paths. Port only the local slice described here; upstream hosted/Oz/cloud/telemetry/remote rollout context is not a Warper reason.

| Commit | Upstream why | Motive | Recommendation | Why Warper should care |
| --- | --- | --- | --- | --- |
| `9d9972cb` | PR `#10263` updates Diesel for GHSA/RUSTSEC SQLite UTF-8 corruption. | Painkiller | Port | Warper pins Diesel and uses it in local SQLite persistence. |
| `64a0dfbe` | PR `#10060` updates transitive `rand 0.9.1` for `GHSA-cq8v-f236-94qc`. | Painkiller | Port | Warper's lockfile contains the vulnerable transitive `rand` graph. |
| `ac091058` | PR `#10513` updates OpenSSL crates; release notes include output-buffer overflow and abort fixes. | Painkiller | Port | Warper's lockfile contains `openssl 0.10.78`. |
| `cc1ee636` | PR `#12090` updates `tar` for `GHSA-3cv2-h65g-fgmm` PAX header desync. | Painkiller | Port | Warper's lockfile contains `tar 0.4.45`. |
| `2f84587a` | PR `#9665` fixes a CoreText font descriptor leak. | Painkiller | Port | Warper uses the patched macOS CoreText stack and font descriptor enumeration. |
| `4295ec08` | Private security PR; diff replaces display-chip shell strings with typed commands and quoting tests. | Painkiller | Port manually | Display chips still build local shell actions from repo/branch/node data. |
| `7f0c4dd2` | Private security PR; diff hardens markdown local-link opening. | Painkiller | Port | Warper renders local markdown and dispatches link targets locally. |
| `43f4f483` | Private security PR; diff quotes grep and file-glob command arguments. | Painkiller | Port manually | OpenRouter exposes local `grep` and `file_glob_v2`; current executors build shell commands from user/repo input. |
| `861dacea` | Private security PR; diff removes shell construction from Linux external editor launch. | Painkiller | Port | Warper retains external editor launch for local files. |
| `0c1e2432` | Private security PR; diff strips leading env assignments before command denylist checks. | Painkiller | Port | OpenRouter exposes local shell execution; env prefixes must not bypass safety checks. |
| `b6caa957` | Private security PR; diff escapes `is_file_path` and `is_git_repository` predicate paths. | Painkiller | Port | Local grep/glob tools call those predicates with repo-controlled paths. |
| `b1a41d0b` | Private security PR; diff gates OSC 52 clipboard read/write after GHSA-class clipboard exposure. | Painkiller | Port manually | Warper parses OSC 52 and can read/write the local clipboard from terminal output. |
| `164e60e4` | Follow-up to the OSC 52 gate; adds local settings UI and blocked-operation banner. | Painkiller | Port manually | The clipboard gate needs local user control and local feedback, without cloud settings sync. |
| `f3b9ce1c` | Private security PR; diff disables non-inline iTerm file download. | Painkiller | Port | Current Warper can write terminal-provided non-inline iTerm payloads into the active cwd. |
| `32d21d15` | Private security PR; diff adds DCS hook integrity checks after spoofable lifecycle hooks. | Painkiller | Port manually | Warper still parses DCS hooks and uses shell bootstrap DCS to mutate terminal/session state; port only the local integrity mechanism. |
| `c697c8f5` | Private security PR; diff escapes `cd` during conversation restoration and skips non-local conversations. | Painkiller | Port manually | Warper's local conversation restore path still builds a `cd` command from a saved path. |
| `88c344e2` | Private security PR; diff fixes command injection in remote SSH command execution. | Painkiller | Port manually | Remote SSH command executor code still exists; port only shell escaping, not remote-server product surface. |
| `ae832ff6` | PR `#12438` fixes zsh prompt width annotations corrupting command grid rendering. | Painkiller | Port | zsh prompt rendering is retained local terminal behavior. |
| `0902e973` | PR `#12473` fixes macOS GUI shell `PATH` capture when rc files print startup output. | Painkiller | Port manually | Warper's macOS app needs reliable local tool discovery without IAP/gcloud/hosted pieces. |
| `fb3cb0e9` | PR `#9514` fixes Meta+Enter/Tab/Escape emitting literal key names. | Painkiller | Port | Terminal key encoding affects shells and local editors. |
| `388f5dc1` | PR `#12085` fixes flat-storage underflow after clear/resize/write. | Painkiller | Port | Local terminal grid crash/corruption is a retained terminal bug. |
| `fc1157e0` | PR `#9711` fixes macOS IME candidate selection receiving duplicate key handling. | Painkiller | Port | macOS is a retained Warper target and IME composition must not trigger shortcuts. |
| `3ff78d29` | PR `#9730` fixes Enter submitting a form while Japanese IME commits text. | Painkiller | Port | Prevents accidental prompt submission and dropped committed text on macOS. |
| `ab081528` | PR `#10443` refreshes IME cursor area after redraw. | Painkiller | Port | Candidate windows must track the terminal/editor cursor on retained desktop UI. |
| `6d4201ba` | PR `#12277` enables IME on X11 and explicitly avoids Wayland. | Painkiller | Port | Warper keeps Linux desktop packaging; port X11 only. |
| `802a881e` | PR `#12590` fixes diff handling for untracked directories. | Painkiller | Port | Local Git diff state feeds code-review UI and agent context. |
| `89f61b63` | PR `#11987` limits apply-diff results to changed ranges. | Painkiller | Port manually | OpenRouter exposes `apply_file_diffs`; returning whole files increases token and privacy blast radius. |
| `48331870` | PR `#12035` caps `get_repo_contents` results. | Painkiller | Port manually | Local repo reads must not materialize unbounded file trees. |
| `5fa22831` | PR `#9326` fixes `read_files` out-of-bounds ranges returning empty success. | Painkiller | Port | OpenRouter exposes `read_files`; bad ranges must be explicit. |
| `9f459842` | PR `#11856` fixes symlinked gitignored paths in code review. | Painkiller | Port | Local repo metadata and watcher state can otherwise run Git commands against wrong paths. |
| `43828a6d` | PR `#12221` avoids cloning whole file trees on view updates. | Painkiller | Port | Retained Project Explorer should not burn CPU/memory on large local repos. |
| `03ad9ea9` | PR `#12211` avoids eager lazy-subtree expansion. | Painkiller | Port | Keeps local repo metadata lazy at scale. |
| `e8024b5a` | PR `#12235` honors force-included paths in lazy metadata. | Painkiller | Port manually | WARPER-005 depends on local skills/rules staying visible even in large repos. |
| `0f97ef18` | PR `#12166` makes oversized repo metadata partial instead of failed. | Painkiller | Port manually | Large local repos should degrade, not erase useful context. |
| `3497d184` | PR `#12122` stops watching gitignored directories; upstream motive was remote-daemon heap. | Painkiller | Port manually | Warper reason is bounded local watcher load, not remote daemon health. |
| `21e70d56` | PR `#10682` avoids panic when OS watcher creation fails. | Painkiller | Port | Local watcher limits should not crash the terminal. |
| `5d8507e4` | PR `#9998` prevents fresh clones from getting stuck in loading state. | Painkiller | Port | Retained Project Explorer should recover local repo state correctly. |
| `bd7202f3` | PR `#10184` fixes file-tree refresh by rebuilding affected roots. | Painkiller | Port manually | Port local selective root refresh only; ignore remote SSH provenance. |
| `a1b76c28` | PR `#9623` fixes multiline partial-line suffix preservation in diff validation. | Painkiller | Port | OpenRouter file-edit validation can otherwise corrupt local edits. |
| `5146a5bf` | PR `#10238` fixes async rule watcher race losing active rules until restart. | Painkiller | Port | WARPER-005 depends on local rules being present in agent context. |
| `b48ece2e` | PR `#11978` removes duplicate project skill tree traversals. | Painkiller | Port | Reduces retained local skill-scan filesystem work. |
| `ac4225c1` | PR `#12040` avoids project skill refresh scans for unrelated repo updates. | Painkiller | Port manually | Reduces local skill metadata churn without adding startup product surface. |
| `92069590` | PR `#10874` caps MCP logs after reports of multi-GB logs. | Painkiller | Port manually | MCP logs are local files; cap disk growth without credential or network expansion. |
| `51c380ce` | PR `#11297` fixes MCP add-path bypassing secret redaction. | Painkiller | Port manually | Local MCP config UI must not bypass local secret/path checks. |
| `65381be1` | PR `#12540` fixes Cmd+Enter losing selected-block context when starting agent conversations. | Painkiller | Port manually | WARPER-005 is specifically about OpenRouter context fidelity; port only the context-preserving local path. |
| `3f83932c` | PR `#12254` adds format-on-save setting after unwanted LSP reformat diffs. | Painkiller | Port | Current local editor auto-formats unconditionally; users need to prevent destructive local edits. |
| `0446a507` | PR `#12313` resolves Cargo target dir in `script/macos/run`. | Painkiller | Port | Warper's dev run script hardcodes `target` paths and breaks under `CARGO_TARGET_DIR`. |
| `6eefa4bb` | PR `#9558` aligns OSS desktop `Exec` with packaged launcher. | Painkiller | Port manually | Warper Linux desktop packaging needs a deliberate Warper launcher decision. |
| `e91b5a21` | PR `#9527` adds missing Linux build deps. | Painkiller | Port | `script/presubmit` runs `clang-format`; Linux bootstrap lacks required packages. |
| `1244ffbe` | PR `#10019` avoids duplicate `.sources` entries in deb packaging. | Painkiller | Port manually | Current Warper deb script references missing repo-template paths; repair or remove Warper-specific appends instead of importing Warp repo setup. |

## Defer

These are real upstream fixes or plausible local improvements, but they are not current Warper painkillers or need a separate Warper-owned product decision.

| Commits | Reason |
| --- | --- |
| `09be9c1f` | iTerm/Kitty image paths exist, but startup inline image ordering for `fastfetch`-style output is visual compatibility, not launch reliability, security, or WARPER-005 fidelity. |
| `71edcac8`, `b7dd0ef8` | Local terminal ergonomics, not current recovery work. |
| `5bee7a75`, `59e802ea`, `2fe9d43c`, `1175e82f`, `ffe93a5e`, `1d2775ac`, `cb4fe42a` | Git/repo UI state, branch-chip command behavior, or watcher API work that is real but outside the OpenRouter file-tool and local repo data-plane critical path. |
| `163380dc`, `edfd4149`, `6289aec1` | MCP correctness that should wait for a narrow MCP credential/startup spec; do not sneak in OAuth or startup behavior. |
| `d2f26ae9`, `ee133f47` | Small local pane/tab customization not needed for current Warper recovery. |
| `35cb40c3`, `c8d39088`, `912e4540`, `3aa6026c`, `56e8617c`, `606e1653`, `8da83b42` | Markdown/Mermaid/notebook correctness or polish; no current bug/security driver strong enough to port now. |

## Skip

These are not Warper work now: dependency paths absent from Warper, Windows-only fixes without a Windows product target, hosted/Oz/cloud orchestration, local control planes, CLI-agent/plugin expansion, queueing UX, grouped tabs, and process-doc churn.

| Commits | Reason |
| --- | --- |
| `c68b9775` | Upstream `actix-http` security fix is real, but Warper has no `actix-http` package. |
| `e59c7a49`, `1df6ff13`, `d426c045`, `03ef4d05`, `2992d02e`, `ebedb9fd`, `e566a6ce`, `a7f668ea`, `f6b28f5e` | Windows/WASM/auth-log paths are absent from current Warper specs or already amputated. |
| `b5a0d89b`, `3019671e` | Global rules and startup rule re-indexing add startup/watch surface; current Warper only needs retained repo rule fidelity. |
| `95518310`, `4dddda60`, `70c725ff`, `f85d69aa`, `fd0a9d10`, `385b2a90`, `63fe7285`, `2c38e1fd`, `967a9485`, `de1ac841`, `a806bfb2`, `148e80ce`, `69ffea41` | Hosted Codex/Claude orchestration, plugin/status protocol expansion, new CLI-agent detections, or rich CLI-agent product surface. WARPER-005 does not require this. |
| `5967abf0`, `aa0a2c21` | Credentialed loopback local control plane and `warpctrl` command catalog. This is new automation surface, not a Warper painkiller. |
| `5a35550d`, `e367c9de`, `19018bf4`, `0aee45df` | Prompt or terminal command queueing changes execution semantics and are not needed for OpenRouter fidelity. |
| `26e81f9d`, `49857685`, `040a7819`, `d09a90ea`, `29394232`, `5fe27354`, `16933d3c`, `f004f417`, `ce73fe07`, `e4695f21`, `49bbe78e`, `30237218`, `d7ecfac5`, `90d214af` | UI polish, already-present behavior, remote-specific UX, or server-backed conversation features. |
| `3984e67f`, `fc110333`, `f3bfb750`, `98dbf783`, `d3757291`, `662bd737`, `e0535ca2`, `b24fce3d`, `a44fbf16`, `4598f4fb`, `af532bdc`, `7076885b`, `011d9da7` | Grouped tabs, cross-window dragging, multi-select, rollout flags, or telemetry-heavy feature bundles. |
| `467daa88`, `e7736435`, `fc1d2ff0`, `d80c0ba9`, `0510ea89`, `a30c03cb`, `3c22e421`, `57f2d4c5`, `5767910b`, `4e0b7c99`, `4ca690be`, `2113a0a3` | Developer-process skills, Oz changelog/cloud onboarding, Nix packaging, formatting wrapper, or gcloud setup. |

## Specs

- Keep `specs/WARPER-006/PRODUCT.md` for local security and dependency painkillers, expanded to include DCS integrity and conversation/SSH command escaping.
- Keep `specs/WARPER-007/PRODUCT.md` for retained terminal/input painkillers, with Windows-only fixes moved to skip/out-of-scope.
- Keep `specs/WARPER-009/PRODUCT.md` for OpenRouter file-tool and local repo metadata painkillers, narrowed to exclude Git/PR/UI polish.
- Add `specs/WARPER-010/PRODUCT.md` for local rule/skill/MCP hygiene and one OpenRouter context-fidelity fix from upstream. This spec is intentionally narrow and does not bless global rules, plugin managers, agent detections, prompt queues, or `warpctrl`.
