# MCP Prompts Capability — Product Spec
GitHub issue: https://github.com/warpdotdev/warp/issues/10740
Figma: none provided. The argument-entry UI must reuse the existing templatable MCP installation variable input pattern rather than introducing a new design surface.
## Summary
Warp should treat MCP server prompt templates as a first-class MCP capability alongside tools and resources. When an MCP server advertises the `prompts` capability during initialization, Warp lists that server's prompts, makes them discoverable in the slash-command menu, lets the user invoke a prompt by selecting or typing its generated slash command, collects any declared arguments, calls `prompts/get`, and submits the rendered prompt into Agent Mode.
The desired outcome is that prompt-focused MCP servers no longer appear connected but empty in Warp. Users should be able to discover and use MCP prompts without manually copying prompt names or leaving the terminal input flow.
## Problem
Warp currently connects to MCP servers and lists `tools` and `resources` when those capabilities are advertised, but it does not query or surface `prompts`. Prompt-only servers can therefore look broken in Warp: they can be connected and useful according to the MCP protocol, but the user sees no available MCP surface in the app.
This also creates an inconsistent MCP mental model. Users can use MCP tools through Agent Mode and MCP resources through resource reads, while MCP prompt templates are invisible even though they are part of the same capability negotiation model.
## Goals
- Query `prompts/list` only for MCP servers that advertise the `prompts` capability during initialization.
- Store prompt metadata on active MCP server state so prompt discovery can reuse the same connected-server lifecycle as tools and resources.
- Surface active MCP prompts in the slash-command menu as generated commands using the `/mcp.<server>.<prompt>` naming family.
- Allow users to invoke an MCP prompt by selecting it from the slash menu or typing the generated command exactly.
- For prompts with declared arguments, show a lightweight argument form using the existing templatable MCP installation variable input affordance, including required/optional handling.
- Call `prompts/get` with the populated argument values and submit the returned prompt text to Agent Mode as a user-initiated prompt.
- Preserve existing AI and MCP safety controls: no prompt should execute when AI is disabled, when the source MCP server is not currently active, or in a way that bypasses normal Agent Mode permissions.
- Fail soft when listing prompts fails, matching the existing tools/resources behavior: the MCP server should still start and its other capabilities should remain usable.
## Non-goals
- Building a new prompt-template designer or new modal design system for MCP prompt arguments.
- Editing, saving, syncing, or sharing MCP prompts as Warp Drive prompts.
- Adding a new Settings page for MCP prompts.
- Changing MCP tool-call, resource-read, or permission semantics.
- Automatically invoking MCP prompts without a user selecting or typing a generated command.
- Supporting arbitrary rich prompt message rendering beyond what Agent Mode can accept as text in the initial implementation. Rich content support can be added later if needed.
- Changing prompt data on the MCP server. Warp only lists prompts and calls `prompts/get`; it does not create, update, or delete prompts.
## User experience
1. When a user has one or more running MCP servers that advertise prompts, opening the slash-command menu includes MCP prompt entries in addition to built-in slash commands, skills, and saved prompts.
2. Each MCP prompt entry displays a generated command name in the format `/mcp.<server-slug>.<prompt-slug>`. The entry description uses the prompt description from `prompts/list` when present. If no description is present, the entry should indicate the server that provides it rather than showing an empty row.
3. Server and prompt slugs are deterministic and stable for the active server session. They should be lowercased, whitespace-normalized, and safe to type in a slash command. If two active prompts would produce the same command name, Warp keeps both discoverable by appending a short deterministic disambiguator to the later command names and showing the original server/prompt names in the row description.
4. MCP prompt entries are grouped with prompt-like items in Cloud Mode V2's slash menu. Existing saved prompts remain visible and continue to behave as saved prompts.
5. MCP prompt entries are hidden when AI is globally disabled. File-based MCP prompts are visible only when their backing file-based MCP server is active under the existing file-based MCP enablement and working-directory scoping behavior.
6. MCP prompt entries update as servers start, stop, reconnect, or change active scope. A stopped server's prompts are removed from the menu. A restarted server's prompts reappear after the next successful `prompts/list` call.
7. Selecting an MCP prompt with no declared arguments immediately invokes it. Warp calls `prompts/get` for the selected server/prompt, converts the returned text prompt messages into the Agent Mode prompt body, and sends that text as a user-initiated Agent Mode prompt.
8. Selecting an MCP prompt with one or more declared arguments opens an argument-entry form before calling `prompts/get`. The form reuses the existing templatable MCP installation variable input pattern: a compact modal with one input per variable, Escape to cancel, and Enter or the primary action button to submit.
9. Argument fields display the MCP argument name and, when present, its description. Required arguments are visually distinguishable and must be filled before submission. Optional arguments may be left blank; blank optional arguments are omitted from the `prompts/get` arguments map unless the MCP SDK requires explicit empty strings.
10. If argument submission fails validation, Warp keeps the form open and focuses the first invalid required field. No request is sent to the MCP server until local validation passes.
11. If `prompts/get` succeeds, Warp closes the argument form, clears the generated slash command from the input buffer if it was typed there, and submits the rendered prompt into Agent Mode. If no Agent conversation is active, Warp enters Agent Mode and starts a new conversation with the rendered prompt. If a conversation is active, Warp submits the rendered prompt as a follow-up using the same user-initiated path as typed prompts.
12. If the selected MCP server disconnects before invocation completes, Warp shows an error toast and does not submit a partial prompt. If reconnection support can transparently reconnect the server, Warp may retry once using the existing reconnecting peer behavior before showing the error.
13. If `prompts/get` returns an MCP error, Warp shows a concise error toast that names the prompt and server. The prompt is not submitted to Agent Mode.
14. If `prompts/get` returns only unsupported non-text prompt content, Warp shows an unsupported-content error and does not submit an empty prompt. If it returns both text and unsupported content, Warp submits the supported text and includes clear placeholder text for omitted content only if that is less confusing than failing the whole invocation.
15. MCP prompt text is treated as user-selected prompt content for the agent, not as trusted application instructions. It must not bypass Agent Mode permission prompts, MCP tool permissions, command allow/deny lists, or any other execution-profile policy.
16. Telemetry may record non-content metadata such as command source, server installation id, prompt name, whether arguments were present, and success/failure. Telemetry must not include argument values or rendered prompt content.
17. Prompt listing failures are not user-blocking at startup. If `prompts/list` fails, the server still appears running and any listed tools/resources remain available. The failure may be logged in MCP logs and omitted from the slash menu.
18. Prompt invocation is explicit. Merely opening the slash menu or filtering MCP prompt entries must not call `prompts/get`.
## Success criteria
- Prompt-only MCP servers that advertise `prompts` no longer appear functionally empty in Warp; their prompts can be found from the slash menu after the server connects.
- A user can type `/mcp` and see matching MCP prompt entries for active prompt-capable servers.
- A user can select a prompt with no arguments and see the rendered prompt submitted to Agent Mode without manually copying or editing it.
- A user can select a prompt with required arguments, fill them in through the existing MCP variable input-style form, and submit a `prompts/get` request with the expected argument map.
- Required arguments cannot be submitted blank; optional arguments can be omitted.
- Prompt entries disappear when their server stops and reappear when it reconnects and prompt listing succeeds.
- Servers that advertise tools/resources but not prompts do not receive `prompts/list` calls.
- Servers that advertise prompts but fail `prompts/list` still start and remain usable for other capabilities.
- AI-disabled users do not see or invoke MCP prompt entries.
- Prompt invocation does not leak argument values or prompt text through telemetry.
## Validation
- Unit tests cover capability gating for tools, resources, and prompts independently across all advertised-capability combinations.
- Unit tests cover `query_prompts_for` happy path, skipped path, empty-list path, and fail-soft MCP/transport errors.
- Unit tests cover generated slash-command names, duplicate disambiguation, filtering, and parsing of typed MCP prompt commands.
- Unit tests cover argument validation for required and optional `PromptArgument` values.
- Unit tests or model tests cover dynamic menu updates when MCP server state changes.
- An integration or manual validation flow uses a test MCP server that advertises prompts only, confirms prompt entries appear in the slash menu, invokes a no-argument prompt, invokes a required-argument prompt, and verifies the rendered text reaches Agent Mode.
- Manual validation confirms a prompt-capable server with a failing `prompts/list` still starts and exposes any tools/resources it lists successfully.
## Open questions
- Should mixed text and non-text prompt content submit best-effort text with placeholders or fail the invocation until rich prompt content support is designed? The initial spec prefers avoiding empty/ambiguous submissions, but implementation should choose the least surprising behavior once rmcp's returned content shapes are inspected.
- Should MCP prompt invocations be included in the same analytics bucket as saved prompts or receive a distinct slash-command accepted detail? The product requirement is metadata-only telemetry; the exact event taxonomy can be decided during implementation.
- Should the generated command name include the raw prompt name when it is already slash-safe, or always use the normalized slug for consistency? The spec requires deterministic slash-safe names either way.
