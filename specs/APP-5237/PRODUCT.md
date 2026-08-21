# APP-5237: Agent Plugins 1.0.0 in Warp and Factories

## Summary
Warp will load Agent Plugins 1.0.0 packages that provide Agent Skills and MCP servers. The desktop app and TUI will use the same package format and discovery roots. File-managed Factories will support factory-, agent-, and automation-scoped plugins from their source repository.

This proposal also moves Factory MCP declarations from YAML/frontmatter toward separate Warp-defined `mcp.json` files. A Factory `mcp.json` is not an Agent Plugins artifact. It can declare managed Warp MCP servers and ordinary MCP servers, while a plugin `mcp.json` remains strictly conformant to Agent Plugins 1.0.0.

Agent Plugins 1.0.0 is the published specification at [agent-plugins.org](https://agent-plugins.org/specification). The similarly named open-plugins.com format is not in scope.

## Goals
- Support the complete Agent Plugins 1.0.0 portable core: root `plugin.json`, `skills/<name>/SKILL.md`, and root `mcp.json`.
- Give the desktop app and TUI the same discovery, validation, scoping, naming, and failure behavior.
- Preserve the current execution behavior of existing Warp skills and file-based MCP servers.
- Add plugins to file-managed Factories without introducing a remote plugin installation protocol.
- Let Factories declare managed and ordinary MCP servers in entity-level `mcp.json` files outside plugin packages.
- Make scope and conflicts deterministic across user, repository, factory, agent, and automation sources.

## Figma
Figma: none provided. The implementation will follow the existing Skills and MCP settings patterns.

## Behavior

### Client discovery and scope

1. Warp treats each immediate child directory of a plugin search root as one plugin candidate. Warp does not recursively search for nested plugin candidates.

2. Warp scans these user search roots:
   - `<home>/.agents/plugins/`.
   - The current build's Warp home config directory plus `/plugins/`. Stable uses `<home>/.warp/plugins/`. Other channels and data profiles use their existing channel-aware Warp home config directory.

3. Warp scans these repository search roots for the detected repository that contains the active working directory:
   - `<repo-root>/.agents/plugins/`.
   - `<repo-root>/.warp/plugins/`.

4. Warp does not scan a bare `<repo-root>/plugins/` directory for client plugins.

5. GUI and TUI scan the same plugin package roots. The separate TUI global MCP config file does not create a separate TUI plugin package root.

6. A plugin is in project scope only while its repository is in scope under the existing skill rules:
   - A local session uses the detected repository that contains the active working directory.
   - A cloud session with multiple configured repositories can use plugins from all configured repositories.
   - If equally ranked repositories provide the same plugin name, Warp reports an ambiguity instead of selecting by filesystem order.

7. Same-name plugins shadow as a complete package. Warp does not combine the manifest from one package with skills or MCP from another package. Precedence from highest to lowest is:
   1. Repository `.agents/plugins/`.
   2. Repository `.warp/plugins/`.
   3. User `.agents/plugins/`.
   4. User Warp home config `plugins/`.

8. The `.agents`-before-`.warp` order matches the existing skill-provider order. A shadowed plugin remains visible in plugin diagnostics with the source that won.

9. Adding, changing, moving, or deleting a candidate causes Warp to update its active plugin package set without restarting. An invalid update does not execute new components.

### Package validation and conformance

10. Warp loads a plugin only when its filesystem-resolved root contains a regular root `plugin.json` whose `$schema` is the canonical Agent Plugins 1.0.0 manifest schema.

11. Warp uses locally bundled schemas. Warp does not fetch a schema URL while loading a plugin.

12. Warp applies the Agent Plugins 1.0.0 manifest rules exactly:
   - Unknown top-level manifest fields are reported and ignored.
   - An `extensions` value with the wrong type is reported and ignored.
   - Other manifest schema violations reject the complete plugin.
   - Unsupported extension namespaces are ignored without inspecting their contents.

13. Warp resolves every package file before use. A path, symlink, junction, or equivalent filesystem reference that resolves outside the plugin root is rejected at the narrowest failure boundary required by the standard.

14. A missing `skills/` directory or `mcp.json` file is not an error.

15. A malformed skill disables only that skill. A malformed top-level MCP configuration disables MCP only for that plugin. A malformed or failed MCP server disables only that server when the standard defines a server-level failure boundary.

16. Warp reports plugin, component, and connection diagnostics with the plugin name, source scope, source path, and actionable reason:
   - Valid skills appear in the existing GUI and TUI skill lists.
   - Valid MCP servers and their connection failures appear in the existing GUI and TUI MCP surfaces.
   - An explicit invocation of an invalid or ambiguous component returns the reason and valid candidates.
   - Package-level parse, unsupported-component, and shadowing diagnostics that have no valid component are written to structured client logs.
   - Diagnostics never include secret values.

### Skills and qualified names

17. A valid plugin skill uses the fixed path `skills/<skill-directory>/SKILL.md`. Warp scans only immediate skill directories, as required by Agent Plugins 1.0.0.

18. Plugin skills use the same Agent Skills parser, working-directory scope, automatic discovery, model context, and invocation permissions as existing repository and user skills. Discovering a skill does not add a new approval step.
   - Loading a skill never launches a process on the plugin's behalf.
   - A skill can instruct the agent to run a bundled file such as `skills/<name>/scripts/check.sh`.
   - That file runs only through the agent's normal shell-command action. The active execution profile, allowlist, denylist, risk classification, and approval behavior apply.
   - A skill-directed command is not managed by the MCP lifecycle and does not automatically receive `PLUGIN_ROOT` or `PLUGIN_DATA`.

19. Warp gives each plugin skill the qualified component name `<plugin-name>:<skill-name>`. Example: the `deploy` skill in plugin `acme-tools` is `acme-tools:deploy`.

20. The qualified skill name appears in:
   - The skill catalog sent to the model.
   - The GUI and TUI skill lists.
   - Explicit user or model invocation, for example `/acme-tools:deploy`.
   - Ambiguity and validation messages.
   - Factory agent or automation prompt text that explicitly invokes the skill.

21. An unqualified skill name remains usable when exactly one active skill has that name. If a flat skill and a plugin skill, or two plugin skills, share the name, Warp requires a qualified name and lists the candidates. A plugin never silently replaces a flat skill.

22. The source `SKILL.md` frontmatter keeps its portable Agent Skills `name`. Warp's qualified name is runtime identity metadata; Warp does not rewrite the package on disk.

### Plugin MCP servers

23. A plugin can declare MCP servers only in root `mcp.json` with the canonical Agent Plugins 1.0.0 MCP schema. Warp does not read plugin MCP configuration from `plugin.json`, `.mcp.json`, or another path.

24. Warp supports the Agent Plugins `stdio` and `streamable-http` transports in v1. Warp skips the optional legacy `sse` transport with an unsupported-transport diagnostic.

25. Warp validates the closed schema for each MCP server and enforces the standard's URL, header, executable-token, working-directory, containment, environment, and placeholder rules. A `cwd` that contains a literal `..` path segment is invalid when `mcp.json` is parsed, before any launch attempt. A name that only contains `..` as a substring remains valid.

26. Warp expands only `${PLUGIN_ROOT}` and `${PLUGIN_DATA}`, and only in stdio `args`, `env` values, and `cwd`. Warp does not expand them in `command`, URL fields, headers, environment keys, or fixed component paths.

27. Each plugin MCP server has the qualified component name `<plugin-name>:<server-name>`. A native MCP tool keeps the tool name returned by its server.

28. The model and UI identify a tool as `<plugin-name>:<server-name>/<tool-name>`. Example: server `github` in plugin `acme-tools` can expose `acme-tools:github/create_issue`. The MCP wire call still sends native tool name `create_issue` to the server, and Warp routes it by the stable server identity.

29. Qualified MCP server names appear in:
   - The MCP settings page and TUI MCP screen.
   - Tool metadata supplied to the model.
   - Tool-call permission and execution details.
   - Connection, authentication, and conflict diagnostics.

30. Existing MCP execution semantics apply based on discovery scope and provider root:
   - A repository plugin MCP server never starts only because Warp discovered it. It appears as a project-scoped file-based server and requires the existing explicit start action.
   - A user `.agents/plugins` MCP server follows existing global third-party file-based MCP behavior. The GUI's existing file-based MCP setting controls automatic start. The TUI requires an explicit start.
   - A user Warp-home plugin MCP server follows existing global Warp file-based MCP behavior. The TUI still defers startup until its existing post-login activation point.
   - Stopping, retrying, authenticating, and viewing logs use the existing file-based MCP controls.

31. Warp adds one user-level `Agent Plugin discovery` preference. It is a global kill switch for plugin packages, not an inventory or a per-plugin control.
   - The default is enabled.
   - The preference applies to every user and repository plugin source in the interactive client. Users cannot configure it separately by source, repository, package, skill, or MCP server.
   - The desktop app shows one toggle under Settings > Warp Agent > Plugins. The widget is gated at page-build time by the Agent Plugins feature flag and remains visible while the preference is disabled.
   - The settings widget search terms include `agent`, `plugin`, `plugins`, `discovery`, `skills`, `MCP`, `disable`, and `stop`.
   - The command palette shows `Disable Agent Plugin discovery` while enabled and `Enable Agent Plugin discovery` while disabled. Both the settings row and command use the same persisted preference.
   - GUI and TUI read the same setting key from their existing frontend-specific settings profiles. The value uses existing user settings synchronization when synchronization is enabled. The TUI adds no setting screen or command in v1.
   - Factory workers ignore this personal preference. Factory plugin discovery is part of the applied Factory definition and remains controlled by Factory source and rollout gates.

32. Turning discovery off takes effect in the active interactive frontend without a restart:
   - Warp stops plugin filesystem watchers and does not scan any new package.
   - Warp withdraws all plugin skills from the model catalog and explicit invocation resolver for subsequent turns.
   - Warp cancels in-flight plugin MCP tool calls, stops plugin MCP connections and stdio server processes, and unregisters their installations from existing MCP surfaces.
   - An explicit reference to a withdrawn plugin component fails with an `agent_plugin_discovery_disabled` diagnostic.
   - Warp preserves plugin directories and `PLUGIN_DATA`.
   - Warp does not terminate an ordinary shell command that a plugin skill caused the agent to start before discovery was disabled. That command uses the normal shell-command lifecycle and is not a plugin-owned process.
   - Turning discovery on performs a complete rescan. Recovered components follow their normal skill availability and MCP start rules.

33. Apart from the global discovery kill switch, Warp does not add plugin-level enablement, trust fingerprints, reapproval prompts, or a plugin inventory surface in v1. Skill invocation and MCP start controls stay in their existing Skills and MCP surfaces.

34. This proposal intentionally preserves today's trust behavior. Repository skills can influence agent behavior and request ordinary shell commands under the active command permissions. Repository MCP commands become executable after the existing explicit MCP start action. The existing MCP start detail must show the resolved command, working directory, package source, and scope before a user starts a project-scoped stdio server.

### Plugin data

35. A `stdio` server declared by plugin `mcp.json` is the only process that Warp launches on a plugin's behalf. Each such MCP server process receives absolute `PLUGIN_ROOT` and `PLUGIN_DATA` environment variables.

36. `PLUGIN_DATA` is outside the package root, writable by the subprocess, dedicated to one stable plugin instance, and preserved across plugin version or package-content updates.

37. A local plugin instance is keyed by source kind, source identity, provider directory, and manifest name. Repository data remains separate from user data and from another repository with the same plugin name.

38. GUI and TUI treat the same discovered package as separate runtime instances. They do not share running MCP processes or writable `PLUGIN_DATA`. This follows the existing frontend-specific MCP state boundary and prevents concurrent client versions from mutating the same plugin state.

39. Warp creates `PLUGIN_DATA` immediately before the first stdio start. Removing a package or disabling discovery does not delete its data. A future uninstall workflow can offer deletion.

### Factories: plugin discovery and scope

40. File-managed Factories discover plugin candidates from immediate children of:
   - `<factory-root>/plugins/` for factory-scoped plugins.
   - `<factory-root>/agents/<agent-name>/plugins/` for agent-scoped plugins.
   - `<factory-root>/automations/<automation-name>/plugins/` for automation-scoped plugins.

41. Factory plugin roots are in the Factory source repository. V1 does not fetch plugin packages from a URL, registry, marketplace, or another repository.

42. A direct agent run loads agent-scoped plugins and factory-scoped plugins. An automation run loads automation-scoped plugins, plugins for the automation's bound agent, and factory-scoped plugins. The Automation projector snapshots `factory_automation_name` into the run configuration so runtime resolution does not silently degrade an automation run to agent scope.

43. Same-name Factory plugins shadow as complete packages in this order:
   1. Automation scope.
   2. Agent scope.
   3. Factory scope.

44. Two same-name plugins in the same Factory scope are a Factory validation error. The Factory does not select by child-directory name.

45. Factory plugins use the same qualified skill and MCP names as local plugins. Examples:
   - An agent prompt can explicitly invoke `/release-tools:deploy`.
   - The model sees MCP server `release-tools:registry` and tool identity `release-tools:registry/publish`.
   - Factory frontmatter does not list plugin components. Directory placement determines plugin scope.

46. Existing Factory `skills/` and `agents/<name>/skills/` remain supported. They remain flat skills. A same-name flat and plugin skill requires qualified invocation; neither silently replaces the other.

47. The client-side `.factory/skills` Droid provider is unrelated to a Factory product's root `skills/` directory. This proposal does not add `.factory/plugins` and does not change the Droid provider.

48. Factory plugin packages must pass the same Agent Plugins validation at Factory sync and again in the runtime checkout. An error-severity sync diagnostic prevents a new invalid Factory definition from being applied. A warning-severity diagnostic does not. A runtime mismatch disables the affected plugin or component and reports a run diagnostic.

49. Factory plugin MCP stdio server processes execute in the selected worker environment. They never execute in the Warp control-plane process. A skill-directed script executes through the run's normal command action and permissions in the same worker environment.
   - The Namespace worker supplies a principal-scoped durable `WARP_PLUGIN_DATA_ROOT` at `/cache/warp/plugin-data/<factory-uid>`. The server validates the Factory UID as one safe path segment before joining it. An invalid UID causes the server to omit the root.
   - Docker sandboxes are recreated per run and do not supply a durable root in v1.
   - Self-hosted workers own their storage contract and do not receive a server-assumed root in v1.
   - When `WARP_PLUGIN_DATA_ROOT` is absent, the client loads plugin skills and Streamable HTTP servers but refuses to start plugin stdio servers with a persistent-data diagnostic.
   - The server also supplies `WARP_FACTORY_UID` for Factory identity and diagnostics. The client never uses that variable to compose a path because the server has already included the UID in `WARP_PLUGIN_DATA_ROOT`.
   - The client appends exactly two path segments: `<scope-segment>/<plugin-key>`. The scope segment is `factory`, `agent-<sanitized-agent-name>`, or `automation-<sanitized-automation-name>`. The plugin key is the sanitized manifest name.
   - Sanitization keeps lowercase letters, digits, `.`, `_`, and `-`; lowercases uppercase ASCII; and replaces every other character with `-`. If the mapping changes a non-reserved input, Warp appends a stable eight-hex-character digest of the original input. An empty, `.`, or `..` result becomes the digest alone. This keeps distinct names from silently colliding and prevents reserved segments from escaping the root.

50. Factory source registration, repository access, branch controls, and the existing Factory apply flow remain the trust boundary. V1 adds no per-run plugin approval prompt.

### Two distinct `mcp.json` artifacts

51. A plugin `mcp.json` and a Factory `mcp.json` are different artifacts:
   - Plugin `mcp.json` is inside a plugin root beside `plugin.json`. It uses the canonical Agent Plugins schema. It never accepts a managed Warp MCP entry.
   - Factory `mcp.json` is outside every plugin root. It uses Warp's Factory MCP schema. It can contain managed and ordinary MCP entries.

52. Factory MCP files use these fixed locations:
   - `<factory-root>/mcp.json`.
   - `<factory-root>/agents/<agent-name>/mcp.json`.
   - `<factory-root>/automations/<automation-name>/mcp.json`.

53. Location is the primary discriminator. The required `$schema` is the second discriminator. A Factory file targets `https://warp.dev/schemas/factory-mcp/1.0.0/schema.json`; a plugin file targets the Agent Plugins MCP schema.

54. A Factory MCP entry uses one of these closed variants:
   - `managed`: required `type: "managed"` and `warpId`.
   - `stdio`: Agent Plugins-shaped `type`, `command`, optional `args`, optional `env`, and optional `cwd`.
   - `streamable-http`: Agent Plugins-shaped `type`, `url`, and optional `headers`.

55. Factory ordinary entries are deliberately similar to plugin entries, but they are not Agent Plugins components. Relative `command` and `cwd` paths resolve against the directory containing the Factory `mcp.json`. Factory files do not define or expand `${PLUGIN_ROOT}` or `${PLUGIN_DATA}`.

56. Factory authors should use a plugin when a local MCP server needs packaged files, skills, or persistent plugin data. Entity-level Factory `mcp.json` is intended for managed MCP references, remote endpoints, bare executables, and simple entity-relative commands.

57. Factoryfile sync reads every applicable Factory `mcp.json`:
   - It validates the complete Warp schema.
   - It projects `managed` entries onto the matching factory, agent, or automation level.
   - It leaves ordinary entries in the repository for the runtime client.

58. The runtime client reads applicable Factory `mcp.json` files and loads only ordinary entries. It recognizes and ignores valid `managed` entries because the server has already projected them into the run's managed MCP configuration.

59. Factory MCP scope follows current Factory behavior:
   - Factory-level managed entries are required additions for all agents and automations.
   - Agent-level entries apply to that agent.
   - Automation-level entries apply only to that automation run and overlay its bound agent's effective configuration.
   - A conflicting same-name entry with different configuration is a validation error. Identical entries deduplicate.

60. A plugin author cannot add a managed server by placing a Factory-shaped `mcp.json` or a `managed` entry in a plugin root:
   - In an interactive client, the plugin loader sees the unsupported schema or entry, disables MCP for that plugin, and reports a diagnostic. Independently valid plugin skills continue to load.
   - In Factory source validation, the same shape is an error-severity diagnostic that blocks sync and the Factory PR check.
   - Factoryfile sync never interprets the plugin-root file as a Factory MCP file and never projects its managed entry.
   - Agent Plugins component isolation does not apply to this Factory sync error. A managed entry asserts a Warp-controlled privilege that a portable plugin cannot hold. Silently dropping it would hide the invalid privilege request from the Factory author.

61. A Factory MCP file with the Agent Plugins `$schema` is invalid at an entity-level Factory MCP location. The error explains that Agent Plugins MCP belongs inside a plugin package.

### Factory MCP migration

62. Existing `mcpServers` fields in `factory.yaml`, agent frontmatter, automation frontmatter, and `agentDefaults` remain readable during migration.

63. New Factory authoring uses entity-level `mcp.json`. The initial release does not automatically rewrite or delete authored YAML/frontmatter.

64. Root Factory `mcp.json` replaces the top-level factory `mcpServers` use case. Agent and automation files replace their matching frontmatter use cases. `agentDefaults.mcpServers` remains legacy-only in v1 because no distinct entity-level file represents a default that can be replaced by every agent. An author who removes it must copy the intended entries into each applicable agent's `mcp.json`.

65. When legacy YAML and a new Factory MCP file declare the same managed server name:
   - The same `warpId` deduplicates.
   - Different `warpId` values fail Factory validation.
   - Ordinary stdio and HTTP entries can be declared only in the new Factory MCP file.

66. The Factory diagnostic channel distinguishes `error` from `warning`. `error` is the zero/default severity so existing diagnostics remain blocking. Factory sync and the Factory PR check fail only when at least one error diagnostic exists and surface warnings separately. Warp emits the legacy `mcpServers` deprecation as a warning after the new format is available. Removing legacy support requires a separate migration decision based on usage telemetry.

### Conformance target and phases

67. The desktop app and TUI target Agent Plugins 1.0.0 client conformance for both standard component types, with stdio and Streamable HTTP support. Legacy SSE remains an optional unsupported transport.

68. Factory runtime support claims Agent Plugins MCP stdio conformance only on a backend that supplies a durable `WARP_PLUGIN_DATA_ROOT`. A backend without that root can still load plugin skills and Streamable HTTP servers, but it does not claim complete plugin MCP conformance. Warp's Factory `mcp.json` is explicitly outside every Agent Plugins conformance claim.

69. Delivery is phased:
   1. Shared client parser, discovery, diagnostics, qualified identity, skills, MCP, plugin data, the global discovery kill switch, and adapters for existing Settings, command palette, Skills, and MCP surfaces.
   2. Factory plugin discovery, sync validation, runtime scoping, environment-variable propagation, and client-enforced plugin-data preconditions.
   3. Factory `mcp.json`, managed-entry projection, ordinary-entry runtime loading, and legacy YAML migration diagnostics.

70. Factory plugin and Factory MCP activation is gated by the server feature flag `factory_agent_plugins`. The flag is enabled in local and staging environments and disabled in production for the initial release. No server-to-client capability channel exists in v1, so apply cannot reject one incompatible worker/client independently. Production activation requires a compatible fleet deployment. Per-worker/client capability negotiation is a follow-up.

## Decisions
- Use `.agents/plugins` and `.warp/plugins` rather than bare repository `plugins/` in the client. This mirrors established provider directories and prevents a generic repository folder from gaining execution semantics.
- Use qualified component identity but preserve native MCP tool names. This removes component ambiguity without changing MCP wire contracts.
- Preserve current skills and file-based MCP execution behavior. A stricter plugin-specific trust model would be safer for global stdio plugins, but it would create inconsistent semantics for equivalent existing configuration.
- Provide one immediate interactive-client discovery kill switch. Per-source and per-plugin controls are deferred because the requested v1 control is global.
- Keep plugin and Factory MCP files separate. Extending the closed Agent Plugins schema with `warpId` would break conformance.
- Reuse the Factory environment-variable dispatch seam across worker types. This matches existing Factory skill propagation and avoids three separate CLI integrations.
- Treat an absent Factory plugin data root as a client-side stdio precondition failure. The server cannot claim durable storage for every worker backend.
- Compose the Factory UID into the durable root on the server, then let the client append one sanitized scope segment and one sanitized plugin key. This assigns each namespace part to the side that owns it and prevents Factories under one principal from sharing data.
- Use in-repository Factory plugins only. Agent Plugins 1.0.0 defines no installation or distribution protocol.
- Reserve `dev.warp.client` and `dev.warp.factory` extension namespaces, but require no Warp extension data in v1.

## Risks
- Existing global Warp MCP behavior can auto-start a stdio server from a user-controlled config location. Applying the same behavior to user Warp-home plugins increases the amount of executable configuration that can use that path. Existing MCP connection details and structured logs must preserve plugin source and command provenance. A unified trust redesign for all file-based MCP is a follow-up, not a plugin-only exception.
- A plugin skill can steer the model to run a bundled script. This uses existing command permissions, risk classification, allowlists, denylists, and approval behavior. It does not bypass command controls through the plugin or MCP lifecycle.
- Disabling plugin discovery cannot identify and terminate an ordinary shell command that a skill started earlier. The toggle stops plugin MCP processes and prevents new plugin skill use; the user retains the existing shell-command controls for an already-running command.
- A self-hosted direct worker that supplies a durable plugin-data root executes plugin MCP stdio servers and skill-directed shell commands with that backend's existing process isolation. Agent Plugins path containment is not a sandbox. Factory documentation must state this boundary.
- Docker and self-hosted workers do not yet have a Warp-owned durable plugin-data contract. Skills and Streamable HTTP remain available, but plugin stdio is unavailable until the worker supplies a durable root.
- The Factory plugin-data layout crosses two repositories. A stale vendored contract can remain internally consistent, so the client copy must record the canonical server commit and file hash and release validation must compare both copies.
- The `factory_agent_plugins` flag gates a deployment rather than an individual client capability. Production rollout must not enable the flag until every routed worker/client can consume the runtime environment contract.
- Two similar `mcp.json` schemas can confuse authors. Fixed locations, distinct required `$schema` values, editor schemas, and targeted diagnostics mitigate the risk.

## Out of scope
- Plugin installation, updates, registries, marketplaces, remote fetch, signatures, provenance, or dependency resolution.
- A new permission model, subprocess sandbox, or plugin-specific trust store.
- A dedicated plugin inventory or management UI, including a GUI Plugins inventory page and TUI `/plugins`. The single Warp Agent discovery toggle is in scope. A future phase can add browsing, package status, and plugin- or scope-level controls without changing v1 discovery or component identity.
- A Factory-level plugin discovery toggle or per-run override.
- Agent Plugins extensions beyond reserving Warp-owned namespaces.
- Legacy SSE support for Agent Plugins MCP.
- Automatic deletion, backup, or migration of plugin data.
- Warp-owned durable plugin-data provisioning for Docker and self-hosted workers.
- A server-to-worker/client capability negotiation channel for Agent Plugins.
- Reconstructing plugin packages or Factory `mcp.json` during live-Factory-to-file rendering. These file-only resources have no live-Factory counterpart, so `RenderTree` omits them.
- Remote plugins outside the Factory source repository.
- Claude Code plugin loading or conversion.

Claude Code remains a future compatibility target. Default `skills/<name>/SKILL.md` content and most metadata align, but a Claude Code plugin is not directly conformant: its manifest and MCP paths, closed-schema behavior, required MCP `type`, and placeholders differ. The shared parser, discovery, identity, and runtime work in this proposal reduces future effort, but support will still require a converter or a separate provider. It is not a near-zero-cost alias.
