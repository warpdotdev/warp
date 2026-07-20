# GUI-to-TUI migration matrix

Use this matrix to set expectations before reading or changing any local file.

## Already shared or available after login

No file copy is needed for:

- global and project rules discovered from shared paths;
- user and repository skills discovered from shared paths;
- bundled skills;
- Drive objects, saved prompts, and cloud execution profiles after the user logs
  in to TUI and sync completes.

Account login is still a separate TUI action. Do not describe cloud availability
as migration of local GUI data.

## Explicit local import

### Settings

Only public settings whose generated JSON Schema property has
`x-warp-surfaces` containing both `gui` and `tui` are eligible. The annotation is
authoritative and dynamic; do not maintain a setting-name allowlist.

The settings inspector may reveal only those eligible dotted paths and values.
The user approves each mutation. Missing TUI values may be added, while existing
TUI values win unless the user explicitly approves an overwrite. Preserve
comments, unknown settings, and TUI-only settings.

Permission changes deserve an explicit impact summary. Machine-local file
allowlists should be offered only when their source paths exist and are valid on
the current host.

### Global file-based MCP

The raw server definitions in the resolved GUI Warp global `.mcp.json` may be
merged into the resolved TUI Warp global `.mcp.json`. The destination wins name
conflicts. `${VAR}` and other placeholder-bearing header/environment strings are
preserved.

The helper skips source definitions that look like managed/template installation
records or that contain literal header/environment values which may be
credentials. It reports counts only. Project and third-party MCP configs are out
of scope because importing them could change scope and working-directory
behavior.

## Reauthentication or reinstallation

Handle these as setup tasks in TUI, not file migration:

- TUI account login;
- templatable or gallery MCP installations;
- MCP OAuth grants and refresh tokens;
- provider credentials, API keys, literal secrets, and secure values;
- Keychain, Windows Credential Manager, Secret Service, or other credential-store
  entries;
- MCP process running state and installation state.

If a raw file-based server is imported but needs authentication, tell the user to
authenticate it in TUI. Never copy the GUI credential.

## Unsupported

Do not migrate:

- GUI-only or private settings;
- keybindings;
- custom themes;
- launch or tab configurations;
- local workflows;
- shell and startup preferences;
- windows, tabs, panes, session state, or command history;
- GUI MCP installation or running state;
- SQLite databases or individual database rows;
- project-scoped or third-party MCP configuration in v1.

Do not invent a fallback for an unsupported category. Explain the supported TUI
setup path, if one exists, and leave the GUI source untouched.
