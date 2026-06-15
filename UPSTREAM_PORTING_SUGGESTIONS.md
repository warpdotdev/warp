This document is based on a local comparison of `master` in this Warper fork against the fetched upstream ref `upstream/master` from `warpdotdev/warp`, whose fork point is commit `c0feac21` from 2026-04-29; the comparison found 175 fork-only commits and 1047 upstream-only commits as of upstream commit `09be9c1f` from 2026-06-14. Its purpose is to identify upstream changes worth considering for Warper as "Warp without the cloud fluff"; the detailed row-by-row audit is in `UPSTREAM_PORTING_ANALYSIS.md`, and this file is the resulting priority list.

# Upstream Porting Suggestions

## Port First

| Commit | Upstream title | Recommendation | Why it is relevant to Warper |
| --- | --- | --- | --- |
| `9d9972cb` | `fix: update diesel to resolve GHSA-h5x4-m2qf-r4f2` | Port | Warper currently pins Diesel 2.3.4 and uses Diesel in local SQLite persistence. |
| `64a0dfbe` | `fix: update rand to 0.9.4 to resolve GHSA-cq8v-f236-94qc` | Port | Warper's lockfile currently contains `rand 0.9.1`. |
| `ac091058` | `build(deps): bump openssl from 0.10.78 to 0.10.79` | Port | Warper's lockfile currently contains `openssl 0.10.78`. |
| `cc1ee636` | `Build(deps): Bump tar from 0.4.45 to 0.4.46` | Port | Warper's lockfile currently contains `tar 0.4.45`. |
| `2f84587a` | `Bump core-text version to fix font descriptor leak` | Port | Warper's macOS build patches and uses the affected CoreText stack. |
| `f3b9ce1c` | `[Security] Disable iterm file download, limit support to inline files` | Port | Current Warper writes non-inline iTerm data into the active cwd; terminal output should not create arbitrary local files. |
| `b1a41d0b` | `Gate OSC 52 clipboard access behind user setting` | Port | Current Warper handles OSC 52 clipboard read/write; terminal output should not silently control the user's clipboard. |
| `164e60e4` | `Add settings UI and blocked-operation banner for OSC 52 clipboard access` | Port manually | Needed UX for the OSC 52 gate, with Warper-local settings and copy only. |
| `43f4f483` | `Fix command injection in code search tools` | Port manually | OpenRouter currently exposes local `grep` and `file_glob_v2` tools. |
| `0c1e2432` | `[Security] Strip env vars before checking command blocklist` | Port | OpenRouter currently exposes local shell command execution. |
| `b6caa957` | `[Security] properly escape is_file_path and is_git_repository` | Port | Local file/repo predicates remain part of retained agent/tool plumbing. |
| `7f0c4dd2` | `Fix security vulnerability in markdown open link` | Port | Warper still renders local Markdown; unsafe local-link dispatch is a local security boundary. |
| `861dacea` | `Fix escaping issues when opening files in an external editor` | Port | External editor launch is retained local file UX. |
| `ae832ff6` | `Properly fix zsh command grid rendering glitches` | Port | zsh prompt rendering is core local terminal behavior. |
| `0902e973` | `Harden interactive shell PATH capture against rc startup output` | Port manually | macOS GUI launches need reliable shell-derived PATH; omit IAP/gcloud/hosted pieces. |
| `fb3cb0e9` | `Fix meta+enter/tab/escape sending literal key names in legacy encoding` | Port | Terminal key encoding affects shell/editor correctness. |
| `388f5dc1` | `Fix underflow in flat storage RowIterator after a clear` | Port | Local terminal crash/corruption fix. |
| `fc1157e0` | `fix(macos/ime): skip key-equivalent priority path while IME is composing` | Port | macOS input correctness on a retained desktop target. |
| `3ff78d29` | `fix(macos/ime): don't submit form when Enter confirms Japanese IME conversion` | Port | Prevents accidental prompt submission during IME composition. |
| `ab081528` | `fix: refresh IME cursor area after redraw` | Port | macOS input/rendering correctness. |
| `6d4201ba` | `Enable IME on X11` | Port | Warper has active Linux packaging and desktop resources, so X11 IME support is a retained desktop-target fix. |
| `802a881e` | `Fix diff handling for untracked directories` | Port | Local Git/diff correctness feeds current local code-review and agent context. |
| `89f61b63` | `Limit apply diff results to changed ranges` | Port manually | OpenRouter currently exposes `apply_file_diffs`; result fidelity matters. |
| `48331870` | `Add max result limit to get_repo_contents` | Port manually | Prevents oversized local repo reads. |
| `5fa22831` | `Fix read_files with out-of-bounds line ranges producing empty result` | Port | OpenRouter currently exposes `read_files`; current local line-read behavior omits missing lines. |
| `9f459842` | `Fix symlinked gitignored paths in code review` | Port | Local repo metadata correctness. |
| `43828a6d` | `Avoid cloning whole file tree on view update and flatten entries` | Port | Retained Project Explorer should not burn CPU/memory on large local repos. |
| `03ad9ea9` | `Do not eagerly expand subtrees on lazy loaded repo update` | Port | Local large-repo performance fix. |
| `0f97ef18` | `Allow partial build of repo metadata for repos exceeding max limits` | Port manually | Large local repos should degrade instead of failing. |
| `3497d184` | `Stop watching gitignored directories in the repo file watcher` | Port manually | Reduces local filesystem load; preserve explicit skill/rule force-includes. |
| `21e70d56` | `Fix panic when file watcher failed to get created` | Port | Local stability fix. |
| `5d8507e4` | `Don't get a freshly cloned repo stuck in a loading state` | Port | Retained Project Explorer reliability. |
| `bd7202f3` | `Fix file tree refresh logic` | Port manually | Local Project Explorer performance/correctness. |
| `a1b76c28` | `Fix multiline partial-line suffix preservation` | Port | OpenRouter currently exposes local diff application; validator correctness matters. |
| `b48ece2e` | `Avoid duplicate project skill tree scans` | Port | Reduces local filesystem/startup work for retained skills. |
| `ac4225c1` | `Avoid project skill refresh scans for unrelated repo updates` | Port manually | Reduces retained local metadata churn. |
| `92069590` | `fix(mcp): size-rotate MCP server log files to cap disk usage` | Port manually | MCP server logs are local files; cap disk growth without adding startup credential/network behavior. |
| `51c380ce` | `Fix MCP +Add path bypassing secret redaction check` | Port manually | Local MCP config UI remains; config path secret redaction is a local safety boundary. |
| `5fe27354` | `Fix copy keybinding to prioritize input text over selected blocks` | Port | Small retained local input correctness fix. |
| `0446a507` | `Resolve Cargo target dir in script/macos/run instead of hardcoding ./target` | Port | Small local dev-run correctness fix. |
| `6eefa4bb` | `align OSS .desktop Exec with packaged binary name` | Port manually | Warper Linux packaging should deliberately align `Exec` with the packaged launcher. |
| `e91b5a21` | `Add libclang-dev and clang-format to Linux bootstrap deps` | Port | `script/presubmit` runs `clang-format`; Linux bootstrap should install required local build tools. |

