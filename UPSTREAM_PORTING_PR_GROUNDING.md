# Upstream PR Grounding Check

This document records the re-audit grounding pass for `UPSTREAM_PORTING_ANALYSIS.md`, `UPSTREAM_PORTING_SUGGESTIONS.md`, and the product specs. The pass used `git show` for commit diffs, `gh pr view --repo warpdotdev/warp` for public PR bodies and linked issue references, and current Warper code searches to verify whether each claimed path exists. Private or deleted upstream PRs are identified as not publicly resolvable; their upstream why comes from commit diffs and tests only.

## Corrections Made

| Commit | Earlier problem | Correction |
| --- | --- | --- |
| `09be9c1f` | Prior text risked treating startup inline images as terminal correctness. | Classified as `Lip gloss`. Warper has iTerm/Kitty image paths, but this fixes `fastfetch`-style visual compatibility, not Warper launch reliability or OpenRouter fidelity. |
| `32d21d15` | Prior text deferred DCS integrity as startup-sensitive. | Promoted to `Port manually` because current Warper has DCS lifecycle hooks. Scope is local hook integrity only, not upstream remote/Windows/shared-session breadth. |
| `c697c8f5` | Missing from the port list. | Added as `Port manually` for local conversation-restore `cd` escaping. |
| `88c344e2` | Missing or easy to dismiss as remote fluff. | Added as `Port manually`; the useful part is shell escaping in the current SSH executor code, not remote-server product surface. |
| `e59c7a49`, `1df6ff13`, `d426c045`, `03ef4d05`, `2992d02e`, `ebedb9fd` | Windows fixes were sometimes described as deferred. | Classified as `Skip`: upstream pain is real, and Warper has some Windows code, but current Warper specs do not target Windows. |
| `b5a0d89b`, `3019671e` | Global rules and startup rule re-indexing were treated as possible local-rule work. | Classified as `Skip`: they add startup/watch/settings surface beyond the repo-rule fidelity Warper needs. |
| `5146a5bf` | Rule-race fix was mixed into vague WARPER-005 language. | Promoted to `Port`: current project-rule model has the async race path, and WARPER-005 needs reliable local rule context. |
| `fd0a9d10`, `63fe7285`, `967a9485`, `de1ac841`, `a806bfb2`, `69ffea41` | CLI-agent/plugin work was left as defer. | Classified as `Skip`: these are agent/plugin surface expansion, not required by WARPER-005. |
| `65381be1` | Cmd+Enter context loss was treated as optional agent UI. | Promoted to `Port manually`: it is directly about preserving selected-block context in a local agent turn. |
| `5967abf0`, `aa0a2c21` | `warpctrl` was sometimes softened as safe local automation. | Classified as `Skip`: credentialed loopback control plane and broad command catalog are new product surface. |
| `3f83932c` | Format-on-save setting was skipped as editor preference. | Promoted to `Port`: current local editor auto-formats unconditionally, and upstream fixed unwanted LSP reformat diffs. |
| `5fe27354`, `16933d3c`, `f004f417`, `29394232` | Already-present or low-value UI rows were kept as candidates/deferred. | Classified as `Skip` or already aligned after checking current code. |
| `59e802ea`, `e8024b5a` | Local repo rows had vague or invalid labels. | `59e802ea` moved to `Defer` because branch-chip command behavior is outside WARPER-009; `e8024b5a` remains `Port manually` for skill/rule force-includes. |
| `5bee7a75`, `2fe9d43c`, `1175e82f`, `ffe93a5e`, `1d2775ac` | Git/UI rows were over-promoted into WARPER-009. | Kept `Defer`: real upstream fixes, but not current OpenRouter file-tool or local repo data-plane painkillers. |
| `1244ffbe` | Packaging repair was skipped as upstream apt repo work. | Promoted to `Port manually`: current Warper deb script references missing repo-template paths, so the Warper-specific repair is valid. |

## Public PR Findings

