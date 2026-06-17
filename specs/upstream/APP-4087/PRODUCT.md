# APP-4087: Fix Warp skill and MCP home config paths after watcher unification Product Spec
## Summary
The immediate regression is that `oz agent run --skill <name>` no longer finds Warp skills stored in Warp’s home-relative config directory on Linux and Windows. APP-3945 intentionally centralized Warp-owned filesystem watching to avoid recursively watching `.warp*/worktrees`, but it also tied Warp skill lookup to platform app data paths. That works on macOS because Warp’s app data path is home-relative and channel-aware, but it breaks non-macOS users because app data follows XDG/AppData conventions instead of Warp’s `.warp*` home config directory convention.
Fixing the skill regression led us to audit other Warp-owned home config paths affected by the same watcher unification. MCP has the same shape: Warp’s file-based MCP config should live next to other home-relative Warp config for the current app environment. APP-4087 should restore those environment-aware home paths without undoing APP-3945’s worktree-watch safety.
## Problem
Before APP-3945, Warp skill discovery could find skills in Warp’s home config directory, such as `~/.warp/skills`. After APP-3945, `SkillProvider::Warp` stopped going through generic home-provider watching and instead relied on the centralized Warp watcher and app data paths. That prevented broad recursive watches under `~/.warp`, which was intended, but it also meant that home-relative Warp skill directories stopped being considered when app data was somewhere else.
Linux and Windows expose the bug because their app data directories differ from Warp’s home-relative `.warp*` config directories. A Stable user can therefore have a valid skill at `~/.warp/skills/foo/SKILL.md`, run `oz agent run --skill foo`, and see the skill fail to resolve because the resolver and watcher are looking in the platform app data directory.
macOS hides most of this because Stable and Preview commonly resolve app data to `~/.warp`, while Dev and Local resolve to environment-specific home directories such as `~/.warp-dev` and `~/.warp-local`. The desired behavior is not simply “always use `~/.warp`”; it is “use the home-relative Warp config directory for the current Warp app environment.”
While investigating Skills, we checked MCP because its global config has the same shape. Warp MCP config should use the same environment-aware home config directory as Skills, e.g. `~/.warp/.mcp.json`, `~/.warp-dev/.mcp.json`, or `~/.warp-local/.mcp.json` depending on channel/profile.
## Goals
- Preserve the APP-3945 invariant that Warp does not recursively watch `.warp*/worktrees`.
- Restore `oz --skill <name>` resolution for Warp home skills on Linux, Windows, and macOS.
- Preserve environment isolation for Dev, Local, Integration, OpenWarp, and development profiles.
- Use a single purpose-specific home config path helper for Warp Skills and MCP.
- Keep `data_dir()` and `config_local_dir()` for their existing app-managed configuration responsibilities.
- Keep Warp-specific filesystem watching centralized instead of reintroducing ad hoc recursive watchers in `SkillWatcher` or `FileMCPWatcher`.
## Non-goals
- Changing the public skill file format or MCP config schema.
- Changing project-level skill or MCP discovery.
- Migrating existing files between platform app data directories and home-relative `.warp*` directories.
- Treating non-macOS XDG/AppData `data_dir()/skills` or `data_dir()/.mcp.json` as Warp Skills or MCP sources of truth.
- Changing non-Warp provider paths such as `~/.agents/skills`, `~/.claude/skills`, `~/.codex/config.toml`, or project provider paths.
- Introducing a generic filtering API in `repo_metadata::DirectoryWatcher`.
## Figma / design references
Figma: none provided.
## User Experience
### Warp home skills
- A Stable user can store a skill at `~/.warp/skills/<skill-name>/SKILL.md`.
- Dev, Local, Integration, OpenWarp, and profiled builds use their own home-relative Warp config directories, such as `~/.warp-dev/skills`, `~/.warp-local/skills`, or `~/.warp-local-<profile>/skills`.
- Running `oz agent run --skill <skill-name> ...` resolves the skill from the current app environment’s Warp home skills directory even when platform app data is elsewhere.
- Warp home skill resolution continues to take precedence over project skill resolution for unqualified skill names.
- The resolver must not require the asynchronous `SkillManager` cache or filesystem watcher to be warmed before `oz --skill` works.
### Warp home MCP config
- A user can configure file-based MCP servers for the current Warp app environment at `<warp-home-config-dir>/.mcp.json`.
- Examples include `~/.warp/.mcp.json`, `~/.warp-dev/.mcp.json`, and `~/.warp-local/.mcp.json`.
- When the MCP file is created, edited, moved into place, or deleted, Warp updates detected file-based MCP servers without requiring a restart, as long as the relevant parent path is watchable.
- Warp MCP config is scoped as a user-level config, not as a project config or platform app-data config.
### Worktree exclusion
- Activity under `.warp*/worktrees` must not trigger reloads for themes, workflows, tab configs, settings, skills, or MCP config.
- Supporting Warp home skills must not be implemented by recursively watching all possible `.warp*` directories.
- Supporting Warp home MCP config must not be implemented by recursively watching all possible `.warp*` directories.
### Existing app paths
- Channel-aware app files under `data_dir()` continue to work as before for non-Skills/MCP app config.
- `data_dir()` remains the root for channel-scoped themes, workflows, launch configs, tab configs, and other app-managed files.
- `config_local_dir()` remains the root for platform-specific config files such as `settings.toml`, `keybindings.yaml`, and `user_preferences.json`.
## Success Criteria
- `oz agent run --skill <name>` can resolve `<warp-home-config-dir>/skills/<name>/SKILL.md` from a cold start.
- Skill resolution still finds project skills when no matching Warp home skill exists.
- Warp home skills still take precedence over project skills for unqualified skill names.
- File-based MCP detection includes `<warp-home-config-dir>/.mcp.json` as a user-scoped Warp provider config when present.
- Dev/Local/Profiled builds use isolated `.warp*` home config directories instead of Stable’s `~/.warp` directory.
- No code path treats non-macOS XDG/AppData `data_dir()/skills` or `data_dir()/.mcp.json` as Warp home Skills or MCP sources.
- No code path reintroduces a generic recursive watcher rooted at `~/.warp`.
- `.warp*/worktrees` changes remain excluded from Warp-managed reload behavior.
## Validation
- Add or update unit coverage for `oz --skill` resolving a skill from an explicit Warp home skills directory.
- Add or update unit coverage for Warp home config path helper behavior.
- Add or update unit coverage for MCP path classification so `<warp-home-config-dir>/.mcp.json` is recognized as a Warp MCP config path.
- Add or update watcher helper tests to verify managed Skills/MCP helpers return the current environment’s Warp home paths.
- Add or update skill utility tests so only the current environment’s Warp home skills directory is classified as home Warp skills.
- Run targeted Rust tests for path helpers, skill resolution, skill file watcher utilities, MCP provider/path helpers, and Warp managed path filtering.
## Alternatives Considered
- Use only hardcoded `~/.warp` for Skills/MCP. Rejected because it loses Dev/Local/Profile environment isolation.
- Keep using `data_dir()` for Skills/MCP. Rejected because non-macOS app data paths are XDG/AppData paths, not Warp’s home-relative `.warp*` config paths.
- Use `config_local_dir()` for Skills/MCP. Rejected because non-macOS config-local paths are also platform project directories, not home-relative `.warp*` paths.
- Add only a resolver fallback for `oz --skill`. Rejected because it fixes cold CLI resolution but leaves app hot reload, skill discovery, and MCP path behavior inconsistent.
- Re-add Warp to generic home-provider watchers. Rejected because that watcher shape can recursively watch `.warp*` parents and reintroduce `.warp*/worktrees` churn.
- Watch all of `~/.warp` recursively and filter in consumers. Rejected because it recreates the broad watcher shape APP-3945 was designed to remove.
## Open Questions
- Should Warp proactively create the current environment’s Warp home config directory on startup, or only watch it when it already exists? The implementation should prefer the least invasive approach unless product explicitly wants fresh installs to create these paths.