## Defer

These are potentially local but not current recovery work: `32d21d15`, `09be9c1f`, `71edcac8`, `b7dd0ef8`, `b5a0d89b`, `3019671e`, `163380dc`, `edfd4149`, `6289aec1`, `fd0a9d10`, `63fe7285`, `967a9485`, `de1ac841`, `a806bfb2`, `69ffea41`, `65381be1`, `5bee7a75`, `59e802ea`, `2fe9d43c`, `1175e82f`, `ffe93a5e`, `1d2775ac`, `cb4fe42a`, `040a7819`, `d2f26ae9`, `29394232`, `ee133f47`, `16933d3c`, `f004f417`, `35cb40c3`, `c8d39088`, `912e4540`, `3aa6026c`, `56e8617c`, `606e1653`, `8da83b42`, `467daa88`, `e7736435`, `fc1d2ff0`, `d80c0ba9`, and `5767910b`. See `UPSTREAM_PORTING_ANALYSIS.md` for the per-commit reason.

## Skip

Skip current-absent dependencies, Windows-only fixes, hosted/Oz/cloud orchestration, new local control planes, grouped-tabs feature bundles, and polish not tied to recovery: `c68b9775`, `e59c7a49`, `1df6ff13`, `d426c045`, `03ef4d05`, `2992d02e`, `9d635254`, `95518310`, `4dddda60`, `70c725ff`, `f85d69aa`, `385b2a90`, `2c38e1fd`, `148e80ce`, `5a35550d`, `e367c9de`, `19018bf4`, `0aee45df`, `54712e5d`, `5967abf0`, `aa0a2c21`, `3f83932c`, `ce73fe07`, `26e81f9d`, `49857685`, `d09a90ea`, `49bbe78e`, `30237218`, `d7ecfac5`, `90d214af`, `3984e67f`, `fc110333`, `f3bfb750`, `98dbf783`, `d3757291`, `662bd737`, `e0535ca2`, `b24fce3d`, `a44fbf16`, `4598f4fb`, `af532bdc`, `7076885b`, `011d9da7`, `0510ea89`, `a30c03cb`, `3c22e421`, `57f2d4c5`, `4e0b7c99`, `4ca690be`, `1244ffbe`, and `2113a0a3`.
