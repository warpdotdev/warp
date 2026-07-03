# Post-reconciliation upstream porting analysis

This audit only covers upstream commits after the previous reconciliation in `specs/upstream/2026-06-17`. The baseline is upstream commit `09be9c1f`, so the audited range is `09be9c1f..upstream/master`.

## Git facts

- Current fork head: `f46f2049` (`master`, `origin/master`, `warper-v0.4.2`).
- Upstream head fetched for this pass: `7ae929bc` (`upstream/master`).
- Audited upstream range: `09be9c1f..7ae929bc`.
- Upstream commits in range: 299.
- Local post-reconciliation fork fixes checked: `bf0bc87b`, `8d61c3fe`, `d4df0291`, `1ddd3fae`, `78aaa9e1`.

## Plain-English result

The first version over-recommended ports. The branch only carries fixes that were locally reproduced or accepted as narrow, retained-path bugfixes:

- Ported in `bugfix/upstream-reconciliation-2026-07-03`: `f1816928`, `c682422f`, `e082f0b9`, `11f6f4a9`.
- Defer unless you hit the exact workflow: `2e518448`, `4ececc2d`, `73ab280e`, `a59c4b8a`.
- Skip for current use: `6903db03`, `abf98bff`, `974bd763`, `098c307c`.
- Already covered locally: `ec29aefa`, and the earlier diff-suffix and flat-storage classes covered by local commits `1ddd3fae` and `d4df0291`.

## Ported in this branch

| Upstream commit | Local commit | What actually breaks | Why it matters in Warper | Validation used |
| --- | --- | --- | --- | --- |
| `f1816928` Prevent RowIterator crash after clear resize truncates wide-character spacer | `e7dc2187` | A normal terminal row can contain a wide character at the right edge. If the grid is cleared and resized so the spacer half is chopped off, the retained cell still says "I am a wide char". Later flat-storage materialization writes the missing spacer at `idx + 1` and can panic out of bounds. | This is terminal grid state, not Warp agent behavior. Current Warper still shrinks rows in `app/src/terminal/model/grid/grid_storage/resize.rs`, and row materialization still writes wide-character spacers through `crates/warp_terminal/src/model/grid/flat_storage/row_iterator.rs`. | Added the split-wide-character resize regression in `app/src/terminal/model/grid/grid_handler_test.rs` and ran the focused terminal-grid test set before committing. |
| `c682422f` Fix markdown delimiter counter overflow | `6663f7eb` | Markdown with 256 or more repeated `*`, `_`, or `~` can overflow an 8-bit delimiter count. Debug builds can panic; release builds can tokenize the delimiter run incorrectly. | This is retained Markdown rendering, not Warp agent orchestration. Warper still uses `crates/markdown_parser/src/markdown_parser.rs` for Markdown rendered in local UI surfaces. | Added the long-delimiter regression in `crates/markdown_parser/src/markdown_parser_test.rs`; changed the delimiter counter type in `crates/markdown_parser/src/markdown_parser.rs`; ran the markdown parser tests before committing. |
| `e082f0b9` Skip SSH warpify when a host has `RemoteCommand` | `afe370cf` | OpenSSH rejects a command-line remote command when the selected host config also has `RemoteCommand`, with `Cannot execute command-line and remote command.` Warper's shell integration wrapper normally passes its bootstrap as a command-line remote command. | This is retained SSH shell integration, not the built-in Warp agent. Users with `RemoteCommand` in `~/.ssh/config` cannot use that host through the wrapper unless Warper backs off and lets OpenSSH run the configured remote command. | Added the guard to bash, zsh, and fish bootstrap scripts under `app/assets/bundled/bootstrap`; verified the generated command behavior manually before committing. |
| `11f6f4a9` Avoid reversed file read range on first-line truncation | `86ce71b2` | If a file read has a byte budget smaller than the first selected line, the accumulator can report a reversed range such as `2..1` or drop the empty truncated segment. | This is retained local file tooling. OpenRouter exposes `read_files` in `app/src/ai/agent/api/openrouter.rs`, maps it to local agent actions, and the executor reaches `crates/warp_files/src/text_file_reader.rs`. Bad range metadata can confuse follow-up file-context reasoning even when file contents are empty. | Added first-line truncation regressions in `crates/warp_files/src/text_file_reader_tests.rs`; fixed `TextFileAccumulator::flush_range`; ran `cargo test -p warp_files truncated_on_first`, `cargo test -p warp_files`, `cargo clippy -p warp_files --all-targets --all-features --tests -- -D warnings`, `cargo fmt --check`, and `git diff --check`. |

## Defer

These are real upstream fixes, but the plain-English impact is conditional or too weak to justify porting before there is a local repro or release task.

