# Define team scoping for the built-in Factory MCP client

GitHub: [warpdotdev/warp#15608](https://github.com/warpdotdev/warp/issues/15608).

Originating request: [Slack thread](https://warpdev.slack.com/archives/C0BDQDW8V5E/p1787859850292039?thread_ts=1787859850.292039).

## Summary
The built-in Factory MCP connection is a cross-team control-plane connection. It does not inherit the team selected in a Warp window. Each Factory MCP tool resolves and authorizes its own explicit `factory_uid` or `team_uid`, and the built-in client does not send `X-Warp-Team-Uid`.

## Goals
- Keep Factory MCP usable across every Factory that the authenticated principal can access.
- Prevent a team selected in one window from changing another window's Factory MCP requests.
- Apply the same explicit-scope behavior in GUI, TUI, local CLI, and cloud runs.
- Preserve server-side authorization for every Factory or team named by a tool call.

## Behavior
1. The built-in Factory MCP connection is not scoped to a current Warp team.
   - The selected team in a GUI or TUI window does not filter the connection.
   - The selected team does not become an implicit default for a Factory MCP tool.
   - The selected team does not affect Factory MCP billing or authorization.

2. The built-in client sends `Authorization` to `/api/v1/mcp/factory`.
   - It does not send `X-Warp-Team-Uid`.
   - This omission is intentional, not a missing team-resolution fallback.
   - A future implementation must not add a process-global team header.

3. Two concurrent GUI or TUI windows can select different teams and use the same built-in Factory MCP connection.
   - A tool call from either window sees only resources authorized for its authenticated principal.
   - A tool call from either window gets the same cross-team discovery behavior.
   - Changing or closing one window does not respawn, retarget, or mutate the connection used by another window.

4. `list_factories` lists every Factory visible to the authenticated principal unless the tool call includes its own `team_uid` filter.
   - The active window's selected team does not supply that filter.
   - The caller can use the returned `factory_uid` and `team_uid` values in later tool calls.

5. A tool that accepts `factory_uid` authorizes that Factory against the authenticated principal.
   - The server does not trust the identifier because the client sent it.
   - An inaccessible or nonexistent Factory is not disclosed.
   - The connection's incidental server metadata cannot widen access.

6. A tool that accepts `team_uid` authorizes that team against the authenticated principal.
   - A user must be an active member of the team.
   - A service-account principal can act only within its server-pinned team.
   - A stale, unknown, or unauthorized team identifier fails without falling back to another team.

7. A local `oz agent run` uses the same cross-team Factory MCP behavior.
   - The local run does not infer a Factory MCP team from a GUI window.
   - A user with several teams is not assigned the first team for Factory MCP tool targeting.
   - The agent resolves a Factory or team through the Factory MCP tool workflow before it performs a scoped operation.

8. A cloud run uses the same cross-team Factory MCP transport.
   - The worker does not add `X-Warp-Team-Uid` to the built-in installation.
   - A Factory service-account credential remains restricted by its principal membership and Factory association.
   - A user-issued credential can access only the teams and Factories that user can access.

9. Ambiguous scope fails closed at the tool workflow boundary.
   - The client does not guess a team from window order, workspace order, or membership order.
   - The agent uses `list_factories`, `list_teams`, or the saved Factory preference defined by the Factory MCP skill.
   - The agent asks the user when the tool workflow requires a selection and no valid saved selection exists.

10. Factory work is billed and authorized from the explicit resource selected by the tool.
    - Creating a Factory uses the `team_uid` in the tool input after membership authorization.
    - Sending work uses the resolved Factory, its owning team, and its foreman identity.
    - A route-level fallback team is not an authorization decision for Factory operations.

11. Removing a user from a team takes effect on later tool calls.
    - Cached window state does not preserve access.
    - A previously returned `factory_uid` or `team_uid` does not bypass current server authorization.

12. This change adds no new user interface and no new team selector.

## Non-goals
- Add a general-purpose dynamic-header API to every MCP transport.
- Add `--team` to `oz agent run`.
- Change the Factory MCP tool schemas or the saved default-Factory workflow.
- Make a selected Warp window team filter cross-team Factory discovery.

## Open question
**Approval required:** Confirm that Factory MCP is a cross-team control-plane surface and that a window's selected team must not implicitly filter or bill Factory MCP tool calls. The alternative is a larger request-scoped transport design described in `tech.md`.
