# MCP Prompts Capability — Tech Spec
Product spec: `specs/GH10740/product.md`
GitHub issue: https://github.com/warpdotdev/warp/issues/10740
## Problem
Warp's MCP startup path already negotiates server capabilities and lists tools/resources, but it omits the `prompts` capability. The implementation needs to add prompts to the MCP active-server model, expose active prompt metadata through the slash-command data sources, add an invocation path that calls `prompts/get`, and submit the rendered prompt into Agent Mode without bypassing existing AI/MCP safety gates.
## Relevant code
- `app/src/ai/mcp/templatable_manager/utils.rs:9` — capability-gated helper pattern for listing resources and tools with fail-soft behavior.
- `app/src/ai/mcp/templatable_manager/utils_tests.rs:11` — unit tests that sweep tool/resource capability combinations and list-helper failures.
- `app/src/ai/mcp/templatable_manager.rs:120` — `TemplatableMCPServerInfo`, currently storing `resources` and `tools` for a connected server.
- `app/src/ai/mcp/templatable_manager.rs:224` — manager iterators/lookups for active resources and tools.
- `app/src/ai/mcp/templatable_manager/native.rs:839` — successful spawn handler inserts `TemplatableMCPServerInfo` into `active_servers` and emits state changes.
- `app/src/ai/mcp/templatable_manager/native.rs:1909` — `spawn_server` reads peer capabilities and lists resources/tools before constructing server info.
- `app/src/ai/mcp/reconnecting_peer.rs:116` — reconnecting request wrapper used for MCP tool/resource calls and the right pattern for `prompts/get` retry behavior.
- `app/src/ai/blocklist/action_model/execute/call_mcp_tool.rs:107` — MCP tool execution path that resolves the active server and calls through `ReconnectingPeer`.
- `app/src/ai/blocklist/action_model/execute/read_mcp_resource.rs:91` — MCP resource execution path that resolves active resources and calls through `ReconnectingPeer`.
- `app/src/terminal/input/slash_commands/data_source/mod.rs:226` — slash-command data source recomputes active static commands from session, AI, and command-context gates.
- `app/src/terminal/input/slash_commands/data_source/mod.rs:426` — `SyncDataSource` query implementation for static commands and skills.
- `app/src/terminal/input/slash_commands/data_source/saved_prompts.rs:21` — async saved-prompt source that returns `AcceptSlashCommandOrSavedPrompt` menu actions.
- `app/src/terminal/input/slash_commands/data_source/zero_state.rs:124` — Cloud Mode V2 zero-state adds saved prompt rows under the prompt section.
- `app/src/terminal/input/slash_commands/view.rs:46` — slash menu events and accepted item variants for static commands, saved prompts, and skills.
- `app/src/terminal/input/slash_commands/mod.rs:65` — `AcceptSlashCommandOrSavedPrompt`, the action enum shared by slash-menu data sources.
- `app/src/terminal/input/slash_commands/mod.rs:312` — selected saved prompts are dispatched from the slash-menu event handler.
- `app/src/terminal/input/slash_command_model.rs:416` — parsing for typed slash commands based on the active command set.
- `app/src/settings_view/mcp_servers/installation_modal.rs:48` — existing templatable MCP variable input pattern to reuse for MCP prompt arguments.
- `app/src/terminal/input.rs:13072` — queued/typed prompts are routed into existing slash, skill, or Agent Mode submission paths.
- `app/src/terminal/input.rs:13367` — normal Agent Mode prompt submission enters Agent Mode or sends a follow-up in the current conversation.
## Current state
`TemplatableMCPServerManager::spawn_server` connects to an MCP server, reads `server_info.capabilities`, then calls `query_resources_for(... service.list_all_resources())` and `query_tools_for(... service.list_all_tools())`. The returned metadata is stored in `TemplatableMCPServerInfo` and exposed through manager iterators/lookups. The manager emits state changes after a server becomes running or stops, and consumers can subscribe to those model events.
The slash-command menu is not limited to compile-time static commands. It mixes a sync source for static commands/skills, async saved-prompt results, and a zero-state source. Menu selection is represented by `AcceptSlashCommandOrSavedPrompt`, which currently supports static commands, saved prompts, and skills. Typed slash command parsing only considers active `StaticCommand` values, so dynamic MCP prompt commands need a parallel dynamic parse path rather than being added to the static registry.
The existing templatable MCP installation modal already renders one variable input per `TemplateVariable`, supports text inputs/dropdowns, Enter submission, and Escape cancellation. The MCP prompt argument UI should reuse this component pattern, but it needs prompt-specific required/optional validation because `PromptArgument` has different semantics from installation template variables.
## Proposed changes
### 1. Add prompt listing to MCP startup
Extend `app/src/ai/mcp/templatable_manager/utils.rs` with:
- `should_query_prompts(capabilities: Option<&rmcp::model::ServerCapabilities>) -> bool`, returning true only when `capabilities.prompts.is_some()`.
- `query_prompts_for`, mirroring `query_resources_for` and `query_tools_for`, returning `Vec<rmcp::model::Prompt>` and failing soft by logging and returning an empty vector on `rmcp::ServiceError`.
Update `utils_tests.rs` to make the helper `caps(...)` cover tools, resources, and prompts. Keep the independence sweep explicit: each capability should be queried iff that capability is advertised, regardless of the other two. Add prompt-specific tests for skipped/no server info, happy path, empty list, transport error, MCP error, and exactly-once list invocation.
In `native.rs`, import `query_prompts_for` and call it in `spawn_server` after capabilities are available:
- `let prompts = query_prompts_for(capabilities, &server_name, || service.list_all_prompts()).await;`
- Store `prompts` alongside `resources` and `tools` in the returned `TemplatableMCPServerInfo`.
This preserves the current startup behavior: prompt listing cannot make the server fail to start, and servers without the prompt capability do not receive a prompt-list request.
### 2. Store and expose active prompt metadata
Extend `TemplatableMCPServerInfo` in `templatable_manager.rs` with a `prompts: Vec<rmcp::model::Prompt>` field and a `prompts(&self) -> &Vec<rmcp::model::Prompt>` getter.
Add manager-level helpers consistent with the existing resource/tool API:
- `prompts(&self) -> impl Iterator<Item = &rmcp::model::Prompt>` for all active prompts.
- `prompts_for_server(&self, uuid: Uuid) -> Vec<rmcp::model::Prompt>` for a single server.
- `server_with_prompt_name(&self, installation_id: Uuid, prompt_name: &str) -> Option<ReconnectingPeer>` for invocation.
- Optionally `active_prompt_entries(...)` returning `(installation_id, server_name, prompt)` tuples if that makes the slash data source simpler and avoids exposing `active_servers` internals.
The implementation should include prompts from all active server categories already represented by `active_servers`: installed templatable servers, file-based servers that are currently in scope/running, and CLI-spawned ephemeral servers. No separate prompt cache is required in persistence for the first implementation; prompt metadata is runtime state tied to a live server connection.
### 3. Add `prompts/get` request support
Extend `app/src/ai/mcp/reconnecting_peer.rs` with:
- `pub async fn get_prompt(&self, params: rmcp::model::GetPromptRequestParams) -> Result<rmcp::model::GetPromptResult, rmcp::ServiceError>` using the same `with_reconnect_retry` wrapper as `call_tool` and `read_resource`.
Add a small MCP prompt invocation helper, either under `app/src/ai/mcp/templatable_manager/` or in a new `app/src/ai/blocklist/action_model/execute/get_mcp_prompt.rs` if routing through the action model is preferable. The helper should:
- Resolve the selected `installation_id` and `prompt_name` against active manager state at invocation time.
- Build `GetPromptRequestParams` with the prompt name and argument map after local validation.
- Call `ReconnectingPeer::get_prompt`.
- Convert the returned prompt messages to a single text prompt body for Agent Mode.
For the first implementation, conversion should support text content directly and handle unsupported non-text content according to the product spec: do not submit an empty prompt, and avoid silently dropping all returned content. Keep the conversion logic unit-testable separately from UI code.
### 4. Add a dynamic MCP prompt slash-menu source
Create a new data-source module, for example `app/src/terminal/input/slash_commands/data_source/mcp_prompts.rs`, with a snapshot of active prompt entries. It can be a sync source because active prompt metadata is already in memory.
Each entry should carry:
- `installation_id: Uuid`
- `server_name: String`
- `prompt_name: String`
- `prompt_description: Option<String>`
- generated `command_name: String`
- argument metadata copied from `rmcp::model::Prompt`
Generate command names as `/mcp.<server-slug>.<prompt-slug>`. Slug generation should be deterministic, slash-safe, and covered by tests. Keep a per-snapshot map to detect generated-name collisions; append a short stable suffix derived from installation id and prompt name when needed.
Extend `AcceptSlashCommandOrSavedPrompt` with an MCP prompt variant, for example:
- `MCPPrompt { installation_id: Uuid, prompt_name: String, command_name: String }`
Add `InlineItem::from_mcp_prompt(...)` that uses a prompt/dataflow icon, monospace command title, description, match highlighting, and compact-layout support.
Wire this source into both slash menu mixers in `view.rs` and `cloud_mode_v2_view.rs`. For Cloud Mode V2 zero state, either add active MCP prompts in `zero_state.rs` near saved prompts or make the MCP prompt source run in zero state when `is_cloud_mode_v2` is true. `Section::for_action` should classify MCP prompts as `Section::Prompts` so they appear with saved prompts.
### 5. Keep active prompt results fresh
Subscribe `SlashCommandDataSource` or the MCP prompt source owner to `TemplatableMCPServerManager` events. At minimum, trigger a menu query refresh on:
- `TemplatableMCPServerManagerEvent::StateChanged` for Running/NotRunning/Failed transitions.
- `ServerInstallationAdded` and `ServerInstallationDeleted`.
- `TemplatableMCPServersUpdated`.
If needed, introduce a broader `MCPPromptEntriesUpdated` or reuse `UpdatedActiveCommands` to tell existing menu views to rerun their current query. Avoid overloading static `active_commands_by_id` with dynamic prompts; prompt entries are not static commands and should not be stored in `COMMAND_REGISTRY`.
### 6. Parse typed MCP prompt commands
Extend typed slash-command detection so a buffer matching a generated MCP prompt command returns a detected dynamic prompt state. There are two reasonable implementation shapes:
1. Add `SlashCommandEntryState::MCPPromptCommand(DetectedMCPPromptCommand)` and route Enter/Cmd-Enter through a new `execute_mcp_prompt_command` method.
2. Keep `SlashCommandEntryState` focused on static/skill commands and have `Input::maybe_handle_enter_for_slash_command` ask the MCP prompt data source for an exact command match before falling back to plain AI input.
Prefer the first approach because it preserves syntax highlighting, hides the menu after a detected command, and mirrors the skill-command path. The detected state should include the selected installation id, prompt name, generated command name, and no freeform trailing argument; prompt arguments are collected by the form rather than parsed from shell-like text.
### 7. Implement argument collection UI with existing modal patterns
Add a prompt-specific modal/body that reuses the visual and behavioral pattern from `InstallationModalBody` rather than changing the installation modal itself. Reusing shared helper components is fine, but avoid coupling prompt invocation to installation data types.
The prompt argument model should be derived from `rmcp::model::PromptArgument`:
- name/key
- description/help text
- required flag
- optional default if rmcp exposes one in the model version
The form should create text inputs in prompt argument order. Dropdowns are only needed if MCP/rmcp exposes allowed values for prompt args; otherwise all prompt args can be text inputs. Required blank inputs block submission and focus the first invalid field. Escape cancels without calling `prompts/get`.
On submit, build a `HashMap<String, String>` or the exact rmcp argument map type expected by `GetPromptRequestParams`. Omit blank optional values unless rmcp requires explicit empty values.
### 8. Dispatch rendered prompts into Agent Mode
Add an input-level method such as `Input::invoke_mcp_prompt(...)` or `Input::submit_rendered_mcp_prompt(...)` that receives the rendered text from the async invocation helper and sends it through the existing user-initiated Agent Mode submission path:
- If Agent View is inactive, emit `Event::EnterAgentView { initial_prompt: Some(rendered_prompt), origin: AgentViewEntryOrigin::SlashCommand { trigger } }`.
- If an Agent conversation is active, call the same controller path used by `submit_ai_query` for `send_user_query_in_conversation` and emit `Event::ExecuteAIQuery`.
- If the queue-next-prompt setting is active and the current conversation is in progress, use the same queuing behavior as typed prompts rather than bypassing the queue.
Do not introduce a new AI action type unless implementation discovers that agent-visible structured results are required. MCP prompt invocation is user-initiated prompt composition, not an agent-requested tool action, so it can live in input/slash-command handling rather than the blocklist action queue.
### 9. Telemetry and privacy
Extend `SlashCommandAcceptedDetails` or add a dedicated metadata event for MCP prompt acceptance. Include only non-content metadata: generated command family, installation id or redacted stable id, prompt name, has_arguments, argument_count, success/failure category, and whether the command was accepted in Agent View. Do not log argument values or rendered prompt content.
The MCP log may include protocol errors but should not log prompt argument values beyond what rmcp/transport logging already captures. If the invocation helper logs failures, keep messages to server/prompt names and error categories.
## End-to-end flow
1. A user starts or scopes into an MCP server.
2. `spawn_server` connects, reads capabilities, and calls `query_prompts_for` only if prompts are advertised.
3. The resulting prompts are stored in `TemplatableMCPServerInfo` under the active server's installation UUID.
4. The slash menu opens or reruns its query. The MCP prompt data source snapshots active prompt entries and emits menu items named `/mcp.<server>.<prompt>`.
5. The user selects or types an MCP prompt command.
6. If arguments are declared, Warp opens the prompt-argument modal and validates required inputs. If no arguments are declared, Warp proceeds immediately.
7. Warp resolves the active server again, calls `prompts/get` through `ReconnectingPeer`, and converts the result to text.
8. Warp submits the rendered text to Agent Mode as a user prompt. Existing Agent Mode permission checks continue to apply to whatever actions the agent later requests.
## Risks and mitigations
- **Prompt command name collisions:** Use deterministic slugs plus short suffixes for duplicate generated names. Keep original server/prompt names in descriptions so users can distinguish entries.
- **Prompt content may contain prompt injection:** Treat returned content as user-selected prompt text, never as app instructions. Normal Agent Mode permissions remain authoritative.
- **Non-text prompt content:** Keep conversion logic explicit and tested. Do not silently submit an empty prompt if all returned content is unsupported.
- **Stale active-server state:** Resolve the server and prompt again at invocation time. If the server stopped after the menu snapshot, show an error and do not submit.
- **Startup regressions:** Keep `query_prompts_for` fail-soft so prompt listing cannot prevent server startup or hide tools/resources.
- **MCP file-based scope confusion:** Build prompt entries only from active servers already scoped by the manager. Do not separately scan file-based config files for prompts.
- **Telemetry leakage:** Add tests or code review checks around telemetry payload construction to ensure argument values and rendered prompt text are not included.
## Testing and validation
- Unit test `should_query_prompts` and `query_prompts_for` in `utils_tests.rs` alongside tools/resources.
- Unit test triple-capability independence so tools, resources, and prompts are listed only from their own advertised capability flags.
- Unit test prompt slug generation, command collision disambiguation, exact typed-command parsing, and fuzzy matching.
- Unit test MCP prompt data-source gating when AI is disabled and when no prompt-capable servers are active.
- Unit test prompt argument validation and argument map construction for required, optional, blank, and filled values.
- Unit test prompt result conversion for text-only, mixed text/non-text, and unsupported-only returned messages.
- Add model or UI tests for selecting an MCP prompt menu item and routing it to the argument form or immediate invocation path.
- Add a manual validation MCP server fixture, or extend an existing MCP test fixture, to advertise prompts and respond to `prompts/list` / `prompts/get` so the full flow can be verified locally.
- Run the relevant Rust test targets for `app::ai::mcp::templatable_manager::utils_tests`, slash-command data-source/model tests, and any new prompt invocation tests.
## Follow-ups
- Support richer MCP prompt content if Agent Mode adds first-class handling for images, embedded resources, or multi-message role preservation.
- Consider saved/favorite MCP prompts if users want to pin frequently used server prompts independent of server connection state.
- Consider exposing MCP prompt counts or descriptions in Settings → MCP Servers so prompt-only servers do not look empty outside the slash menu.
- Evaluate whether MCP prompt command names should be customizable if generated names are too long or ambiguous for common servers.
