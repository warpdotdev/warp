# File-based MCP config edit actions — Product Spec
GitHub issue: https://github.com/warpdotdev/warp-external/issues/9882
Figma: none provided

## Summary
File-based MCP server cards in Warp settings should let users jump directly to the local configuration file that defines the server. The edit affordance on an active file-based server and the `Edit config` action on an errored file-based server should both open the relevant `.mcp.json`, `.claude.json`, `.codex/config.toml`, or `.agents/.mcp.json` file in Warp's editor.

## Problem
Warp already detects MCP servers from local config files and shows them in MCP settings, but the cards do not consistently expose a working way back to the source file. Spawned file-based servers suppress the hover edit icon, and errored file-based servers can render an `Edit config` button that routes to the normal MCP edit page instead of opening the backing config file. This makes error recovery harder because the user can see which file-based server failed but must manually locate the config file before fixing it.

## Goals
- Active, starting, authenticating, shutting down, offline, and errored file-based MCP cards expose a config-edit action whenever Warp knows the defining local config file.
- Clicking that action opens the defining config file in Warp's editor, matching the project-rules `Open file` experience as closely as the settings surface allows.
- The error-state `Edit config` text button and the active-card hover edit icon use the same target resolution and failure handling.
- The behavior works for all supported file-based MCP providers: Warp, Claude, Codex, and Other Agents.
- The behavior is deterministic when a deduplicated file-based server is referenced by more than one supported config file.

## Non-goals
- Editing the parsed MCP server through Warp's templatable MCP edit form. File-based servers remain source-of-truth-local-file objects; this change opens the file instead of converting them into editable cloud-backed templates.
- Adding a picker for duplicate definitions. If the same server is detected from multiple files, Warp chooses one deterministic config file to open.
- Creating missing config files from the MCP card. The action opens a file Warp has detected; if the file disappears before the click is handled, Warp reports that it could not open the config.
- Changing file-based MCP discovery, parsing, spawning, OAuth, sharing, deletion, or auto-spawn behavior.
- Changing the visual design of MCP server cards beyond enabling the existing edit affordance for file-based cards where appropriate.

## Behavior
1. A file-based MCP server card that is promoted into the installed/running section shows the same hover edit icon placement used by other editable MCP cards, provided Warp can resolve at least one local config file that defines that server.
   - This applies while the server is running, starting, authenticating, shutting down, offline, or in any other installed-card state that already has the icon action row.
   - The icon tooltip remains the existing edit wording unless the implementation already has a more specific `Edit config` tooltip for this control.

2. When a file-based MCP server is in an error state and the card renders the `Edit config` text button, clicking the button opens the same file target as the hover edit icon would open for that server.

3. The file opened by the action is the provider config file that contains the file-based MCP server definition:
   - Warp provider, project-scoped: `<project root>/.warp/.mcp.json`
   - Warp provider, global: Warp's managed global MCP config file, displayed to users as the global Warp MCP config.
   - Claude provider, project-scoped: `<project root>/.mcp.json`
   - Claude provider, global: `~/.claude.json`
   - Codex provider, project-scoped: `<project root>/.codex/config.toml`
   - Codex provider, global: `~/.codex/config.toml`
   - Other Agents provider, project-scoped: `<project root>/.agents/.mcp.json`
   - Other Agents provider, global: `~/.agents/.mcp.json`

4. Clicking an enabled file-based config edit action opens the file in a Warp editor pane, not in the templatable MCP edit page and not in the system file explorer.
   - The editor-opening behavior should respect the same in-app editor preferences used by other local file-opening actions where practical.
   - The action should be visible from the MCP settings page without requiring the user to manually copy or browse to the path.

5. If the active window already has editor panes open, the config file opens using Warp's normal file-opening layout behavior for local files. This change does not introduce custom split placement, custom tabs, or special MCP-only editor layout rules.

6. If the same deduplicated server is referenced by multiple config files, the edit action opens the first deterministic target from the set of provider/root references Warp has tracked for that server.
   - The ordering must be stable across repeated renders in the same app version.
   - Opening one deterministic file is acceptable even when title chips show more than one provider or scope.
   - A future picker or per-chip action is out of scope.

7. If Warp cannot resolve any defining config file for the file-based server, the card must not expose a file-edit affordance that silently does nothing.
   - For icon actions, the icon should be hidden when no file target is known.
   - For error-state actions, if existing card state requires showing the `Edit config` text button before target resolution can be confirmed, clicking it must show an error toast rather than doing nothing.

8. If Warp resolves a config path but the file is gone, unreadable, or otherwise cannot be opened by the time the user clicks the action, Warp shows a non-blocking error toast explaining that the config file could not be opened.
   - The server card remains visible.
   - The server running/error state is not changed by the failed open action.
   - The action may become available again after file-based MCP discovery refreshes and re-detects a valid config file.

9. The edit action is local-only behavior. It is available only in builds and environments where Warp can open local filesystem paths in its editor. Non-local-filesystem builds should continue to compile and should not show a broken action.

10. Existing non-file-based MCP card behavior is unchanged.
    - Templatable MCP edit icons still open the MCP edit/setup page.
    - Gallery cards still install from the gallery.
    - View logs, logout, running toggles, sharing, setup, and update actions keep their current behavior.

11. File-based template cards in the `Detected from <provider>` sections continue to use their current primary action to start the detected server. This change does not require adding an edit icon to not-yet-spawned template-style cards unless the implementation can do so without changing the card's primary start action.

12. Opening a file-based MCP config does not parse, validate, modify, or save the config by itself. Any config changes happen only after the user edits and saves the file in Warp's editor; subsequent file-based MCP watcher behavior remains responsible for re-parsing and refreshing cards.

13. The action target must never be derived from user-visible card text such as server title, description, error text, or title chip labels. The opened path must come from Warp's tracked file-based MCP discovery metadata.

## Success Criteria
1. A running file-based MCP server detected from a supported config file shows an edit affordance, and clicking it opens that config file in Warp's editor.
2. An errored file-based MCP server's `Edit config` button opens the same defining config file instead of navigating to a non-working edit page.
3. Warp, Claude, Codex, and Other Agents file-based servers resolve to their provider-specific project/global config paths.
4. Duplicate references resolve deterministically and do not produce a no-op.
5. Existing templatable, gallery, logs, logout, and running-toggle MCP actions are not regressed.
