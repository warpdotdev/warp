# Upstream PR Grounding Check

This document records the follow-up verification pass for `UPSTREAM_PORTING_ANALYSIS.md` and the resulting implementation-shaped specs. The check used `git show -s` to map candidate commits to upstream PR references in commit subjects, then used `gh pr view --repo warpdotdev/warp` to fetch public PR descriptions and linked issue references where available. Public PR metadata was available for 88 PR references; 19 references from commit subjects were not resolvable as public pull requests in `warpdotdev/warp`, so those entries remain grounded in commit diffs rather than public PR bodies.

## Corrections Made

| Commit | Original problem | Correction |
| --- | --- | --- |
| `e59c7a49` | The generated terminal spec treated PTY response sequence queuing as generally relevant. The public PR body describes the bug as a Windows PTY issue. | Moved to deferred in `WARPER-007` and `UPSTREAM_PORTING_ANALYSIS.md`. |
| `1df6ff13` | The generated terminal spec treated Shift+Backspace as generally relevant. The public PR body describes the bug as Windows-specific. | Moved to deferred in `WARPER-007` and `UPSTREAM_PORTING_ANALYSIS.md`. |
| `d426c045`, `03ef4d05`, `2992d02e` | These were already changed to deferred in the previous pass, but the PR grounding check confirmed the direction: Windows behavior is not grounded in current Warper specs. | Kept deferred. |
| `43f4f483` | The generated security spec mentioned PowerShell testing even though Warper's current specs do not define Windows as a target. | Reworded to shell families Warper actually executes. |

## Public PR Body Findings

| Area | Public PR evidence | Effect on Warper decision |
| --- | --- | --- |
| Dependency hygiene | PRs `10263`, `10060`, `10513`, `12090`, and `9665` describe concrete dependency version bumps. `9665` specifically explains a macOS CoreText font descriptor leak. | Keep in `WARPER-006`, but apply only when Warper's current dependency graph contains the affected package. |
| Startup inline images | PR `10478`, linked to issue `#10020`, says iTerm2/Kitty image completions during startup or early output parsed successfully but did not render like normal output. | Keep in `WARPER-007`; this is local terminal behavior, not cloud behavior. |
| Zsh command grid rendering | PR `12438` describes zsh prompt width annotation constructs corrupting command grid rendering in Warp prompt mode. | Keep in `WARPER-007`; relevant to local zsh prompt rendering. |
| Shell PATH capture | PR `12473` explains macOS GUI launch gets a minimal launchd `PATH`, then noisy `.zshrc` output corrupts captured path data. | Keep in `WARPER-007`; relevant to Warper's macOS app launch and local tool resolution. |
| Page Up/Page Down | PR `9624`, linked to issue `#9008`, says Page Up/Down did editor navigation from the prompt instead of terminal scrollback when menus were closed. | Keep in `WARPER-007`; local terminal ergonomics. |
| Selection auto-scroll | PR `9448` says blocklist drag events were filtered by z-index coverage checks, preventing terminal selection auto-scroll. | Keep in `WARPER-007`; local terminal selection behavior. |
| Meta-key encoding | PR `9514`, linked to issue `#9517`, says Meta+Enter/Tab/Escape sent literal key names. | Keep in `WARPER-007`; local terminal input encoding. |
| Flat storage underflow | PR `12085` mentions a Sentry crash source, but the described root cause is local terminal flat storage underflow after clear/resize. | Keep in `WARPER-007`; port the local storage fix, not Sentry plumbing. |
| macOS IME | PRs `9711`, `9730`, and `10443` describe macOS IME candidate selection, Japanese conversion submission, and cursor-area alignment. | Keep in `WARPER-007`; directly relevant to macOS Warper. |
| Linux X11 IME | PR `12277`, linked to issue `#11543`, enables X11 only and explicitly avoids Wayland due to an infinite-loop issue. | Keep conditional in `WARPER-007`; only relevant if Warper keeps Linux UI builds. |
| Global rulefiles | PR `9325`, linked to issue `#9788`, says users need rules that apply across all projects, starting with `~/.agents/AGENTS.md`, but also lists cloud rules in Settings. | Evidence goes into the audit only. No spec was created because this is not yet a narrow Warper implementation target. |
| Rule and skill scan reliability | PRs `10238`, `10377`, `11978`, and `12040` describe active rule races, startup re-indexing, duplicate skill scans, and scoped refreshes. | Audit keeps the scan-reduction fixes as candidates; no broad rules/skills spec was created. |
| MCP reliability | PRs `10640`, `10874`, `9460`, `9436`, and `11297` describe schema coercion, log rotation, OAuth refresh timing, independent capability queries, and MCP add-path redaction. | Audit keeps MCP log rotation and path redaction as candidates; no broad MCP spec was created. |
| CLI agent interop | PRs `9497`, `9667`, `9833`, `11627`, and `12540` describe local CLI agent recognition, image paste behavior, and Cmd+Enter context routing. | Deferred or skipped in the audit because new CLI-agent interop expands product/startup surface. |
| Hosted Codex/plugin work | PRs `10176` and `11871` mention app.warp.dev conversations, Drive plans, cloud agents, or Codex plugin support for cloud-agent use cases. | Skipped or deferred in the audit; no hosted orchestration or plugin spec was created. |
| Git and repo metadata | PRs `12590`, `12126`, `11987`, `12035`, `9326`, `11856`, `9905`, `11242`, `10265`, `9291`, `9238`, `9532`, `12221`, `12211`, `12235`, `12166`, `12122`, `10682`, `11464`, `9998`, `10184`, and `9623` describe local diff, worktree, file-read, watcher, and project explorer behavior. Several mention Oz or Sentry context, but the changed behavior is local repo state. | Keep in `WARPER-009`, manually excluding hosted code indexing, remote SSH state, GitHub PR creation UI, telemetry, and Oz provenance. |
| Local control CLI | PRs `11772` and `12327` describe a local control protocol and a command catalog. `12327` says Auth, Drive, History, and Block families are excluded, but the catalog still needs Warper-specific narrowing. | Skipped for now in the audit; no spec was created because this is a new credentialed control plane. |
| Local editor/conversation/settings UX | PRs `12254`, `10012`, `12323`, `12409`, `11512`, and `9837` describe editor settings, local settings palette coverage, and conversation rename surfaces. `12323` explicitly renames on the server for Oz/web persistence. | Mostly skipped or deferred in the audit; no spec was created. |
| Terminal UX polish | PRs `9347`, `9305`, `9491`, `9475`, and `10612` describe reopen closed session menu entry, `/set-tab-color`, copy priority, file-link reveal tooltip, and Clear context menu. | Only copy-priority remains a port candidate; no broad UX spec was created. |
| Markdown and Mermaid | PRs `10431`, `10432`, `11548`, `12488`, `12155`, `12613`, `10143`, `9483`, and `12371` describe local Mermaid rendering, failure states, lightbox, fit-width sizing, renderer updates, ToC anchors, Markdown Viewer preference, and header alignment. | Deferred in the audit; no Markdown/Mermaid spec was created. |
| Developer skills/tooling | PRs `11424`, `9415`, `9701`, `10818`, `10400`, `11747`, `12313`, `9558`, and `9527` describe logged-out bug repro guidance, keybinding skill, visual-evidence review requirements, tech-spec parallelization, formatting script, macOS target-dir fix, Linux desktop Exec fix, and Linux bootstrap deps. | Only small build/package fixes remain candidates; no developer-process spec was created. |

