# Multi-team API scope — Tech Spec
## Scope
[`specs/multi-team-context/TECH.md`](../multi-team-context/TECH.md) owns the `TeamContext` type, its ownership rules, and the APIs added to or removed from `UserWorkspaces`.

This spec inventories the network-facing APIs that need attention after that foundation lands. It separates them into four buckets:

1. The operation explicitly identifies a target team in its body or owner, so the raw team UID remains part of its API.
2. The request needs request-local team identity that is not already explicit in the operation.
3. The endpoint's selected-team versus cross-team behavior is unclear and needs a product or server contract decision.
4. An existing resource identifies the team, so the endpoint should remain resource-scoped.

`TeamContext` is required when a GUI request infers its team from the current view and window. An operation that explicitly names team A may continue to accept team A's raw UID; if it also sends `X-Warp-Team-Uid`, the header must come from that same UID. Existing-resource operations prefer resource identity. Team identity stays request-local and is never stored on `ServerApi` or `BaseClient`. The server authenticates membership, rejects body/header or resource/header mismatches, and rejects new team-owned work with no team instead of selecting the first team.
## API buckets
### Bucket 1: Existing team identity on the wire
These operations explicitly identify their target team. They keep a raw team UID rather than exchanging the current window for `TeamContext`.

| Client boundary | Operations | Current wire identity | Required change |
|---|---|---|---|
| [`TeamClient`](https://github.com/warpdotdev/warp/blob/8b88df9874d8d632eba3bcfdd330acc9dabd23b0/app/src/server/server_api/team.rs#L70-L164) | `add_invite_link_domain_restriction`, `delete_invite_link_domain_restriction`, `remove_user_from_team`, `leave_team`, `send_team_invite_email`, `delete_team_invite`, `rename_team`, `reset_invite_links`, `set_is_invite_link_enabled`, `set_team_discoverability`, `set_team_member_role` | `team_uid` | Keep the explicit `ServerId`. Do not replace a team named by the action or settings row with the window's current `TeamContext`. |
| [`WorkspaceClient`](https://github.com/warpdotdev/warp/blob/8b88df9874d8d632eba3bcfdd330acc9dabd23b0/app/src/server/server_api/workspace.rs#L42-L68) | `generate_stripe_billing_portal_link`, `update_usage_based_pricing_settings`, team purchase in `purchase_addon_credits`, `update_addon_credits_settings` | `team_uid`, optional `team_uid`, or legacy `workspace_uid` | Keep an explicit team UID for a billing page or action bound to a named team. Keep the personal add-on-credit form separate rather than using an optional team target. |
| [`FactoryClient::upsert_runner`](https://github.com/warpdotdev/warp/blob/8b88df9874d8d632eba3bcfdd330acc9dabd23b0/app/src/server/server_api/factory.rs#L39-L76) | Team-owned runner creation | GraphQL `Owner` in `UpsertRunnerInput` | Keep the explicit team owner UID. Updates and deletes remain runner-resource-scoped. |
| [`ObjectClient`](https://github.com/warpdotdev/warp/blob/8b88df9874d8d632eba3bcfdd330acc9dabd23b0/app/src/server/server_api/object.rs#L164-L229) | Team branches of workflow, notebook, folder, generic-string-object, and bulk creation; team ownership transfer; team trash operations | `Owner::Team { team_uid }` or equivalent GraphQL owner | Keep the explicit team destination UID. Existing-object mutations remain resource-scoped. |

`join_team_with_team_discovery` is not in this group. It targets a discoverable team that is not yet selected, so its explicit team UID remains a resource identifier.
### Bucket 2: Request-local team identity
These operations need team identity added to the request. A GUI flow inferred from the current window accepts `TeamContext`. An explicitly named target accepts a raw team UID. Documented no-team and resource-owned forms remain separately scoped.

| Request | Current identity | Required contract |
|---|---|---|
| `POST /ai/multi-agent` | [`RequestParams`](https://github.com/warpdotdev/warp/blob/8b88df9874d8d632eba3bcfdd330acc9dabd23b0/app/src/ai/agent/api.rs#L146-L180) has no team scope. The final request has optional conversation, ambient-task, fork, and parent-agent IDs only ([request construction](https://github.com/warpdotdev/warp/blob/8b88df9874d8d632eba3bcfdd330acc9dabd23b0/app/src/ai/agent/api/impl.rs#L109-L151)). | Capture `TeamContext` from the owning terminal view, pass it separately from cloneable `RequestParams`, and send the header on every request. Persist it for new conversations; validate it against existing conversations, ambient tasks, and forks. |
| `POST /ai/passive-suggestions` | Same transport as `/ai/multi-agent` | Require `TeamContext` and send the header on every request, including requests with an existing conversation token. |
| `POST /api/v1/agent/run` | [`SpawnAgentRequest`](https://github.com/warpdotdev/warp/blob/8b88df9874d8d632eba3bcfdd330acc9dabd23b0/app/src/server/server_api/ai.rs#L191-L239) has `team: Option<bool>`, not a team UID. | Replace the ambiguous team boolean at internal call sites with an explicit personal/team scope. The team form always consumes `TeamContext` and sends the header, including child runs and conversation continuations. |
| GraphQL `CreateAgentTask` | [`CreateAgentTaskInput`](https://github.com/warpdotdev/warp/blob/8b88df9874d8d632eba3bcfdd330acc9dabd23b0/crates/graphql/src/api/mutations/create_agent_task.rs#L37-L47) has optional `environment_uid`, optional `parent_run_id`, and config, but no team. | Require `TeamContext` and send the header through a team-scoped GraphQL adapter on every request. The server validates it against a parent run or environment when present. |
| `POST /api/v1/agent/identities` | Agent identity creation has no existing identity resource. | Team-owned creation consumes `TeamContext` and sends the header. Personal creation remains user-scoped. |
| `GET /api/v1/agent/identities` | No selector | Accept `&TeamContext` and send the header on every request. |
| `GET /api/v1/agent/connected-self-hosted-workers` | [`list_connected_self_hosted_workers`](https://github.com/warpdotdev/warp/blob/8b88df9874d8d632eba3bcfdd330acc9dabd23b0/app/src/server/server_api/ai.rs#L2187-L2194) sends no selector, and [`ConnectedSelfHostedWorkersModel`](https://github.com/warpdotdev/warp/blob/8b88df9874d8d632eba3bcfdd330acc9dabd23b0/app/src/ai/connected_self_hosted_workers.rs#L14-L104) has one process-wide result. | Accept `&TeamContext`, send the header on every request, and return workers connected for that team only. Key cached results by team or move them into a team-bound owner; a singleton unkeyed worker list cannot represent several windows. User-authenticated worker daemons select an explicit team, while a service-account principal keeps its assigned team. |
| `POST /api/v1/agent/handoff/upload-snapshot` | [`upload_local_handoff_snapshot`](https://github.com/warpdotdev/warp/blob/8b88df9874d8d632eba3bcfdd330acc9dabd23b0/app/src/server/server_api/ai.rs#L2196-L2203) runs before a task exists and the request contains only file metadata. | Consume `TeamContext` and send the header on every request. |
| GraphQL `TransferTeamOwnership` | [`TransferTeamOwnershipInput`](https://github.com/warpdotdev/warp/blob/8b88df9874d8d632eba3bcfdd330acc9dabd23b0/crates/graphql/src/api/mutations/transfer_team_ownership.rs#L29-L31) contains only the new owner email. | Accept the explicitly targeted team UID and send it in the header. |
| GraphQL `GetFeatureModelChoices` | No selected-team input | Accept `&TeamContext` and send the header because model availability and routing policy can vary by team. Partition cached model choices by team identity. |
| GraphQL `GetAiOveragesForWorkspace` | [`GetAiOveragesForWorkspaceVariables`](https://github.com/warpdotdev/warp/blob/8b88df9874d8d632eba3bcfdd330acc9dabd23b0/crates/graphql/src/api/queries/get_ai_overages_for_workspace.rs#L24-L27) contains only request context; [`WorkspaceClient::refresh_ai_overages`](https://github.com/warpdotdev/warp/blob/8b88df9874d8d632eba3bcfdd330acc9dabd23b0/app/src/server/server_api/workspace.rs#L138-L166) reads the first workspace. | Accept `&TeamContext`, send the header, and return the overages for that team instead of selecting `workspaces.first()`. |
| GraphQL `CreateFileArtifactUploadTarget` | [`create_file_artifact_upload_target`](https://github.com/warpdotdev/warp/blob/8b88df9874d8d632eba3bcfdd330acc9dabd23b0/app/src/server/server_api/ai.rs#L2719-L2777) sends optional `conversation_id` and `run_id`. | Require `TeamContext` and send the header on every request. The server validates it against the conversation or run when present. |
| Managed-secret collection queries | `list_secrets` and `list_harness_auth_secrets` currently have no team selector. | Accept `&TeamContext`, send the header, and return only personal secrets plus secrets for that team. Key harness-secret state by team and harness rather than harness alone. |
| Managed-secret encryption configuration | `get_managed_secret_configs` currently returns the personal config and a map for every accessible team. | Replace it with target-specific personal and team config methods. The team method accepts an explicit team UID and sends it in the header; the personal method sends no team header. Do not return an all-team config map. |
| Team managed-secret mutations | Team create, update, and delete currently serialize `SecretOwner::Team { team_uid }`. | Keep the explicit owner UID and send the same UID in `X-Warp-Team-Uid`. The server rejects a body/header mismatch. Personal-only mutations remain user-scoped and send no team header. |
| API key listing and team creation | `AuthClient::list_api_keys` has no selector and returns mixed owner scopes. Team creation serializes `team_id`. | Listing accepts `&TeamContext`, sends the header, and returns personal keys plus keys owned by that selected team, including keys for agent identities owned by that team. Team creation keeps the explicit team UID, sends the same UID in the header and body, and rejects a mismatch. Personal and agent-identity creation remain separately scoped and send no team header. |
| GraphQL `simpleIntegrations` and `createSimpleIntegration` | The requests contain provider slugs and optional configuration but no team selector. The server currently uses unordered `teamIDs[0]`; `environment_uid` is configuration, not ownership. | Scope list, create, and update to one selected team and send it in `X-Warp-Team-Uid`. GUI callers use `TeamContext`; headless callers use a validated explicit team UID. The server authorizes that team and stops selecting the first membership. |
| GraphQL `getRunners` | [`FactoryClient::get_runners`](https://github.com/warpdotdev/warp/blob/8b88df9874d8d632eba3bcfdd330acc9dabd23b0/app/src/server/server_api/factory.rs#L35-L57) sends no selector and returns personal runners plus runners from every accessible team. Each [`Runner`](https://github.com/warpdotdev/warp/blob/8b88df9874d8d632eba3bcfdd330acc9dabd23b0/crates/graphql/src/api/queries/get_runners.rs#L86-L94) includes its owner scope, but current pickers discard it. | Accept `&TeamContext`, send the header, and return personal runners plus runners owned by that team. Preserve owner scope through the picker. When new work names a runner, the server validates that it is personal or owned by the selected team. Team creation keeps its explicit owner UID and sends the same header; update and delete remain runner-resource-scoped. |
| GitHub repository discovery and preflight | GraphQL `userGithubInfo`, `userRepoAuthStatus`, and `suggestCloudEnvironmentImage` currently send repository names but no team selector through [`IntegrationsClient`](https://github.com/warpdotdev/warp/blob/8b88df9874d8d632eba3bcfdd330acc9dabd23b0/app/src/server/server_api/integrations.rs#L50-L137). This can approve access with user OAuth even when later team work must use the team's GitHub installation. | Give each call an explicit personal or team form. The team form consumes `TeamContext`, sends the header, and makes the server validate repository access through that team's installation grants. Add the server's team UID argument to the client `userGithubInfo` query and preserve installation and repository IDs needed by later requests. The personal form uses user OAuth and sends no team header. |
| `GET /api/v1/agent?repo=...` skills | [`AIClient::list_skills`](https://github.com/warpdotdev/warp/blob/8b88df9874d8d632eba3bcfdd330acc9dabd23b0/app/src/server/server_api/ai.rs#L2472-L2483) sends a repository selector but no owner scope. | The team-skill form accepts `&TeamContext`, sends the header, and resolves the repository through that team's GitHub installation. The personal form uses user OAuth and sends no header. Skill loading for an existing task remains task-scoped. |

For `/ai/multi-agent`, do not place `TeamContext` inside `RequestParams`: that type is cloned and represents the serializable model request. Pass the capability as a separate owned argument through `app/src/ai/agent/api/impl.rs`; borrow it only while the multi-agent client builds the HTTP request.

Task-secret retrieval and workload identity-token issuance remain task-scoped. They use task and workload identity rather than an interactive selected team.
#### `oz secret` team selection
The headless CLI has no `ViewContext`, so its team-inclusive operations use an explicit raw team UID:

- With no teams, only personal operations are available and no team header is sent.
- With one team, team-inclusive collection queries and team mutations use that team.
- With more than one team, team-inclusive collection queries and team mutations require `--team <team-uid>`.
- `--personal` operations do not require `--team` and send no team header.
- The selected UID should resolve to a current membership before the request is sent. The server remains responsible for authorization.

Change the current boolean `--team` flag to a UID-valued option. Do not use `sole_team_uid()` when the user has multiple teams.
#### `oz integration` team selection
Slack and Linear simple integrations are team-owned. Their CLI commands use an explicit raw team UID:

- With one team, list, create, and update use that team.
- With more than one team, list, create, and update require `--team <team-uid>`.
- With no teams, these commands are unavailable.
- The selected UID should resolve to a current membership before the request is sent. The server remains responsible for authorization.

The server stores Slack and Linear configuration under a team-owned OAuth connection. An optional environment remains configuration within that team scope and does not select the owner.
#### `oz runner` team selection
Runner listing and creation follow the same personal-plus-selected-team contract as secrets and API keys:

- With no teams, listing returns personal runners and creation defaults to personal ownership.
- With one team, a team-inclusive list may use that team.
- With more than one team, a team-inclusive list requires `--team <team-uid>`.
- Team creation requires an explicit team UID and sends the same UID as its owner and header.
- Personal creation sends no team header.
- Update and delete resolve an existing runner and remain resource-scoped.

The selected UID must resolve to a current membership before the request is sent. Do not drop `Runner::scope` when a list result enters a picker or name-to-UID resolver.
#### Repository and GitHub team selection
Repository discovery must use the credentials that later work will use:

- A new team-owned environment or team skill consumes `TeamContext` in the GUI and sends `X-Warp-Team-Uid`.
- A headless team-owned environment or team-skill operation requires `--team <team-uid>`.
- A personal environment or personal skill uses user OAuth and sends no team header.
- An existing environment or task derives its team from that resource instead of the current window or CLI default.

The server confirms that the selected team's GitHub installation grants access to every requested repository. A successful user-OAuth check is not sufficient for work that will execute with team credentials.
### Bucket 3: Scope contract unresolved
The following APIs have no selector, but code inspection alone cannot determine whether they should use the selected team or aggregate across all visible teams. Their product contract must be decided before changing signatures.

| API | Decision required |
|---|---|
| Harness-support external conversation creation | Decide whether the ambient task always supplies resource scope. If not, new external conversations need `TeamContext`. |

Until each decision is made, these methods must not select `teams.first()` or infer from a process-global current workspace.
### Bucket 4: Existing resource determines the team
Do not add `TeamContext` to an operation whose resource already determines team ownership. This avoids combining a live window selection with a stable resource identity.

The following stay resource-scoped:

- Conversation metadata, rename, delete, and transcript operations by conversation ID. Inference follow-ups and forks through `/ai/multi-agent` remain in bucket 2.
- Agent run follow-ups, events, messages, client events, and cancellation by run or task ID.
- Agent identity update and delete by identity UID.
- Artifact confirmation and download by artifact UID.
- Attachment, credential, and handoff retrieval by task ID.
- Memory and memory-store mutations by store or memory UID.
- Runner update and delete by runner UID.
- Runner lookup for an existing run by runner UID.
- GitHub access and skill loading for an existing environment or task. `getIntegrationsUsingEnvironment` must derive the team from its environment UID rather than selecting the caller's first team.
- Existing Drive object update, move, share, trash, and delete operations by object UID, except when selecting a new team destination.

If a selected-team header is also present for defense in depth, the server must validate it against the resource and reject a mismatch. It must never use the header to override resource ownership.
## Testing and validation
### Transport behavior
- For each explicit-target operation, assert that the body, owner, and optional header all use the same supplied raw UID.
- For each current-window operation, assert that the header or body uses the UID captured in the supplied `TeamContext`.
- Add request tests for every bucket 2 GraphQL, public REST, and multi-agent path that requires `X-Warp-Team-Uid`.
- Add negative server tests for missing team, unauthorized team, body/header mismatch, and resource/header mismatch.
- Verify two concurrent windows can send requests for different teams through the same `ServerApi` instance without cross-contamination.
### Endpoint behavior
- New `/ai/multi-agent` conversation: header is present and the created conversation records that team.
- Existing conversation follow-up: the header is always present and the server rejects a mismatch with the stored team.
- `/agent/run` and `CreateAgentTask`: the team form always requires context, including when a parent resource also establishes scope.
- Billing, invites, roles, team-owned secret mutations, API key creation, runners, and Drive destinations use the explicitly supplied team UID.
- Managed-secret collection and harness-auth queries return personal secrets plus one selected team's secrets and always send that team header.
- A multi-team `oz secret` invocation fails before sending a team-inclusive request unless `--team <team-uid>` is present; personal-only mutations remain valid without it.
- API key listing returns personal keys plus one selected team's direct and agent-identity keys. Headless listing follows the same zero-, one-, and multi-team selection rules as `oz secret`.
- Team API key creation sends the same explicit team UID in its header and body. Personal and agent-identity creation remain separately scoped.
- Simple integration list, create, and update use one selected team; a multi-team `oz integration` invocation requires `--team <team-uid>`.
- Runner listing returns personal runners plus one selected team's runners and preserves each runner's owner scope. A multi-team `oz runner list` invocation requires `--team <team-uid>` for a team-inclusive result.
- New work rejects a runner owned by a different team. Runner update, delete, and existing-run lookup remain valid after the window changes teams.
- Connected-worker results for team A never appear in team B's picker or overwrite team B's cached results.
- Team repository discovery, authorization preflight, image suggestion, and skill lookup send the selected team header and validate that team's GitHub installation grants. Equivalent personal operations send no team header.
- Existing-environment and existing-task repository operations continue after the window changes teams and use the resource's team.
- Resource-scoped operations continue after the window switches teams and still target the original resource team.
### Repository validation
- Run `./script/format`.
- Run the focused transport, billing, Agent Mode, and team administration tests.
- Run `cargo check -p warp --lib`.
- Run the `cargo clippy` command used by `./script/presubmit`.
- Run `git diff --check`.
## PR breakdown and parallelization
### Dependency: PR 0 TeamContext foundation
Owned by [`specs/multi-team-context/TECH.md`](../multi-team-context/TECH.md). This PR introduces the capability and temporary compatibility needed for downstream migrations. It is the only blocking client dependency.

### Group 2: API migrations

These are sub-PRs of the second rollout group in the companion TeamContext spec. They begin after additive PR 0 and after their client call-site ownership dependencies are ready.

#### PR 2A: Team administration and billing
Preserve explicit UIDs throughout the `TeamClient` and `WorkspaceClient` rows in bucket 1. Add the explicit header UID for `TransferTeamOwnership`, and add `TeamContext` only to the current-window `GetAiOveragesForWorkspace` query. This PR owns team settings and billing call sites, generated mocks, and focused tests.

#### PR 2B: Agent creation and cost attribution
Add request-local team propagation for `/ai/multi-agent`, `/agent/run`, `CreateAgentTask`, agent identity creation, connected workers, handoff snapshots, feature model choices, and artifact upload targets. This PR owns the GraphQL, public REST, and multi-agent transport changes needed for `X-Warp-Team-Uid`, team-keyed connected-worker state, and runner-owner validation when new work is created.

The corresponding warp-server support can proceed in parallel with PR 0 after the header and validation contract is agreed. The client PR must not ship before the server accepts and validates the new scope.

#### PR 2C: Managed platform resources
Use `TeamContext` for current-window managed-secret, API-key, and runner collection queries. Preserve explicit UIDs for team-owned secret mutations, encryption configuration, API-key creation, runner creation, `oz secret --team`, and `oz runner --team`. This PR owns the personal-plus-selected-team response contracts, owner-scope preservation, team-and-harness cache partitioning, CLI membership validation, and focused request tests. Personal-only mutations and task-scoped retrieval remain unchanged.

#### PR 2D: Simple integrations
Add selected-team scope to Slack and Linear integration listing, creation, and updates. This PR owns the `oz integration` selector, client header propagation, server membership and admin authorization, removal of `teamIDs[0]`, and focused multi-team tests. Server support can proceed in parallel with PR 0 after the header contract is agreed.

#### PR 2E: Warp Drive creation and destinations
Preserve explicit team destination UIDs for team-owned workflow, notebook, folder, generic-string-object, and bulk creation, plus ownership transfers. Existing-object operations remain resource-scoped. Add a team header only where the server contract requires one, using the same explicit destination UID.

#### PR 2F: Repository and GitHub scope
Add explicit personal and team forms for GitHub repository discovery, authorization preflight, image suggestion, and repository-backed skill listing. This PR owns `userGithubInfo` schema alignment, selected-team header propagation, team-installation grant validation, headless team selection, and resource-derived behavior for existing environments and tasks.

PRs 2A, 2B, 2C, 2D, and 2E can run in parallel once their client call-site dependencies are ready. They own separate API traits and feature areas. PR 2B owns shared request-header plumbing; the other PRs should not add competing generic transport helpers. A later cleanup PR owned by the companion TeamContext spec removes temporary `UserWorkspaces` compatibility after these migrations; it is not part of additive PR 0.
## Risks and mitigations
- **Accidental ambient team state**: storing a team on `ServerApi` would race across windows. Keep `TeamContext` or the explicit target UID as a method argument and request-local header.
- **Raw UID used as an ambient escape hatch**: explicit-target APIs remain appropriate only when the action, owner, or destination already names the team. Current-window callers must not switch to those APIs merely to avoid carrying `TeamContext`.
- **Header/body disagreement**: sending both without validation creates two sources of truth. The server rejects mismatches.
- **Resource reassignment by header**: a header must not change the owner of an existing resource. Resource ownership remains authoritative.
- **Cross-team collections mistaken for selected-team collections**: runner, secret, and API-key pickers return personal plus one selected team. APIs intended to remain aggregate must retain owner metadata and state that contract explicitly.
## Follow-ups
- Add a typed server-client team-scope primitive if more crates need request-local team headers.
- Resolve whether harness-support external conversation creation always has ambient-task scope.