| Area | Public PR evidence | Effect on Warper decision |
| --- | --- | --- |
| Dependencies | PRs `#10263`, `#10060`, `#10513`, `#12090`, and `#9665` describe concrete dependency/security/leak fixes. | Keep in `WARPER-006` only when the package is present in Warper's current graph. |
| Terminal images | PR `#10478` and linked issue `#10020` describe completed iTerm2/Kitty image actions before preexec not rendering. | Defer as `Lip gloss`; protocol paths exist, but the pain is visual compatibility. |
| Terminal/input | PRs `#12438`, `#12473`, `#9514`, `#12085`, `#9711`, `#9730`, `#10443`, and `#12277` describe local zsh, PATH, key encoding, grid, macOS IME, and X11 IME pain. | Keep in `WARPER-007`. |
| Windows terminal/input | PRs `#11906`, `#11563`, `#10442`, `#9476`, `#11714`, and `#11203` describe real Windows pain. | Skip until a Warper spec adds Windows support. |
| Rules and skills | PRs `#10238`, `#11978`, and `#12040` describe retained local-rule/skill correctness and scan reduction. PRs `#9325` and `#10377` add global/startup rule surface. | Port the narrow local rule/skill painkillers; skip global/startup expansion. |
| MCP | PRs `#10874` and `#11297` describe local log growth and add-path redaction bypass. PRs `#10640`, `#9460`, and `#9436` touch broader MCP tool/OAuth/startup behavior. | Port log rotation and redaction; defer broader MCP behavior. |
| CLI agents and plugins | PRs `#10176`, `#11571`, `#11871`, `#11892`, `#9497`, `#9667`, `#9833`, and `#11627` add or extend child-agent/plugin/detection/rich-input surfaces. | Skip; WARPER-005 does not require this product expansion. |
| Local repo/file tools | PRs `#12590`, `#11987`, `#12035`, `#9326`, `#11856`, `#9905`, `#12221`, `#12211`, `#12235`, `#12166`, `#12122`, `#10682`, `#9998`, `#10184`, and `#9623` fix retained local file-tool/repo metadata pain. | Keep in `WARPER-009`, with remote/Oz provenance stripped. |
| Git/PR UI polish | PRs `#12126`, `#11242`, `#10265`, `#9291`, and `#9238` fix real Git UI or PR workflow issues. | Defer; not current WARPER-005 file-tool correctness. |
| Local control | PRs `#11772` and `#12327` add a credentialed local control protocol and command catalog. | Skip; new control plane. |
| Editor/build/package | PRs `#12254`, `#12313`, `#9558`, `#9527`, and `#10019` describe small local editor/build/package painkillers. | Port or port manually without product-surface expansion. |
| Markdown/Mermaid/UI/process | PRs in this group mostly describe renderer UX, grouped tabs, process skills, Oz workflows, or rollout flags. | Defer or skip; no broad spec. |

## Public PR References Not Resolvable

These commit-subject references could not be fetched as public pull requests from `warpdotdev/warp`; their rationale must not cite a public PR body unless private upstream evidence is supplied later.

| Reference | Affected area |
| --- | --- |
| `#25398`, `#25353`, `#25351`, `#25365`, `#25258`, `#26138`, `#25339`, `#25625`, `#25261`, `#25395`, `#25383`, `#25354`, `#25377`, `#26090`, `#26091`, `#25311` | Security hardening commits in or near `WARPER-006`. |
| `#10596`, `#7723`, `#8863`, `#6798` | Superseded by accessible MCP follow-up PRs `#10640`, `#10874`, `#9460`, and `#9436`. |
| `#9607` | Superseded by accessible PR `#9667` for Mistral Vibe detection. |
| `#10472` | Superseded by accessible PR `#10612` for Clear context menu. |
| `#9381` | Superseded by accessible PR `#9558` for Linux desktop Exec. |

## Remaining Limits

- The high-numbered security PRs are not public. Their upstream why is grounded in commit diffs and tests, not PR descriptions.
- Several public PRs mention Oz, Sentry, remote daemon, hosted conversations, or cloud validation. That provenance is not a Warper rationale. Only retained local code paths survive the audit.
- Windows fixes are not port candidates until a Warper spec explicitly adds Windows as a supported desktop target.
