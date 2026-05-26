---
name: warpctrl
description: Use the Warp Control CLI to inspect or control a running local Warp app during dogfood. Use when operating Warp product surfaces through warpctrl, checking local-control availability, targeting a running Warp instance, or evaluating planned warpctrl commands against the implemented catalog.
user-invocable: true
---

# Warp Control CLI

Use `warpctrl` to operate an already-running local Warp app through the approved local-control command surface. Prefer native tools for code editing, local file content reads or writes, ordinary shell execution, web requests, and MCP calls.

Use `warpctrl` when the task is about Warp product surfaces: windows, tabs, panes, sessions, input staging, visible app state, settings surfaces, Warp Drive views, and permissioned Drive object actions.

## Current workflow

1. List compatible running instances:

   ```bash
   warpctrl --output-format json instance list
   ```

2. Inspect the selected app's implemented action catalog:

   ```bash
   warpctrl --output-format json action list --instance <instance_id>
   ```

3. Target one instance explicitly for automation:

   ```bash
   warpctrl --output-format json tab create --instance <instance_id>
   ```

4. Use JSON output for scripts and agent workflows. Treat parser errors and structured errors as boundaries, not as a reason to bypass the catalog.

## Implemented areas

The dogfood stack implements metadata reads, selected underlying-data reads, layout mutations, input staging, session navigation, allowlisted setting/theme mutations, surface opens/toggles, file/project app-state opens, Drive metadata/app-state opens, Drive object create/update/delete/insert/share-to-team, and `input run` with additional execution-policy approval.

Always verify the selected app build advertises an action as implemented before relying on it. `drive workflow run` remains a stub in this stack.

## Permission posture

- Metadata reads require read-metadata authority.
- Terminal content, input buffer, history, and Drive content reads require underlying-data read authority.
- Visible app changes such as tab, pane, surface, file open, project open, and Drive open operations require app-state mutation authority.
- Setting and theme writes require metadata-configuration mutation authority.
- Drive object writes and `input run` require underlying-data mutation authority.
- Drive actions and `input run` also require authenticated-user authority where catalog metadata says so.
- `input run` is fail-closed unless explicit local execution policy approval is active.

## File and project boundaries

`warpctrl` does not provide local file content commands. Do not use or invent commands such as `file read`, `file write`, `file append`, or `file delete`.

The approved file/project scope is app-state only:

- `file open <path>` opens a file in Warp's visible editor surface.
- `file list` lists files currently open in Warp editor state.
- `project open`, `project list`, and `project active` operate Warp project/workspace state.

Use the agent's native file tools or ordinary shell commands for local filesystem content reads, writes, appends, and deletes.

## Warp Drive sharing boundaries

Warp Drive sharing v0 has two paths:

- `drive object share open <id>` opens the share dialog for user review without changing sharing state.
- `drive object share-to-team <id>` is the only direct native sharing mutation. It makes a personal object available to the current user's team and requires authenticated-user plus underlying-data-mutation authority.

Do not use `warpctrl` for arbitrary ACL editing, external sharing, named-user sharing, public links, accepted-command submission, agent-prompt submission, or arbitrary internal dispatch.

## Handling unsupported commands

When a command is in the product spec but not implemented in the selected app build, report it as planned or unsupported. Expected boundary errors include `unsupported_action`, `not_allowlisted`, `insufficient_permissions`, `authenticated_user_required`, `authenticated_user_unavailable`, and `execution_context_not_allowed`.
