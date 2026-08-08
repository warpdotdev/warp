# APP-5237: Agent Plugins 1.0.0 Technical Design
## Context
The [product spec](PRODUCT.md) defines client and Factory behavior. This technical design is pinned to:
- Warp client commit [`7a6044bd`](https://github.com/warpdotdev/warp/tree/7a6044bd5377d708ab1d3767ece505a49d232aed).
- Warp server commit [`d35b195a`](https://github.com/warpdotdev/warp-server/tree/d35b195a9bee8b512f860df1dcb77619ecf278d9).
- Warp server runtime-contract commit [`d07f5070`](https://github.com/warpdotdev/warp-server/tree/d07f507034abf13886bb623013bcc61f717d9898) from [warp-server PR #14030](https://github.com/warpdotdev/warp-server/pull/14030).
- Published [Agent Plugins 1.0.0](https://github.com/agentplugins/agent-plugins-spec/blob/main/spec/1.0.0.md).

The client has no plugin package abstraction today.

- [`crates/ai/src/skills/skill_provider.rs:106`](https://github.com/warpdotdev/warp/blob/7a6044bd5377d708ab1d3767ece505a49d232aed/crates/ai/src/skills/skill_provider.rs#L106) defines flat skill-provider precedence. `.agents` ranks before `.warp`.
- [`app/src/ai/skills/file_watchers/skill_watcher.rs:92`](https://github.com/warpdotdev/warp/blob/7a6044bd5377d708ab1d3767ece505a49d232aed/app/src/ai/skills/file_watchers/skill_watcher.rs#L92) watches home and repository skills.
- [`app/src/ai/skills/skill_manager.rs:106`](https://github.com/warpdotdev/warp/blob/7a6044bd5377d708ab1d3767ece505a49d232aed/app/src/ai/skills/skill_manager.rs#L106) scopes skills by home, current repository, or all cloud repositories.
- [`app/src/ai/skills/resolve_skill_spec.rs:103`](https://github.com/warpdotdev/warp/blob/7a6044bd5377d708ab1d3767ece505a49d232aed/app/src/ai/skills/resolve_skill_spec.rs#L103) resolves explicit CLI skill names and already reports repository ambiguity.
- [`app/src/ai/mcp/file_mcp_watcher.rs:122`](https://github.com/warpdotdev/warp/blob/7a6044bd5377d708ab1d3767ece505a49d232aed/app/src/ai/mcp/file_mcp_watcher.rs#L122) watches provider-specific file-based MCP configuration.
- [`app/src/ai/mcp/file_based_manager.rs:18`](https://github.com/warpdotdev/warp/blob/7a6044bd5377d708ab1d3767ece505a49d232aed/app/src/ai/mcp/file_based_manager.rs#L18) owns parsed file-based installations and scope.
- [`app/src/ai/mcp/file_based_manager.rs:345`](https://github.com/warpdotdev/warp/blob/7a6044bd5377d708ab1d3767ece505a49d232aed/app/src/ai/mcp/file_based_manager.rs#L345) preserves current MCP auto-start behavior: global Warp always, enabled global third-party in GUI, and never project-scoped.
- [`app/src/settings/ai.rs:1967`](https://github.com/warpdotdev/warp/blob/7a6044bd5377d708ab1d3767ece505a49d232aed/app/src/settings/ai.rs#L1967) is the settings declaration pattern for an Agent/MCP preference with a TOML path, platform surface, default, and cloud-sync policy.
- [`app/src/settings_view/ai_page.rs:635`](https://github.com/warpdotdev/warp/blob/7a6044bd5377d708ab1d3767ece505a49d232aed/app/src/settings_view/ai_page.rs#L635) registers AI setting action pairs for the command palette.
- [`app/src/settings_view/ai_page.rs:2855`](https://github.com/warpdotdev/warp/blob/7a6044bd5377d708ab1d3767ece505a49d232aed/app/src/settings_view/ai_page.rs#L2855) builds searchable Warp Agent settings widgets.
- [`app/src/settings_view/mod.rs:793`](https://github.com/warpdotdev/warp/blob/7a6044bd5377d708ab1d3767ece505a49d232aed/app/src/settings_view/mod.rs#L793) creates state-aware enable/disable command-palette bindings from one setting action.
- [`app/src/workspace/view.rs:23384`](https://github.com/warpdotdev/warp/blob/7a6044bd5377d708ab1d3767ece505a49d232aed/app/src/workspace/view.rs#L23384) publishes AI preference state into the keymap context used to select the correct command-palette label.
- [`crates/warp_core/src/paths.rs:62`](https://github.com/warpdotdev/warp/blob/7a6044bd5377d708ab1d3767ece505a49d232aed/crates/warp_core/src/paths.rs#L62) owns Warp's channel-aware home config directory.
- [`crates/warp_core/src/paths.rs:208`](https://github.com/warpdotdev/warp/blob/7a6044bd5377d708ab1d3767ece505a49d232aed/crates/warp_core/src/paths.rs#L208) intentionally separates TUI global MCP configuration from GUI configuration.
- [`app/src/settings/mod.rs:600`](https://github.com/warpdotdev/warp/blob/7a6044bd5377d708ab1d3767ece505a49d232aed/app/src/settings/mod.rs#L600) intentionally gives GUI and TUI separate `settings.toml` files.

Factories currently have direct flat skills and managed MCP references only.

- [`logic/factoryfile/path.go:8`](https://github.com/warpdotdev/warp-server/blob/d35b195a9bee8b512f860df1dcb77619ecf278d9/logic/factoryfile/path.go#L8) defines factory and agent skill paths.
- [`logic/factoryfile/v1alpha1.go:28`](https://github.com/warpdotdev/warp-server/blob/d35b195a9bee8b512f860df1dcb77619ecf278d9/logic/factoryfile/v1alpha1.go#L28) admits `mcpServers` in factory YAML and agent/automation frontmatter.
- [`logic/factoryfile/v1alpha1.go:543`](https://github.com/warpdotdev/warp-server/blob/d35b195a9bee8b512f860df1dcb77619ecf278d9/logic/factoryfile/v1alpha1.go#L543) accepts only `warpId` entries.
- [`logic/factoryfile/resolution/resolution.go:643`](https://github.com/warpdotdev/warp-server/blob/d35b195a9bee8b512f860df1dcb77619ecf278d9/logic/factoryfile/resolution/resolution.go#L643) merges managed MCP by name and rejects conflicting IDs.
- [`logic/factoryfile/projector/agent.go:370`](https://github.com/warpdotdev/warp-server/blob/d35b195a9bee8b512f860df1dcb77619ecf278d9/logic/factoryfile/projector/agent.go#L370) validates managed MCP against team scope before projection.
- [`logic/factoryfile/projector/automation.go:35`](https://github.com/warpdotdev/warp-server/blob/d35b195a9bee8b512f860df1dcb77619ecf278d9/logic/factoryfile/projector/automation.go#L35) validates automation MCP overrides.
- [`logic/ai/ambient_agents/workers/common/factory_skill_dirs.go:22`](https://github.com/warpdotdev/warp-server/blob/d35b195a9bee8b512f860df1dcb77619ecf278d9/logic/ai/ambient_agents/workers/common/factory_skill_dirs.go#L22) derives applicable Factory skills at dispatch and sends them through `WARP_SKILL_DIRS`.
- [`logic/ai/ambient_agents/workers/common/task_utils.go:816`](https://github.com/warpdotdev/warp-server/blob/d35b195a9bee8b512f860df1dcb77619ecf278d9/logic/ai/ambient_agents/workers/common/task_utils.go#L816) sends effective managed MCP to the client through `--mcp`.

The completed server implementation establishes these additional contracts:
- [`logic/ai/ambient_agents/workers/common/factory_runtime_dirs.go (18-261) @ d07f507`](https://github.com/warpdotdev/warp-server/blob/d07f507034abf13886bb623013bcc61f717d9898/logic/ai/ambient_agents/workers/common/factory_runtime_dirs.go#L18-L261) defines the shared Factory plugin/MCP environment-variable seam, validates the Factory UID, and composes the Factory-specific plugin-data root.
- [`logic/ai/ambient_agents/workers/common/testdata/factory_plugin_runtime_contract.json @ d07f507`](https://github.com/warpdotdev/warp-server/blob/d07f507034abf13886bb623013bcc61f717d9898/logic/ai/ambient_agents/workers/common/testdata/factory_plugin_runtime_contract.json) is the canonical executable contract for Factory runtime variables, plugin-data composition, segment sanitization, and worked examples.
- [`logic/ai/ambient_agents/sources/agent_config.go:112 @ d07f507`](https://github.com/warpdotdev/warp-server/blob/d07f507034abf13886bb623013bcc61f717d9898/logic/ai/ambient_agents/sources/agent_config.go#L112) stores `factory_automation_name` in run configuration.
- [`logic/factoryfile/rendertree.go:3 @ d07f507`](https://github.com/warpdotdev/warp-server/blob/d07f507034abf13886bb623013bcc61f717d9898/logic/factoryfile/rendertree.go#L3) documents the file-only resources omitted by rendering.
- [`config/features/features.go:373 @ d07f507`](https://github.com/warpdotdev/warp-server/blob/d07f507034abf13886bb623013bcc61f717d9898/config/features/features.go#L373) defines `factory_agent_plugins`.
## Proposed changes
### 1. Shared client package model
Add `crates/ai/src/plugins/` with no UI or filesystem-watcher dependency:
- `manifest.rs` contains versioned manifest types and semantic validation.
- `mcp.rs` contains versioned Agent Plugins MCP types and per-entry validation.
- `package.rs` contains `PluginPackage`, `PluginComponent`, diagnostics, canonical identities, and failure boundaries.
- `paths.rs` performs filesystem-resolved containment checks.
- `schema/1.0.0/` vendors the published immutable manifest and MCP schemas.

The loader selects a parser by exact canonical `$schema`. It never performs a network fetch. Semantic checks supplement JSON Schema for path containment, URL origin, case-insensitive duplicate headers, command-token rules, and version matching.

Use a structured identity instead of overloading a display string:

```rust
pub struct PluginInstanceId {
    pub scope: PluginScopeId,
    pub source: PluginSourceId,
    pub manifest_name: String,
}

pub struct PluginSourceId {
    pub kind: PluginSourceKind,
    pub stable_identity: String,
}

pub struct PluginComponentId {
    pub plugin: PluginInstanceId,
    pub kind: PluginComponentKind,
    pub local_name: String,
}
```

`PluginSourceKind` is `AgentsDirectory`, `WarpDirectory`, or `FactoryRepository` in v1. `stable_identity` identifies the user root, repository, or Factory source without depending on a mutable version. Keep source identity opaque at component boundaries so a later remote source kind does not change component identity. `PluginScopeId` distinguishes user, repository, factory, agent, and automation instances. UI/model adapters render `<plugin>:<component>`.

Do not treat plugins as one more row in `SKILL_PROVIDER_DEFINITIONS`. That list describes flat skill roots, while a plugin requires manifest-first loading and package-level shadowing.
### 2. Client discovery and watching
Add `app/src/ai/plugins/`:
- `plugin_watcher.rs` watches configured search roots and detected repositories.
- `plugin_manager.rs` owns candidate snapshots, precedence, active packages, diagnostics, and component registration.
- `plugin_data.rs` resolves persistent data paths and prepares stdio environments.
- `factory_mcp.rs` parses the distinct Warp Factory MCP schema supplied by a worker.

`PluginWatcher` reuses:
- `HomeDirectoryWatcher` to notice `.agents` creation.
- `WarpManagedPathsWatcher` for the channel-aware Warp home plugin root.
- `RepoMetadataModel` and repository subscribers for project roots.

Extend `WarpManagedPathsWatcher` with a precise recursive root for `<warp-home-config-dir>/plugins`. Preserve its existing `worktrees` exclusion. Do not recursively watch the complete Warp home config directory.

For each search root:
1. Enumerate immediate child directories.
2. Resolve the candidate root and root manifest.
3. Parse `plugin.json`.
4. Build the package snapshot.
5. Parse standard components independently.
6. Publish one generation-tagged update so stale asynchronous parses cannot replace newer state.

Package-level parse, unsupported-component, and shadowing diagnostics emit structured log events with stable diagnostic codes, source path, and scope. Invalid or ambiguous explicit skill invocation returns the matching codes and candidate identities. Component-level skill and MCP status continues through existing Skills and MCP models.

Precedence is a tuple:

```text
(scope rank: repository < user, provider rank: .agents < .warp)
```

Lower rank wins. Same-rank cross-repository collisions are ambiguous. Shadowing occurs after manifest validation and by manifest `name`, not child-directory name.

Add one setting to `AISettings`:

```rust
plugin_discovery_enabled: AgentPluginDiscoveryEnabled {
    type: bool,
    default: true,
    supported_platforms: SupportedPlatforms::ALL,
    sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
    surface: settings::SettingSurfaces::ALL,
    private: false,
    toml_path: "agents.plugins.discovery_enabled",
    description: "Whether Warp discovers Agent Plugin packages.",
}
```

The GUI surface follows the existing AI settings pattern:
- Add `PluginDiscoveryWidget` to `AISettingsPageView::build_page(Some(AISubpage::WarpAgent), ...)` behind the static `FeatureFlag::AgentPlugins` gate.
- Render one `Plugins` section with the `Agent Plugin discovery` switch and a short description that turning it off removes all plugin skills and MCP servers from the interactive client.
- Keep the setting in its own `SettingsWidget`. Use exactly `agent plugin plugins discovery skills mcp disable stop` as `search_terms`.
- Do not use `should_render` for the setting value. The widget must remain present while discovery is disabled. The build-time feature-flag check is static for the process.
- Add `AISettingsPageAction::TogglePluginDiscovery`, persist through `toggle_and_save_value`, and notify observers.
- Add `flags::PLUGIN_DISCOVERY_ENABLED` to `app/src/settings_view/mod.rs` and publish it from `Workspace::keymap_context`.
- Register `ToggleSettingActionPair::new("Agent Plugin discovery", ...)` in `ai_page::init_actions_from_parent_view`, under `BindingGroup::WarpAi`, behind `FeatureFlag::AgentPlugins`. This yields the mutually exclusive `Enable Agent Plugin discovery` and `Disable Agent Plugin discovery` command-palette entries through the existing command binding data source.

GUI and TUI use the same setting type and TOML key. They keep their existing frontend-specific `settings.toml` files; existing settings synchronization carries the value when enabled. V1 adds no TUI toggle surface.

`PluginManager` observes `AgentPluginDiscoveryEnabled` in interactive execution. Model discovery policy explicitly distinguishes:

```rust
pub enum PluginDiscoveryPolicy {
    InteractivePreference,
    RequiredByFactory,
}
```

On an enabled-to-disabled transition, `PluginManager` must first reject new registrations and component lookups, then:
1. Stop `PluginWatcher` subscriptions and invalidate outstanding generation-tagged parse tasks.
2. Publish an empty plugin-skill generation to `SkillManager`.
3. Cancel in-flight plugin MCP tool calls with `agent_plugin_discovery_disabled`.
4. Stop and unregister every plugin-provenance installation in `FileBasedMCPManager`.
5. Preserve package files, structured diagnostics, and plugin data.

Historical transcript content remains renderable. A shell command previously requested by a plugin skill is not plugin-provenance MCP work and is not terminated by this transition.

On a disabled-to-enabled transition, create fresh watchers and perform a complete rescan. Do not revive stale snapshots. Recovered components then follow the normal provider/scope start policy.

Factory dispatch selects `RequiredByFactory`. It does not consult a service account's or requester's personal `AgentPluginDiscoveryEnabled` preference. A future Factory-level kill switch requires an explicit Factory schema and policy design.
### 3. Skill integration
Add a plugin ingestion API to `SkillManager`. It accepts parsed skills with explicit `PluginComponentId` and owning scope instead of deriving provider and parent from a flat path.

Extend `ParsedSkill` or `SkillDescriptor` with optional plugin provenance and a runtime invocation name. Keep the Agent Skills frontmatter name unchanged. The runtime invocation name is qualified.

Update:
- Skill catalog serialization to send the qualified invocation name.
- Slash-command and explicit `--skill` resolution to accept `<plugin>:<skill>`.
- `SkillReference` with a plugin component variant, or an equivalent stable structured reference. Do not encode identity only in a mutable path.
- Ambiguity errors to include flat and plugin candidates.
- GUI and TUI skill lists to show the qualified plugin skill name and existing source detail.

Unqualified resolution first gathers every active candidate. It returns a match only when exactly one candidate has that local name. Existing repository qualification remains available for flat skills and can disambiguate cross-repository plugin sources before plugin qualification.
### 4. MCP integration
Do not parse plugin MCP through the native provider parser. Agent Plugins has a different closed schema, required `type`, and different placeholder rules.

Map each valid plugin server into a `TemplatableMCPServerInstallation` plus immutable launch context:

```rust
pub struct PluginMcpLaunchContext {
    pub component_id: PluginComponentId,
    pub plugin_root: PathBuf,
    pub plugin_data: PathBuf,
    pub discovery_scope: FileBasedMCPServerScope,
    pub source: PluginSourceId,
}
```

Extend `FileBasedMCPManager` with a plugin source kind and registration API. Its stable hash must include `PluginComponentId` and normalized configuration. Two packages with identical JSON must not collapse into one installation.

Preserve `FileBasedMCPManager::auto_start_decision` by mapping:
- User Warp plugin source to `GlobalWarp`.
- User Agents plugin source to `GlobalThirdParty`.
- Every repository plugin source to `ProjectScoped`.
- Factory runtime source to an explicit runtime-start policy described in section 7.

Plugin `mcp.json` stdio servers are the only processes that the plugin loader launches on a package's behalf. The spawner must:
1. Create the dedicated plugin data directory.
2. Expand exact `${PLUGIN_ROOT}` and `${PLUGIN_DATA}` occurrences once in `args`, `env` values, and `cwd`.
3. Resolve and contain `command` and `cwd`.
4. Overlay configured `env`.
5. Set authoritative `PLUGIN_ROOT` and `PLUGIN_DATA` last.
6. Launch `command` as one executable token with a separate argument vector.

The MCP parser rejects a `cwd` with a literal `..` segment in `validate_cwd_form`, before registration or launch. Check whole slash-delimited segments, not substrings, so `./a..b` remains valid. Filesystem resolution at launch still performs the final containment check after `${PLUGIN_ROOT}` or `${PLUGIN_DATA}` is known. Rust and Go must run this case through the shared conformance corpus.

For HTTP:
- Parse absolute URL semantics before mapping to the native transport.
- Reject userinfo, fragments, non-HTTPS non-loopback origins, invalid duplicate headers, and redirect forwarding to another origin.
- Keep URL and headers literal.

The manager exposes plugin server provenance to GUI/TUI MCP models. Model tool metadata uses structured installation ID plus native tool name. Display adapters render `<plugin>:<server>/<tool>` without changing the MCP request's tool name.
### 5. Persistent plugin data
Add a `PluginDataLocator` interface:

```rust
pub trait PluginDataLocator {
    fn data_dir(&self, instance: &PluginInstanceId) -> Result<PathBuf>;
}
```

Local data lives under the active frontend's `warp_core::paths::data_dir()/plugins/data/<instance-key>`. `instance-key` is a filesystem-safe hash of frontend identity, stable source identity, scope, and manifest name. It excludes manifest version and component content. GUI and TUI therefore discover the same packages but do not share writable plugin state or running processes.

Skill-bundled scripts do not use `PluginDataLocator`. The skill content can direct the model to run one through the ordinary shell-command action. `BlocklistAIPermissions::can_autoexecute_command` applies the active execution profile, allowlist, denylist, risk classification, and user approval behavior. The plugin loader does not spawn the script and does not inject `PLUGIN_ROOT` or `PLUGIN_DATA`.

Factory runtime uses a cross-repository plugin-data contract:
- The server composes `WARP_PLUGIN_DATA_ROOT` as `<durable-base>/plugin-data/<factory-uid>`. It validates the Factory UID as one non-empty path segment that is neither `.` nor `..` and contains no `/` or `\`. An invalid UID causes the server to omit the root.
- The Namespace worker uses `/cache/warp` as its principal-scoped durable base and therefore exports `/cache/warp/plugin-data/<factory-uid>`.
- The Docker sandbox supplies no root because its filesystem is recreated for each run.
- The server assumes no root for a self-hosted worker because that worker owns its storage contract.
- Absence of `WARP_PLUGIN_DATA_ROOT` is the runtime capability signal. The client still loads plugin skills and Streamable HTTP servers, but it refuses to start a plugin stdio server and reports that persistent plugin data is unavailable.
- The client, not dispatch, enforces writability immediately before stdio start. The server must not emit an ephemeral fallback.
- Durable roots for Docker and self-hosted workers require follow-up changes in their worker or sandbox repositories.
- The server exports `WARP_FACTORY_UID` whenever the feature-gated file-managed Factory scope resolves. The client retains it for identity and diagnostics only. It must never pass the value to a path join because the server already included it in the root.

The client appends exactly two segments below `WARP_PLUGIN_DATA_ROOT`:
- Scope: `factory`, `agent-<sanitize(agent-name)>`, or `automation-<sanitize(automation-name)>`.
- Plugin key: `sanitize(manifest-name)`.

`sanitize(input)` is total and returns one safe path segment:
1. Map each character. Keep `[a-z0-9._-]`, lowercase `A-Z`, and replace every other character with `-`.
2. If the mapped value is byte-identical to the input and is not `""`, `"."`, or `".."`, return it unchanged.
3. Otherwise compute SHA-256 over the original input bytes and encode the first four digest bytes as eight lowercase hexadecimal characters.
4. If the mapped value is empty or reserved, return the digest alone. Otherwise return `<mapped>-<digest>`.

Two porting traps require explicit fixtures:
- Hash the original raw input, never the mapped value. Distinct inputs that map to the same visible segment must remain distinct.
- `..` survives the character mapping unchanged. The explicit reserved-value check, not the changed-value check, must catch it.

The composed path is `${WARP_PLUGIN_DATA_ROOT}/<scope-segment>/<plugin-key>`. Tests assert exactly two segments below the root. When `WARP_FACTORY_UID` is present, the client asserts that the UID occurs exactly once in the composed namespace, at the server-provided root boundary; it never appends the UID. Keeping Factory composition on the server prevents two Factories under one principal from colliding. It also assigns each part of the namespace to the side that owns the information. The earlier prose-only split let the two repositories implement incompatible layouts with no implementation capable of forming the specified namespace.

[`logic/ai/ambient_agents/workers/common/testdata/factory_plugin_runtime_contract.json`](https://github.com/warpdotdev/warp-server/blob/d07f507034abf13886bb623013bcc61f717d9898/logic/ai/ambient_agents/workers/common/testdata/factory_plugin_runtime_contract.json) is canonical. At `d07f507`, its SHA-256 is `8134f4924a429cc89322c29c1c7697c854572191e41183b63f0bbc4fb95b1692`. The client vendors it byte-for-byte at `crates/ai/src/plugins/contract/factory_plugin_runtime_contract.json` and stores these fields in adjacent `factory_plugin_runtime_contract.provenance.json`:

```json
{
  "upstream_repository": "warpdotdev/warp-server",
  "upstream_path": "logic/ai/ambient_agents/workers/common/testdata/factory_plugin_runtime_contract.json",
  "upstream_commit": "d07f507034abf13886bb623013bcc61f717d9898",
  "sha256": "8134f4924a429cc89322c29c1c7697c854572191e41183b63f0bbc4fb95b1692"
}
```

Every vendored copy must record equivalent provenance. A downstream test that compares code only with its own stale copy is insufficient. Cross-repository validation must compare the vendored bytes and recorded provenance with the canonical file at the server revision intended for deployment.

Concurrent processes can share one plugin instance data directory. Warp guarantees directory persistence, not application-level locking.
### 6. Factory source model and validation
Extend `logic/factoryfile` source classification with:
- Factory `plugins/<child>/plugin.json`.
- Agent `agents/<name>/plugins/<child>/plugin.json`.
- Automation `automations/<name>/plugins/<child>/plugin.json`.
- Factory `mcp.json`.
- Agent `agents/<name>/mcp.json`.
- Automation `automations/<name>/mcp.json`.

Add canonical source records:

```go
type PluginResource struct {
    Scope        PluginScope
    OwnerName    string
    ManifestName string
    RootPath     string
    Digest       string
}

type FactoryMCPFile struct {
    Scope     MCPScope
    OwnerName string
    Path      string
    Servers   map[string]FactoryMCPServerEntry
}
```

The tree parser validates plugin packages against vendored 1.0.0 schemas and semantic rules. Go and Rust conformance fixtures must be generated from the same committed fixture corpus so the implementations cannot drift.

Plugin package content is not copied into projected live-Factory rows. `PluginResource.Digest` records the validated package digest for diagnostics and future use. Runtime reads the checked-out package and revalidates it.

Projected resource semantic hashes deliberately exclude plugin content. Factory sync records the applied source SHA even when the plan contains no projected resource operation, so a plugin-only change advances the applied revision without manufacturing live-resource update churn. Do not inject `PluginResource.Digest` into another resource's semantic hash.

Extend `factoryfile.Diagnostic` with severity:
- `error` is the zero value. Every existing constructor and diagnostic remains blocking without migration.
- `warning` is explicit and non-blocking.
- Factory sync and the Factory PR check fail only when the result contains at least one error diagnostic.
- Both paths return warnings separately from blocking errors so authors see non-blocking migration guidance.
- The legacy `mcpServers` deprecation uses warning severity.

Factory source validation treats a Factory-schema plugin-root `mcp.json`, or any plugin-root MCP entry with `type: "managed"`, as a blocking error. This check occurs before ordinary Agent Plugins component-isolation handling. The file remains owned by the plugin location and is never parsed or projected as entity-level Factory MCP.

`RenderTree` reconstructs only resources with a live-Factory counterpart. Plugin packages and entity-level Factory `mcp.json` are file-only resources, so live-to-file export omits them. The export is not a round trip for these sources; the original Factory repository remains authoritative.
### 7. Factory runtime scoping
Add a runtime `FactoryFileScope` snapshot with:
- Factory UID and checked-out Factory root.
- Bound agent name.
- Optional automation name.
- Ordered applicable plugin collection paths.
- Ordered applicable Factory MCP file paths.

The Automation projector writes `factory_automation_name` into the run configuration JSONB snapshot. This needs no database migration. Dispatch reads the snapshot to populate the optional automation name. Without this value, runtime resolution has no Automation identity and cannot include automation-scoped plugin or Factory MCP paths, so every Automation-created run must carry it.

Factory plugin collection paths are:
- Automation `plugins/`, when present.
- Bound agent `plugins/`.
- Factory `plugins/`.

The client receives plugin collection directories, not individual package roots. It enumerates only immediate children of each collection. If a manifest name repeats in a later collection, the package from the earlier collection shadows it.

Factory MCP paths are:
- Automation `mcp.json`, when present.
- Bound agent `mcp.json`.
- Factory `mcp.json`.

Extend the existing environment-variable dispatch seam used by Factory skills. All three worker implementations call the shared `FactoryRuntimeEnvForTask` helper. Do not add worker-specific plugin CLI arguments.

- `WARP_PLUGIN_DIRS` is a comma-separated list of plugin collection directories, ordered automation > agent > factory.
- `WARP_FACTORY_MCP_FILES` is a comma-separated list of entity-level Factory MCP files in the same order.
- Each list item is relative to the environment working directory. The server prefixes the Factory-relative path with the cloned repository directory before encoding it.
- A path containing a comma cannot be encoded. The server omits that path and emits a warning rather than creating a corrupted list item.
- The client parses each `WARP_FACTORY_MCP_FILES` item, loads ordinary entries, and ignores valid managed entries.
- `WARP_FACTORY_UID` is the opaque Factory identity for diagnostics. The server emits it whenever the feature-gated file-managed Factory scope resolves. The client never uses it for path composition.
- `WARP_PLUGIN_DATA_ROOT` is separate from the ordered lists and the identity variable. It is one optional absolute path already namespaced to the Factory as `<durable-base>/plugin-data/<factory-uid>`. The client appends only the two sanitized segments defined in section 5.

Factory runtime plugins are part of the applied Factory definition and start with the run. Plugin MCP servers are not project cards that require an interactive start inside a headless worker. This follows the existing behavior of MCP already attached to a Factory agent or automation. The Factory source/apply trust boundary is responsible for this difference from an interactive repository session.

Before encoding a runtime path:
- Verify the path remains inside the checked-out Factory root.
- Verify the runtime checkout corresponds to the applied Factory source revision.
### 8. Warp Factory MCP schema
Publish and vendor an immutable closed schema at:

```text
https://warp.dev/schemas/factory-mcp/1.0.0/schema.json
```

Its shape is:

```json
{
  "$schema": "https://warp.dev/schemas/factory-mcp/1.0.0/schema.json",
  "mcpServers": {
    "search": {
      "type": "managed",
      "warpId": "00000000-0000-0000-0000-000000000000"
    },
    "lint": {
      "type": "stdio",
      "command": "./bin/lint-server",
      "args": ["--mode", "factory"],
      "cwd": "./"
    },
    "issues": {
      "type": "streamable-http",
      "url": "https://mcp.example.com/issues"
    }
  }
}
```

Top level permits only `$schema` and `mcpServers`. Each entry is exactly one closed variant.

- `managed` permits only `type` and `warpId`.
- `stdio` uses the Agent Plugins field shape, but paths are relative to the entity directory that contains the Factory MCP file.
- `streamable-http` uses the Agent Plugins field shape.
- `${PLUGIN_ROOT}` and `${PLUGIN_DATA}` are invalid in a Factory MCP file.
- V1 does not add Factory-specific secret interpolation. Managed MCP and existing Factory secrets remain the credential path.

Factoryfile sync is authoritative for managed entries:
1. Parse the file by fixed entity location.
2. Require the Warp Factory schema identifier.
3. Validate each `warpId` in team scope.
4. Merge managed entries at the existing entity level.
5. Project them through the existing service-account and automation paths.

The client is authoritative for ordinary entries:
1. Parse only the ordered files in `WARP_FACTORY_MCP_FILES`.
2. Require the Warp Factory schema identifier.
3. Ignore valid `managed` entries without creating an installation.
4. Load stdio and Streamable HTTP entries through `FileBasedMCPManager` with Factory provenance.
5. Resolve relative paths against the containing entity directory.

The interactive client must not accept the Factory schema or a managed entry in a plugin root. The plugin MCP parser reports the unsupported schema or entry and disables only plugin MCP. This preserves Agent Plugins component isolation for local interactive loading.

Factory sync is intentionally stricter. A Factory schema or managed entry in plugin-root `mcp.json` emits a blocking error rather than an isolated component warning. The shape asserts a Warp-managed privilege that the package location cannot hold. Factoryfile never promotes or silently drops it. Conversely, factoryfile rejects the Agent Plugins schema at an entity-level Factory MCP path.
### 9. Legacy Factory MCP migration
Keep legacy `mcpServers` parsing in `v1alpha1.go` during the transition. Convert both old and new managed entries into the existing canonical `MCPServerEntry` before resolution.

Merge by entity and server name:
- Same name and same normalized `warpId`: one entry.
- Same name and different `warpId`: source validation error naming both files.
- Ordinary entries exist only in the new Factory MCP source and are not projected as managed MCP.

Sync and reconciliation preserve the authored source representation. They do not rewrite a user's YAML/frontmatter into a new file. Separate live-to-file `RenderTree` export is lossy for plugin packages and Factory MCP files as described in section 6.
Root Factory `mcp.json` migrates top-level `factory.yaml` MCP, while agent and automation `mcp.json` files migrate their matching frontmatter. Keep `agentDefaults.mcpServers` as a legacy-only source in v1. There is no new default-only Factory MCP file. Migration tooling or documentation expands those defaults into each intended agent file before the legacy field is removed.

Add a warning-severity source diagnostic and telemetry counter for legacy declarations. Do not set a removal release in this change.
### 10. Feature rollout and capability gap
Use the client Agent Plugins feature gate for interactive parsing and discovery. Gate server-side Factory plugin parsing, Factory MCP handling, and runtime environment emission together with `factory_agent_plugins` (`FactoryAgentPluginsEnabled()`).

- Local and staging configurations enable `factory_agent_plugins`.
- Production configuration disables it for the initial release.
- No server-side channel reports an individual worker or client Agent Plugins capability.
- Factory apply therefore cannot reject one incompatible worker/client before dispatch.
- Production rollout can enable the flag only after the routed fleet has a compatible client and environment-variable contract.
- Per-worker/client capability advertisement and apply-time rejection remain a follow-up.

Stable rollout order:
1. Ship the capable client and worker contract.
2. Enable local user and repository discovery.
3. Enable Factory source validation.
4. Enable Factory runtime propagation.
5. Enable Factory MCP authoring and legacy diagnostics.
## Decisions
### Separate plugin and Factory MCP schemas
Options considered:
- Extend Agent Plugins `mcp.json` with `warpId`. Rejected because the standard schema is closed and an entry must match a standard transport.
- Put managed references in a Warp extension directory inside each plugin. Rejected because managed MCP is Factory configuration, not a portable plugin component.
- Define entity-level Factory `mcp.json`. Selected because location and `$schema` make ownership explicit while one Factory file can carry managed and ordinary servers.
- Treat a managed entry in a Factory plugin as an isolated malformed MCP component during sync. Rejected because it would silently hide a privilege request and let an invalid Factory definition apply.
- Reject a managed entry or Factory schema in a plugin root as a blocking sync error. Selected because managed MCP can be granted only from an entity-level Factory file. The interactive client still isolates the invalid plugin MCP component.
### Reuse existing execution semantics
Options considered:
- Add package-wide enablement and content-fingerprint approval. Safer for changed stdio packages, but inconsistent with equivalent existing skill and MCP sources.
- Reuse current source semantics. Selected by product direction. The implementation preserves source provenance in existing component details and logs, not a new trust system.
### Structured identity instead of string rewriting
Options considered:
- Rewrite source skill and MCP tool names. Rejected because it mutates portable metadata and MCP wire names.
- Carry structured package/component identity and render qualified labels at boundaries. Selected because routing stays stable and source packages remain portable.
### Runtime-local Factory MCP
Options considered:
- Import ordinary Factory MCP into the managed MCP database. Rejected because local package paths are meaningful only inside a checkout and worker.
- Let the client read applicable entity files from the checkout. Selected because it preserves path context and keeps the control plane from executing local commands.
### Shared Factory runtime environment
Options considered:
- Add repeated plugin and Factory MCP CLI arguments to each worker. Rejected because the three worker implementations already share the Factory environment-variable dispatch seam.
- Extend the shared seam with `WARP_PLUGIN_DIRS`, `WARP_FACTORY_MCP_FILES`, `WARP_FACTORY_UID`, and optional `WARP_PLUGIN_DATA_ROOT`. Selected because it centralizes scoping and preserves the current worker launch contract.
### Server-owned Factory plugin-data namespace
Options considered:
- Give the client a durable base and require it to append Factory UID, scope, and plugin identity. Rejected after adversarial cross-repository review. The server and client independently interpreted the prose differently, neither implementation could form the intended namespace, and Factories under one principal could collide.
- Have the server append the Factory UID and have the client append one scope segment plus one plugin key. Selected because the server owns Factory and storage identity while the client owns plugin scope and manifest identity. Each side now composes only the information it owns, and the client suffix is always exactly two safe segments.
- Rely on each repository's tests against its own vendored fixture. Rejected because a stale fixture remains internally self-consistent and cannot detect upstream drift.
- Vendor the canonical server fixture byte-for-byte and record its upstream commit SHA and file hash. Selected because reviewers and release validation can prove which contract a client implements and detect a stale copy.
### Applied revision instead of plugin-driven resource churn
Options considered:
- Include plugin digests in projected live-resource semantic hashes. Rejected because plugin content has no live-resource projection and would manufacture update operations.
- Record `PluginResource.Digest` but advance plugin-only source changes through the sync row's applied SHA. Selected because runtime receives the new checkout revision even when the resource plan is otherwise a no-op.
### Immediate global discovery switch
Options considered:
- Apply a disabled preference only after restart. Rejected because the requested control must act as a kill switch for already-loaded plugin MCP servers and skills.
- Add user/repository/package-specific switches. Rejected from v1 because they require inventory and precedence UX that the initial sprint deliberately defers.
- Add one interactive-client preference that immediately withdraws all plugin components. Selected because it is predictable, works from Settings and the command palette, and does not change Factory source semantics.
## Risks and mitigations
- Rust and Go validators can drift.
  - Keep one cross-repository fixture corpus derived from the published schemas. Run it against both implementations in CI.
- A vendored Factory runtime contract can remain internally self-consistent after the canonical server fixture changes.
  - Require byte-identical vendoring plus upstream repository, path, commit, and hash provenance. Compare the client copy with the canonical file from the intended server revision before rollout.
- A Factory UID or author-controlled scope name can escape or collide in persistent storage.
  - Validate the UID server-side before joining. Sanitize scope and plugin segments client-side with the canonical original-input hash rule and assert a two-segment suffix.
- A source revision can change between Factory validation and runtime.
  - Store the applied source revision and verify the checkout before emitting runtime paths.
- Global Warp-home plugins inherit automatic MCP start.
  - Show source and resolved launch details in existing MCP details and logs. Address stricter trust only through a unified file-based MCP design.
- A skill can instruct the agent to run package-supplied code.
  - Keep this on the ordinary shell-command path. Do not add a plugin bypass around execution-profile permissions, risk classification, allowlists, denylists, or approval.
- Plugin paths can escape through symlinks or platform-specific path behavior.
  - Centralize filesystem-resolved containment and test Unix symlinks plus Windows junction/reparse behavior.
- A worker can lack durable Factory plugin data.
  - Omit `WARP_PLUGIN_DATA_ROOT`. The client must reject plugin stdio start while continuing to load skills and Streamable HTTP servers.
- Comma-separated runtime path variables cannot represent a comma in a path.
  - Omit the unencodable item and emit a server warning. Keep generated Factory entity paths comma-free.
- The server cannot detect an incompatible worker/client before apply.
  - Keep production `factory_agent_plugins` disabled until the routed fleet is compatible. Add a capability channel before heterogeneous rollout.
- An Automation run can lose automation-scoped resources if its origin is not preserved.
  - Snapshot `factory_automation_name` during projection and test that dispatch consumes it.
- Qualified skill names can conflict with existing parser syntax.
  - Add parser fixtures for colon-qualified plugin skills and preserve repository-qualified flat skill behavior.
- Two `mcp.json` formats can be confused.
  - Require exact locations and exact schema identifiers. Emit targeted cross-format diagnostics.
- A plugin skill can start an ordinary shell command before discovery is disabled.
  - Withdraw the skill immediately, but leave that command under its existing shell-command lifecycle. Do not claim the discovery switch can identify unrelated process descendants.
## Testing and validation
### Agent Plugins conformance
Create a committed conformance fixture suite that covers every item in Appendix A of Agent Plugins 1.0.0:
- Manifest required fields, naming, unknown fields, extensions exceptions, and unsupported schema.
- Fixed component paths, missing paths, wrong filesystem kinds, and non-recursive skills.
- Symlink/path escape failures at plugin, component, skill, command, and working-directory boundaries.
- MCP top-level and per-server failure isolation.
- Stdio executable-token, default working directory, environment overlay order, reserved variables, and single non-recursive expansion.
- Parse-time rejection of a literal `..` `cwd` segment, while names that contain `..` only as a substring remain valid.
- Streamable HTTP URL, redirect-origin, and header validation.
- Unsupported SSE isolation.
- Component start, connection, authentication, and handshake failure isolation.

Both Rust and Go validators run the applicable shared fixtures.
### Client unit and integration tests
- `PluginWatcher` tests all four local search roots, immediate-child scanning, hot reload, channel-aware Warp paths, same-name precedence, and cross-repository ambiguity.
- `PluginManager` tests package-level shadowing and diagnostic preservation.
- `PluginManager` tests enabled-to-disabled ordering, stale-generation invalidation, skill withdrawal, in-flight MCP cancellation, server stop/unregistration, persistent-data preservation, disabled lookup diagnostics, and a fresh rescan when re-enabled.
- `SkillManager` and `resolve_skill_spec` test qualified invocation, unique unqualified alias, flat/plugin ambiguity, and cloud multi-repository scope.
- `FileBasedMCPManager` tests provider/scope auto-start parity with existing file-based MCP.
- `AISettings` tests the `agents.plugins.discovery_enabled` default, GUI/TUI surfaces, persistence, and sync policy.
- `AISettingsPageView` filter tests assert that `plugin`, `skills`, `MCP`, `disable`, and `stop` each select the single `PluginDiscoveryWidget`, and that the widget remains visible when its value is false.
- Command binding tests assert exactly one of `Enable Agent Plugin discovery` and `Disable Agent Plugin discovery` is available for the current context.
- TUI plugin-manager tests assert that its frontend-specific false setting prevents interactive discovery.
- MCP spawn tests assert exact `argv`, environment overlay order, authoritative variables, default `cwd`, persistent data path, and native tool-name routing.
- Factory runtime environment tests assert ordered parsing of `WARP_PLUGIN_DIRS`, immediate-child enumeration, first-collection shadowing, and ordered parsing of `WARP_FACTORY_MCP_FILES`.
- Factory plugin-data tests assert that `WARP_FACTORY_UID` is diagnostics-only, the UID is not appended below the root, and every path adds exactly `<scope-segment>/<plugin-key>`.
- Sanitization tests consume every canonical worked example, including raw-input digesting, `..`, `.`, empty input, separator replacement, uppercase folding, and two distinct inputs that map to the same visible text.
- Client test `vendored_factory_plugin_runtime_contract_matches_provenance` asserts that the vendored fixture bytes match the recorded SHA-256 and that all upstream provenance fields are present.
- Missing-data-root tests assert Factory plugin skills and Streamable HTTP load while plugin stdio start fails before spawn.
- Factory MCP client tests assert managed entries are ignored and ordinary entries load.
- Interactive-client cross-format tests assert a Factory schema or managed entry in a plugin root disables only that plugin's MCP component.

Run at minimum:

```text
cargo test -p ai plugins
cargo test -p warp --lib ai::plugins
cargo test -p warp --lib ai::skills
cargo test -p warp --lib ai::mcp
cargo nextest run -p warp settings_view
cargo test -p warp_tui plugins
cargo fmt --all -- --check
```

### Factory tests
- Parser fixtures cover all three plugin scopes and all three Factory MCP locations.
- Diagnostic tests assert error is the zero value, warnings do not fail sync or the Factory PR check, errors do fail both, and callers receive warnings separately.
- Plugin-root MCP tests assert a Factory schema or `managed` entry is a blocking sync error and is never projected. The equivalent interactive-client fixture continues to disable only plugin MCP.
- Entity-level Factory MCP tests assert an Agent Plugins schema at a Factory MCP path is a blocking source diagnostic.
- Resolution tests cover automation > agent > factory plugin shadowing.
- Automation projection tests assert `factory_automation_name` is stored in run config and selects automation-scoped runtime paths.
- Managed MCP tests cover legacy/new deduplication, conflicts, scope, team validation, and projection.
- Dispatch tests cover exact comma-separated environment values, `WARP_FACTORY_UID`, most-specific-first ordering, clone-directory prefixing, checkout containment, comma-path warnings, and source revision.
- Worker tests assert Namespace emits principal-scoped `/cache/warp/plugin-data/<factory-uid>`, rejects an invalid UID path segment, and never supplies an unscoped root. Docker and server-side self-hosted dispatch omit `WARP_PLUGIN_DATA_ROOT`.
- Cross-repository CI check `factory-plugin-runtime-contract` checks out the recorded upstream server commit, compares the canonical fixture with the client vendored copy byte-for-byte, and verifies the recorded SHA-256. A client test against only its own copy does not satisfy this criterion.
- Feature tests assert `factory_agent_plugins` gates Factory source activation and runtime environment emission and has the configured local/staging/prod defaults.
- Hash tests assert a plugin-only change leaves projected resource semantic hashes unchanged while sync advances the applied source SHA and retains `PluginResource.Digest`.
- Render tests assert plugin packages and Factory `mcp.json` are omitted and the limitation remains documented on `RenderTree`.
- Dispatch tests assert `RequiredByFactory` ignores any personal interactive discovery preference.
- End-to-end tests run a factory skill, a plugin stdio MCP tool, an ordinary entity-level MCP tool, and a projected managed MCP from one Factory.
- Run the complete stdio case on the Namespace worker. On Docker and self-hosted workers without a supplied durable root, verify skills and Streamable HTTP work and plugin stdio fails with the persistent-data diagnostic.

Run at minimum in `warp-server`:

```text
go test ./logic/factoryfile/...
go test ./logic/ai/ambient_agents/workers/common/...
go test ./logic/ai/ambient_agents/... -run 'Factory|Plugin|MCP'
gofmt -l logic/factoryfile logic/ai/ambient_agents
```

### User-visible proof
- Record a desktop video that adds a repository plugin, shows its qualified skill and MCP server in the existing component surfaces, explicitly starts and uses the project server, finds `Agent Plugin discovery` through Settings search, disables it from the command palette, and shows the skill withdrawal plus MCP server stop without restarting. Re-enable it and show a fresh rescan.
- Record a TUI video that shows the qualified skill and MCP server in the existing component surfaces, invokes the skill, starts the server, and uses one tool.
- Record a Namespace Factory run artifact that identifies the active factory/agent/automation plugin scopes and successfully calls both an ordinary plugin MCP tool and a managed Factory MCP tool.
## Parallelization
Implementation can use two workstreams after the shared identities and JSON contracts are agreed:
- Client: Rust schemas, parser, watcher, SkillManager, MCP manager, data locator, discovery preference/actions, and minimal existing-surface adapters on a Warp worktree.
- Factory: Go source parsing, Factory MCP schema, projection, runtime scope, and worker contract on a warp-server worktree.

Factory development can proceed in parallel against committed fixture/schema contracts, but production Factory rollout waits for a compatible client deployment and explicit `factory_agent_plugins` enablement. Use one PR per repository; keep the PRODUCT and TECH specs aligned in this Warp PR.
## Assumptions
- The Factory source revision can be recorded and compared with the worker checkout before launch.
- Existing Factory source registration and apply permissions remain the product trust boundary for repository code.
- The implementation can publish the proposed immutable Warp Factory MCP schema URL before enabling authoring.
## Out of scope
- Claude Code conversion or provider implementation.
- A dedicated GUI or TUI plugin inventory and plugin- or scope-level management controls beyond the single global discovery toggle.
- A Factory-level discovery toggle.
- Plugin distribution and installation.
- Warp-owned durable plugin-data provisioning for Docker and self-hosted workers.
- Server-side worker/client capability advertisement and apply-time compatibility checks.
- Live-to-file reconstruction of plugin packages and Factory MCP files.
- New secret-reference fields in either plugin or Factory ordinary MCP entries.
- A generalized permissions or subprocess-sandbox redesign.
- Automatic legacy YAML rewriting or removal.
- Agent Plugins legacy SSE transport.