| Commit | What actually breaks | Why it is not a port now |
| --- | --- | --- |
| `2e518448` standalone macOS CLI resources dir | A standalone macOS CLI artifact expects resources next to the binary in `$OUT_DIR/resources`, while current `bundled_resources_dir()` still returns `<bundle>/Contents/Resources`. | Defer to release packaging. This matters only if building or shipping the standalone macOS CLI artifact. It does not affect the normal app bundle. |
| `4ececc2d` linuxdeploy AppImage runtime | The current pinned linuxdeploy can produce AppImages that depend on `libfuse2`. Fedora 44 and similar modern distros no longer provide that by default, so the AppImage may not launch there. | Defer to Linux release packaging. It matters if you ship AppImage artifacts. It does not affect local macOS development or the app source. |
| `73ab280e` dump-debug-info path escaping | If the Warper executable path contains spaces or shell metacharacters, the "Dump debug info" command palette action pre-fills a broken shell command because `app/src/workspace/view.rs:4873-4878` interpolates the path raw. | Defer. It is a diagnostic command the user still has to run manually. Fix when touching diagnostics or command prefill escaping. |
| `a59c4b8a` MCP secret-redaction toggle | Current MCP server editing blocks saves whenever secret detection finds a secret in `app/src/settings_view/mcp_servers/edit_page.rs:428-445`, even if the user intended to disable redaction. | Defer. This is only relevant to local MCP settings. If MCP is not part of the current workflow, porting it risks settings churn for no immediate payoff. |

## Skip for current use

These are the rows most likely to create the "fix bugs for hours" outcome because they belong to Warp's built-in agent stack or CLI-agent integration, not plain terminal use.

| Commit | What actually breaks upstream | Why to skip |
| --- | --- | --- |
| `6903db03` requested-command approval crash | In Warp's built-in Agent Mode, reject a requested shell command with `Ctrl-C`, then press Enter while the stale approval card still has focus. Debug builds hit `debug_assert!(false, "Expected action to be requested command.")`. | Skip unless you actively use Warper's built-in Agent Mode command approval UI. This is not Codex. It does not affect normal terminal use. |
| `abf98bff` CLI-agent permission title cleanup | Claude Code `AskUserQuestion` can leave a stale "Wants to run AskUserQuestion..." title after `ToolComplete`, because current CLI-agent session state does not clear permission-scoped summary fields. | Skip unless you rely on Warper's CLI-agent session title integration. It is cosmetic state leakage, not a terminal crash or file corruption. |
| `974bd763` LRC pre-snapshot queued query race | In Warp's built-in agent flow, sending a follow-up query while an agent-requested long-running shell command is pending can produce duplicate tool results for the same tool-use id. | Skip for current use. This is built-in agent queue semantics, not Codex. The upstream patch touches queue model and UI behavior, which is exactly the risky area to avoid without a local failing workflow. |
| `098c307c` queued `/compact-and` circular update crash | In Warp's built-in agent flow, a queued `/compact-and` can dispatch summarization while `TerminalView` is already updating and trip WarpUI's circular update guard. | Skip for current use. `/compact-and` queued prompt behavior belongs to Warp's agent UI. Do not port it unless that feature becomes a deliberate Warper target. |

## Already covered locally

| Commit | Status |
| --- | --- |
| `ec29aefa` pane_leaves sqlite cleanup | Current `app/src/persistence/sqlite.rs:568-584` already deletes `pane_leaves` during app-state save cleanup. |
| Earlier diff-suffix corruption class | Local `1ddd3fae` added the fork-side file-diff suffix fix and tests. |
| Earlier flat-storage resize-after-clear class | Local `d4df0291` fixed a separate flat-storage resize-after-clear bug. It does not cover `f1816928`. |

## Skip groups

- Hosted Warp, Oz, cloud task, account, billing, onboarding, telemetry, workload identity, Drive sync, remote Agent Mode, and computer-use-video commits: conflict with WARPER-001/WARPER-005 scope.
- TUI commits: new product surface, not current Warper work.
- Tab grouping, pinning, cross-window drag, tab color, and visual polish commits: not local terminal survival work.
- Windows-only commits: not a current Warper release target.
- `/crates/warp_graphql_schema` npm dependency bumps: the directory is absent in this fork.
- `openssl` lockfile bump: `cargo tree -i openssl` does not resolve an `openssl` package in this workspace.

## Specs created

No new product spec was created. The accepted ports were narrow bugfix commits with local regression coverage or explicit manual validation, so they were tracked in this audit instead of a product spec.

## Specs not created

No negative spec was created for skipped hosted, TUI, tab-grouping, Windows, dependency-stale, or built-in-agent work. Those decisions stay in this audit.
