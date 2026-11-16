# Enterprise BYOK / BYOE

## Summary
Let enterprise teams use their own LLM providers in Warp. A team admin configures shared API keys and OpenAI-compatible custom endpoints (e.g. OpenRouter, LiteLLM, Samba) once in the Warp Admin Panel, and those providers appear automatically in every member's model picker. Admins can also choose whether members may add their own local keys and endpoints. Team-managed provider keys are stored by Warp, so they work for both interactive agent requests and Oz cloud agents; member-managed providers are stored locally in secure storage (the current art), and work only for interactive requests

## Problem
Today BYOK and BYO-endpoint are self-serve only: each user pastes keys locally, nothing is stored, and those providers can't be shared across a team or used by cloud agents. Enterprises want to (a) provision approved providers for the whole team centrally, (b) optionally let members bring their own, and (c) be confident inference actually goes to their providers.

## Goals
- A team admin configures shared keys and custom endpoints once; they appear for all members.
- Admins control whether members may add their own keys/endpoints.
- Team-managed providers work for both interactive agent requests and Oz cloud agents. User-managed providers work for interactive agent requests.
- Make it obvious how to keep inference on the team's providers (turn off Direct API and Warp credit fallback).
- Leave the existing self-serve BYOK/BYOE experience unchanged for non-enterprise users.

## Non-goals
- Member-managed (local, device-only) providers powering cloud agents.
- Changing self-serve (non-enterprise) BYOK/BYOE behavior.

