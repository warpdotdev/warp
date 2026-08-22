---
name: factory-mcp
description: Work with software factories via the Warp-hosted Factory MCP server. Use when sending a task or ticket to a factory, listing factories or their tasks, checking factory task status, or handing work back to a factory foreman.
---

# factory-mcp

Software factories are automated pipelines of named agents — foreman, triage, spec, implement, review, and verify — that take tasks from intake to a verified PR. Warp hosts a Factory MCP server for working with them.

## In Warp clients

Warp auto-attaches the Factory MCP server as a built-in MCP server named `warp-factory`. No configuration is needed. For ANY factory-related work — sending a task to a factory, checking factory or task status, or handing work back — look for `warp-factory` among your available MCP servers and PREFER its tools over ad-hoc REST calls or CLI commands.

## Tools

* `list_factories`: List the software factories visible to you, including each factory's uid and agent roster. Use this first to discover factories and get the factory uid the other tools need.
* `list_tasks`: List a factory's tasks with their `factory_task_uid`, ticket metadata, current stage, and PR outputs. Use to find a task's uid or survey factory progress.
* `get_task`: Get one task's status, runs, and outputs. Pass `start_working=true` to add an active-run report plus exact local git next-actions; pass `notify_foreman=true` to send the foreman a heads-up that you're picking up the task.
* `send_task`: The ONE write path. Sending a new ticket starts a foreman intake run; sending for an existing ticket hands work back by resuming that ticket's foreman conversation. `note` is required. Supports branch and `pr_url` references plus a `stage_hint`. Prefer a plan-first flow: agree on a plan with the user before sending. Warp clients should pass `source_conversation_id` to transfer plan, file, and screenshot artifacts; other MCP clients should use the plan/context text fields instead.

## Non-Warp MCP clients

External MCP clients (Claude Code, Cursor, etc.) can connect to the same server manually:

* Endpoint (streamable HTTP): `{{warp_server_url}}/api/v1/mcp/factory`
* Auth header: `Authorization: Bearer <WARP_API_KEY>` — generate an API key in Warp settings on the `Platform` page.