## Public PR References Not Resolvable

These commit-subject references could not be fetched as public pull requests from `warpdotdev/warp` with `gh pr view`. Their rationale should not cite a public PR body unless a private upstream source is provided later.

| Reference | Affected commit area |
| --- | --- |
| `#25398`, `#25353`, `#25351`, `#25365`, `#25258`, `#26138`, `#25339`, `#25625`, `#25261`, `#25395` | Security hardening commits in `WARPER-006`. |
| `#9008` | Superseded by accessible PR `#9624` for Page Up/Page Down. |
| `#9709` | Superseded by accessible PR `#9711` for macOS IME. |
| `#10596`, `#7723`, `#8863`, `#6798` | Superseded by accessible follow-up PRs `#10640`, `#10874`, `#9460`, `#9436` for MCP fixes. |
| `#9607` | Superseded by accessible PR `#9667` for Mistral Vibe detection. |
| `#10472` | Superseded by accessible PR `#10612` for Clear context menu. |
| `#9381` | Superseded by accessible PR `#9558` for Linux desktop Exec. |

## Remaining Limitations

- Public PR metadata is not available for the high-numbered security PR references. Their current rationale is based on `git show` commit subjects and diffs, not PR descriptions.
- Several public PRs contain Oz conversation links or were generated by Oz. That does not make the local code change irrelevant by itself, but any Oz, hosted conversation, telemetry, Drive, or server persistence path is excluded from the Warper port specs.
- The current Warper specs do not establish Windows as a target. Windows-specific upstream fixes are deferred unless a later Warper spec adds Windows support.