## Design
A [Loom walkthrough](https://www.loom.com/share/6eb6e0f8f0764d2a9e8ec4f886957e63) with basic flows. Treat the Loom as the visual reference for the admin section and the member settings layout; this Behavior section is the source of truth for behavior.

## Behavior

### Roles and terms
- "Team admin" is a member with admin/owner permissions; only admins see and edit the Admin Panel. 
- "Team member" is any member; members see the in-app settings surface. 
- A "team-managed" provider is a key or endpoint an admin configured in the Admin Panel. Its secret is stored server-side by Warp and never synced to member devices. For team-managed custom endpoints, the member's client references the selected model by a server-owned `config_key`; the server resolves that reference and injects the stored endpoint secret at request time. For team-managed first-party API keys, members select the normal Warp model ID, and the server applies the team key based on the model's provider when no user key overrides it.
- A "user-managed" provider is a key or endpoint a member added in their own Warp settings - stored only on that member's device (OS secure storage). The client attaches the secret to each interactive request, where the server uses it transiently and never persists it (current BYOK/BYOE for self-serve behaviour). 

### Admin Panel — Models page (Enterprise Team Admin)
- The Models page gains a "Bring Your Own Keys & Endpoints" section alongside the existing Direct API and AWS Bedrock sections. The section has a master enable toggle. When off, no team-managed providers are active for the team and the section's configuration controls are hidden/disabled. When on, the admin can configure team keys and endpoints and they become available to members. When the admin enables team-managed BYO (adds keys/endpoints) while Direct API is still enabled, the section surfaces a prominent prompt explaining that members can still select Warp-managed models, with a one-click affordance to turn off Direct API.
- Team API keys: the admin can paste a key for each first-party provider Warp supports for BYOK (e.g. OpenAI, Anthropic, Google). Each provider row shows whether a key is currently set without revealing the stored value. Saving a key persists it for the team; clearing a row removes that team key.
- Team custom endpoints: the admin can add one or more OpenAI-compatible Chat Completions endpoints. Each endpoint has a name, URL, API key, and one or more models, where each model has a model name (sent to the endpoint) and an optional alias (shown to members). The admin can add, edit, and remove endpoints.
  - Endpoint validation: an endpoint cannot be saved without a name, a valid URL, an API key, and at least one model with a non-empty model name. Invalid fields are indicated inline and block saving (same behaviour as current client). 
- "Allow users to bring their own models" toggle: when on, members may add their own local keys and custom endpoints in their Warp settings; when off, the member-facing self-serve BYO UI is disabled.
- Cloud-agent note: the section states that team-managed keys and endpoints are stored by Warp and are therefore available to Oz cloud agents, unlike member-managed providers which never leave the member's device.
- Saving any team-managed configuration propagates to members: the next time a member's client loads team settings, the team-managed providers (and the "allow users" permission) reflect the admin's latest saved state.
- Validation errors from saving (e.g. malformed endpoint) are shown to the admin and the prior saved state is preserved until a valid save succeeds.

### Team member — Warp settings surface
- The member's AI/Custom Inference settings present two clearly distinct groups: "Provided by your team" (team-managed, read-only) shown first, and "User added keys" (the member's own, editable) shown below. Both groups use the same visual layout (provider key rows and endpoint cards). _The API key section of the "Provided by your team" will not show the redacted API key, instead just whether the API key is configured (checkmark) or not, and whether it's active or overridden by the "user added keys"_. 
- The "Provided by your team" group lists the enabled team-managed providers and team-managed endpoints (name + model chips). Providers or models the admin has disabled do not appear here, mirroring the picker (disabling a model in the Admin Panel removes it from the member's picker automatically). It is read-only: members cannot edit, add, or remove these entries.
- The team group includes a short explanation that these were configured by the team admin, are shared with everyone, and also power cloud agents.
- The "User added keys" group is the existing self-serve BYOK/BYOE experience: paste provider keys, and add/edit/remove custom endpoints (name, URL, API key, model name + alias).
- When the admin's "Allow users to bring their own models" is off, the "User added keys" group is visibly disabled (controls non-interactive and dimmed). Any keys/endpoints the member previously saved locally are not editable while disabled, but will persist in the event the enterprise admin switches this setting back on.
- When "Allow users to bring their own models" is on, the "User added keys" group is fully interactive as it is for self-serve users today (excluding the SuperGrok toggle). 

### Model picker and routing behaviour/logic
- The server-provided model list includes Warp-managed models plus any team-managed custom endpoint models. Team-managed custom endpoint models are treated like normal server-provided picker choices: they have a stable model ID (`config_key`) and display name, but no endpoint URL or API key is sent to the client.
- User-managed custom endpoint models are local to the member's device. When "Allow users to bring their own models" is on, the client unions those local custom models with the existing server-provided model list.
- Custom-endpoint models are distinct picker entries, even when they share the same display alias as a Warp-provided model or another endpoint model. Team-provided endpoints are shown as `<model alias> (Team • <endpoint name>)`; user-provided endpoints are shown as `<model alias> (Custom • <endpoint name>)`. i.e. we are doing Team provided custom endpoints `UNION` User provided custom endpoints.
- First-party provider keys (OpenAI/Anthropic/Google) never create duplicate picker entries: a member's own key for a provider takes precedence over the team key for that provider, so the standard model appears once — routing through the member's key when set, otherwise the team key, otherwise Warp-managed inference if allowed. This precedence is automatic.
- The picker and active model toolbar must make the active source clear for first-party models and custom endpoints. For example, a hosted Anthropic row should indicate whether inference uses the member's key, the team's key, or Warp-managed inference; custom endpoint rows should indicate whether they are a team endpoint or a user-added custom endpoint.

### Cloud agents (Oz)
- Team-managed keys and endpoints are usable by Oz cloud agents for that team: a cloud agent run can perform inference through the team's configured providers without any per-member device state.
- Member-managed (local) keys and endpoints are never used by cloud agents, only interactive requests.
- The product communicates this distinction in both surfaces (admin "available to cloud agents" note, member "interactive agents only" note).

### States, edge cases, and invariants
- Non-enterprise/self-serve users see no change: the existing single BYOK/BYOE settings experience is preserved, with no "Provided by your team" group.
- Disabling the section master toggle (3) removes team-managed providers from members' pickers and hides the member "Provided by your team" group on their next settings/picker load.
- Turning "Allow users to bring their own models" off does not delete a member's locally stored keys/endpoints; it disables the UI and prevents their selection/use until re-enabled.

### How team-managed providers are stored, synced, and injected
This is meant to just be a high-level overview for alignment, more details will be in the tech spec. 

Team-managed configuration is split into public metadata and secrets.
- **Public metadata** syncs from the server down to every member's client through the existing workspace-settings channel and never includes secret values. What syncs differs by provider type:
  - **BYOK (OpenAI/Anthropic/Google):** just a per-provider boolean — whether the team has a key configured. The provider is an already-known, built-in identity, so no value or id is synced. That boolean drives the member's "configured" checkmark and tells the picker which standard models can route through a team key.
  - **Custom endpoints:** The server includes each team endpoint model in the normal model list sent to the client, using the model's server-owned `config_key` as the picker ID. Public endpoint/model metadata such as endpoint name and model display name may also sync so the client can label the row as team-provided. Endpoint URL and API key are withheld.
  This metadata is what lets a member's model picker show team-provided models and source labels without the client ever holding a key.
- **Secrets**: First-party provider keys and endpoint API (+ URL) keys are stored server-side, scoped to the team. They are never synced to clients. At request time the server resolves that reference against the team's stored secrets for the authenticated member, injects the matching key, and routes — at the same boundary that redacts secrets from logs. Two reference paths:
- **First-party keys**: The server resolves by provider and priority: a user key present on the request wins; otherwise the team's stored key for that provider is injected.
- **Custom endpoints** Use a stable per-model reference id (the same `config_key` that maps a model selection back to its provider currently). A user endpoint's request carries both the selected model's reference id and a provider entry holding its URL + API key; a team endpoint's request carries only the reference id, and the server fills in the stored URL + API key for the team endpoint that owns it. For team endpoints this `config_key` is minted and owned server-side and travels to clients as public metadata. Resolution checks the request's own provider entries first (user endpoint, secret present), then the team's stored endpoints by reference id, so a user endpoint and a team endpoint with the same name never collide.


## Open Questions

- Should admins be able to prevent team members from adding their own API key and Custom Endpoints separately? (i.e. it's okay for users to add their own API key, but it's not okay for them to add their own Custom Endpoint). Do we want separate toggles, or should we just treat these the same? **My vote**: Unless enterprise demand for this is strong, I think both being under the same toggle makes more sense / aligns more with what we already have in the backend for the immediate sprint.
- Should we default to "Allow users to bring their own models" for a newly enabled team? **My vote**: No, we should not. 
