# WARPER-010: local skill, rule, MCP, and agent-context hygiene

## Summary

Warper should port a narrow set of upstream fixes that keep retained local rules, local skills, local MCP configuration, and OpenRouter context handoff reliable without adding global rule products, new CLI-agent detections, plugin managers, `warpctrl`, prompt queues, or hosted orchestration.

## Why this matters for Warper

WARPER-005 requires OpenRouter-backed agent turns to preserve local skill instructions, local rule context, local tool calls, and follow-up context. The re-audit found current Warper code paths for repo rules, project skill scans, MCP logs/config editing, and selected-block context handoff. The relevant upstream changes are painkillers only where they fix those retained paths. They are not permission to import upstream's broader agent ecosystem work.

## Source commits

| Commit | Upstream why | Motive | Current Warper evidence | Warper pain | Excluded upstream surface |
| --- | --- | --- | --- | --- | --- |
| `5146a5bf` | PR `#10238` fixes async watcher race that can lose active rules until restart. | Painkiller | `crates/ai/src/project_context/model.rs:341` and `:348` remove rule paths before async update work. | WARPER-005 depends on active local rules being present in agent context. | Global rules, startup re-indexing, cloud rules. |
| `b48ece2e` | PR `#11978` removes duplicate project skill tree traversals. | Painkiller | `app/src/ai/skills/file_watchers/skill_watcher.rs:157` and `:166` scan repo skills on metadata updates. | Duplicate scans waste local filesystem work and slow retained skill fidelity. | Remote skill catalogs and daemon catalog pushes. |
| `ac4225c1` | PR `#12040` stops unrelated 90k-file repo edits from refreshing project skills. | Painkiller | `skill_watcher.rs:157`, `skill_watcher.rs:169`, and `utils.rs:20` still refresh project skills from repo metadata events. | Unrelated repo updates should not trigger full local skill refresh scans. | Remote repo metadata behavior, startup watchers, global rule indexing, unrelated file-tree product changes. |
| `92069590` | PR `#10874` caps MCP logs after local logs reached multi-GB size. | Painkiller | `app/src/ai/mcp/logs.rs:10` writes local MCP server logs. | Local logs must not consume unbounded disk. | MCP OAuth, MCP tool exposure, startup manager behavior. |
| `51c380ce` | PR `#11297` fixes MCP +Add path bypassing local secret redaction. | Painkiller | `app/src/settings_view/mcp_servers/edit_page.rs:428` checks secrets on edit path; `:721` adds from user JSON. | Adding local MCP config paths must enforce the same local secret/path redaction boundary. | MCP OAuth, hosted config sync, tool schema expansion. |
| `65381be1` | PR `#12540` fixes selected blocks not attached when Cmd+Enter starts a new agent conversation. | Painkiller | `terminal/view/agent_view.rs:165`, `terminal/input.rs:10512`, and `terminal/view/init.rs:900` cover context-preserving agent input paths. | WARPER-005 requires local context fidelity across OpenRouter turns. | Telemetry/event-schema changes, child-agent orchestration, new agent entrypoints, CLI-agent/plugin behavior. |

## Explicitly rejected upstream work

| Commit group | Why rejected |
| --- | --- |
| `b5a0d89b`, `3019671e` | Global rules and startup re-indexing add startup/watch/settings surface beyond the retained repo-rule bugfix. |
| `163380dc`, `edfd4149`, `6289aec1` | Deferred outside WARPER-010; no port decision here. |
| `95518310`, `4dddda60`, `70c725ff`, `f85d69aa`, `fd0a9d10`, `385b2a90`, `63fe7285`, `2c38e1fd`, `967a9485`, `de1ac841`, `a806bfb2`, `148e80ce`, `69ffea41` | Hosted Codex/Claude orchestration, plugin/status protocols, agent detections, rich CLI-agent input, and auto-install flows expand product surface. |
| `5967abf0`, `aa0a2c21` | `warpctrl` is a new credentialed loopback control plane and command catalog. |
| `5a35550d`, `e367c9de`, `19018bf4`, `0aee45df` | Prompt and terminal command queueing change interaction semantics and are not required for WARPER-005. |

## Goals / Non-goals

- Goal: keep active local rules present after file updates without requiring restart.
- Goal: reduce duplicate or unrelated local skill scans.
- Goal: prevent local MCP logs from growing without bound.
- Goal: ensure MCP add-path flows use the same local secret redaction checks as edit flows.
- Goal: preserve selected local context when a user starts an agent turn with Cmd+Enter.
- Non-goal: add global rules, cloud rules, settings-synced rules, startup rule re-indexing, plugin auto-install, new CLI-agent detections, agent child orchestration, prompt queueing, terminal command queueing, or local control APIs.
- Non-goal: add MCP OAuth behavior, MCP tool exposure to OpenRouter, or MCP server startup changes in this spec.

## Behavior

1. Updating a local repo rule file cannot temporarily remove that rule from active agent context because async watcher work races with model state.
2. Project skill scans run once per relevant skill tree update, not once per duplicate traversal path.
3. Repo updates unrelated to project skills do not trigger full project skill refresh scans.
4. MCP log files have a local size cap and rotate or truncate without contacting hosted services.
5. Adding an MCP config path runs the same local secret redaction validation as editing an MCP config path.
6. Starting a new agent conversation with Cmd+Enter preserves the selected blocks or attached local context that the user intended to send.
7. None of these fixes start new background network clients, prompt for credentials at launch, auto-install plugins, or add loopback control APIs.

## Validation

- Add a rule watcher regression test where an update completes out of order and the active rule remains present.
- Add skill watcher tests proving duplicate scans and unrelated refresh scans do not run.
- Add MCP log rotation tests with a small size cap.
- Add MCP add-path tests proving secret redaction is enforced before saving.
- Add agent input/context tests proving Cmd+Enter preserves selected blocks when starting a new conversation.
- Add diff/changed-file review checks proving implementation does not introduce `warpctrl`, local control APIs, new CLI-agent detections, plugin auto-install/status protocol changes, MCP OAuth/startup behavior, or hosted telemetry paths.
- Run `./script/warper_offline_local_smoke` after implementation to confirm no hosted service path or startup prompt was added.
