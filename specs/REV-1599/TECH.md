# REV-1599 — Gemini Enterprise (Vertex) BYOLLM client implementation

This document records the **shipped desktop-client implementation** of Gemini Enterprise Agent Platform (GEAP) BYOLLM. It is a posterity reconciliation of client PRs [#12522](https://github.com/warpdotdev/warp/pull/12522), [#12537](https://github.com/warpdotdev/warp/pull/12537), [#12581](https://github.com/warpdotdev/warp/pull/12581), [#12684](https://github.com/warpdotdev/warp/pull/12684), and [#14141](https://github.com/warpdotdev/warp/pull/14141). The server-side API, routing, model registry, and billing contract are specified in `warp-server/specs/REV-1599/TECH.md`.

The client implementation supports **local interactive desktop Agent Mode**. Oz/cloud-agent credential minting, per-model location overrides, and an admin-facing setup panel are not client features in this release.

## Architecture summary

For a signed-in desktop user, the client obtains a short-lived Google Cloud access token without reading local Google credentials:

1. The client calls `IssueTaskIdentityToken` through the existing authenticated Warp GraphQL session.
2. warp-server returns a Warp-signed OIDC JWT whose audience is the configured WIF provider resource name.
3. The client exchanges that JWT at Google Security Token Service (STS) using RFC 8693.
4. If `gcpSaEmail` is configured, the client calls IAM Credentials `generateAccessToken` to impersonate that service account. Otherwise it uses the federated STS token directly.
5. The resulting credential is held in memory and attached to eligible Agent Mode requests.

There is no `gcloud auth application-default login` step, local ADC file, `yup-oauth2`, Google refresh token, service-account key, OAuth consent flow, or persisted GEAP credential. The existing Warp session is the root credential for every mint.

The engine is compiled out of the WASM client (`app/src/ai/mod.rs`) and runs only on desktop platforms. Cloud-agent support is a follow-up.

## Feature and workspace gates

### Client feature rollout

GEAP is gated twice in the client:

- The `gemini_enterprise` Cargo feature in `app/Cargo.toml` compiles the `FeatureFlag::GeminiEnterprise` variant.
- The runtime `FeatureFlag::GeminiEnterprise` controls behavior. It is included in `DOGFOOD_FLAGS`; it is not enabled by the Preview or Stable flag lists in the shipped client.

The client does not add a second per-request rollout flag. Server-side GEAP availability and minimum-client compatibility determine whether the workspace exposes the host and models. The additive `GEMINI_ENTERPRISE` GraphQL enum value is response deserialization plumbing; it is not a query-text deployment-ordering hazard.

### Workspace configuration synchronized to the client

`GEMINI_ENTERPRISE` is a first-class `LlmModelHost`/`LLMModelHost` value in the vendored GraphQL schema, cynic query types, and app model. The existing workspace settings query supplies the `GEMINI_ENTERPRISE` `LlmHostSettings` entry:

- `enabled` — whether the workspace has enabled this host.
- `enablementSetting` — `ENFORCE` or `RESPECT_USER_SETTING`.
- `gcpAudience` — the full workload identity provider resource name. It is used as both the Warp JWT audience and the STS `audience` parameter.
- `gcpSaEmail` — the optional service account email to impersonate after STS. Empty or absent means direct federated-token use.

The client does not consume the server-only `gcpProjectId` or `gcpLocation` fields. Those determine the customer Vertex destination on the server. The app-side `LlmHostSettings` stores the new optional fields with serde defaults so older local workspace caches remain readable.

`UserWorkspaces` exposes the host settings and computes availability from the workspace-wide `llmSettings.enabled` switch plus the GEAP host's `enabled` value. A missing or disabled host never mints or attaches credentials.

### Member enablement

`AISettings::gemini_enterprise_credentials_enabled` is a desktop, cloud-synced setting with a default of `false`. The effective gate is:

1. `FeatureFlag::GeminiEnterprise` is enabled.
2. The user is signed in (anonymous and logged-out users are always disabled because there is no Warp session from which to mint).
3. Workspace AI settings and the GEAP host are enabled.
4. `ENFORCE` enables credentials for every member and makes the member toggle non-editable.
5. `RESPECT_USER_SETTING` consults the member's toggle; the default is opt-in.

`enablementSetting` controls this **client enablement and toggle behavior**. It is not a client-side server fallback policy. The server independently chooses a route from host/token availability and its fixed host priority.

## Credential data model

### In-memory credential types

The shared `ai` crate defines:

- `GeapCredentials { access_token, expires_at }`, with private token material. `Debug` redacts the token, and `access_token_for_request()` is the only token egress.
- `GeapFederation::DirectWif` or `GeapFederation::ServiceAccount { email }`.
- `GeapMintBinding { user_uid, audience, federation }`. A loaded token is valid for attachment only when this exact binding matches the current signed-in user and current workspace configuration.
- `LoadGeapCredentialsError::{MintIdentityToken, ExchangeToken, ImpersonateServiceAccount}`. Exchange and impersonation errors retain an optional HTTP status and structured detail for classification; raw detail is not shown in user-facing copy.

`GeapCredentialsState` is held by the process-wide `ApiKeyManager` and is never persisted:

```text
Missing
Disabled
Unconfigured
Refreshing { previous: Option<(GeapCredentials, GeapMintBinding)> }
Loaded { credentials, loaded_at, minted_for }
Failed { error: LoadGeapCredentialsError }
```

`Missing` is the initial/cold-start state. `Disabled` means the effective feature, auth, workspace, or member gate is off and any held token is dropped. `Unconfigured` is the shipped state for an enabled GEAP host with a missing or blank `gcpAudience`; it directs the member to the workspace admin. `Refreshing` carries the previous token only when it still matches the current binding. `Loaded` records when the token arrived and the binding used to mint it. `Failed` records a first-mint failure or any forced-mint failure; a failed non-forced background refresh instead restores a usable previous token and parks the chain.

## Mint and refresh lifecycle

### Mint sequence

`app/src/ai/geap_credentials.rs` runs the mint off the request path:

1. Trim and validate `gcpAudience`; select direct WIF or service-account federation from `gcpSaEmail`.
2. Request a fresh Warp OIDC JWT from `ManagedSecretManager::issue_task_identity_token` for every mint. The JWT is single-use and is not cached across mints.
3. `POST https://sts.googleapis.com/v1/token` with the token-exchange grant, configured audience, cloud-platform scope, and the Warp JWT as an ID token.
4. For service-account federation, `POST https://iamcredentials.googleapis.com/v1/projects/-/serviceAccounts/{sa_email}:generateAccessToken` with the federated token, cloud-platform scope, and a `3600s` lifetime.
5. Store the resulting token and expiry in `Loaded`, then arm the next one-shot refresh timer.

The direct-WIF path uses STS `expires_in` when present and otherwise falls back to the Warp JWT expiry. The service-account path parses the IAM response's RFC 3339 `expireTime`.

### Triggers and guards

The refresher subscribes to:

- `TeamsChanged` (including initial team/account loading).
- `UpdateWorkspaceSettingsSuccess`.
- Changes to `AISettings::gemini_enterprise_credentials_enabled`.
- The request-time safety net immediately before an Agent Mode request is built.
- A one-shot timer scheduled five minutes before the loaded token expires.
- The Settings **Refresh** action and the inline error view's **Refresh credentials** action, both of which force a mint.

All non-forced triggers use the same guard:

- Gate off → `Disabled`; any held token is discarded.
- Enabled but no usable audience → `Unconfigured`.
- A mint already in `Refreshing` → no-op, including a forced request.
- A fresh `Loaded` token with a matching binding and more than five minutes remaining → no-op.
- A binding mismatch, missing state, failed state, or token within the five-minute lead window → start one mint.

The request-time safety net never delays the triggering request. It re-arms a parked or never-armed chain in the background, and the request carries whichever matching token is currently available.

### Timer, expiry, and failure behavior

`GEAP_REFRESH_LEAD_TIME` is five minutes. The one-shot timer is armed once for each `Loaded` token at `expires_at - 5 minutes`; a 60-second minimum delay prevents a near-expired or clock-skewed token from causing a hot mint loop. There is no periodic polling.

Expiry is used to decide when to refresh, not whether to attach:

- `access_token_for_request()` sends a non-empty token even when it is near expiry or already expired. Google remains the authority on token validity; silently omitting an expired token would hide failures behind an unintended fallback.
- During a re-mint, a previous matching token remains attachable until replacement.
- A failed background re-mint restores a usable previous token and parks the proactive chain. The next request, settings change, team change, or forced refresh can try again; there is no unbounded retry loop.
- A first mint or forced refresh with no usable token becomes `Failed` and exposes the structured error through the client status/recovery UI.
- If the gate or binding changes while a mint is in flight, the result is discarded rather than stored for the wrong user/configuration. The client re-evaluates the current binding and starts a new mint when appropriate.

## Request wire contract

`RequestParams::new` computes the current `GeapMintBinding` and calls `ApiKeyManager::api_keys_for_request`. The method remains a pure in-memory read:

- With a matching `Loaded` token, it sets `ApiKeys.google_cloud_credentials.access_token`.
- With `Refreshing { previous: Some(...) }`, it sets the same field from the matching previous token.
- It omits the field for `Missing`, `Disabled`, `Unconfigured`, `Failed`, first-mint `Refreshing`, blank tokens, or any binding mismatch.
- GEAP attachment is controlled by the GEAP gate, independently of the generic BYO API-key gate.
- No expiry check or network call occurs while constructing the request.

The credential is request-scoped and lives only in memory on the client. The server's existing `ApiKeys` extraction/redaction boundary handles it after receipt; the client does not log token material.

## Settings and model-routing UX

### Settings > Warp Agent

The Gemini Enterprise widget renders only when the client feature flag and workspace host availability are both true. It contains:

- A **Use Gemini Enterprise credentials** toggle when `RESPECT_USER_SETTING` applies.
- An organization-managed, non-editable presentation when `ENFORCE` applies.
- A **Refresh** button that calls the forced mint. It is disabled when AI is unavailable, the effective gate is off, or the current state requires admin action.
- A credential status card driven by `GeapCredentialsState::user_facing_components()`:
  - cold-start/missing,
  - disabled,
  - setup incomplete (`Unconfigured`, with admin guidance),
  - refreshing,
  - loaded (including `Loaded at … · Refresh scheduled for …`, not a raw expiry alarm),
  - or a sanitized per-leg failure.

### Model pickers and details

The inline model selector and execution-profile model menus use the server-provided `LLMInfo.host_configs` data. A Gemini Enterprise Agent Platform badge/icon appears only when both the effective GEAP credential gate and that model's GEAP host config are enabled. If AWS Bedrock and GEAP are both eligible, Bedrock retains the existing priority.

The model details cost row reports **Inference via Gemini Enterprise Agent Platform** for a concrete model and **Inference may use Gemini Enterprise Agent Platform** for Auto. Its **Manage** action opens the Gemini Enterprise section in `Settings > Warp Agent`. The client does not hard-code a GEAP model registry; model availability and supported native/partner models come from the server's model choices.

## Credential-error recovery

When the server reports an invalid API-key result for provider `GEMINI_ENTERPRISE`, the client maps it to `RenderableAIError::GeminiEnterpriseCredentialsExpiredOrInvalid` instead of the generic user-provided Google API-key error. This error:

- Renders the stateful desktop inline view with **Refresh credentials** and **Manage** actions.
- Invokes the existing forced WIF mint when the user selects **Refresh credentials**.
- Shows refreshing and green **Credentials refreshed** states when `ApiKeyManager` reports the corresponding transitions, then instructs the user to retry the request.
- Falls back to a renderer-neutral message in CLI/TUI contexts.
- Classifies the task as `FAILED` with `AuthenticationRequired` for local-agent task status reporting.

The inline action is recovery UX for a server-reported credential failure; it does not add a separate mint protocol or an automatic conversation replay. Inference-time project/location/model errors and customer quota errors remain in the existing server/client inference-error pipeline.

## Security and compatibility invariants

- The only long-lived credential involved is the user's existing Warp session; GEAP access tokens and intermediate tokens are memory-only.
- Token values are redacted from `Debug`, logs, user-facing error copy, and request-independent state. Provider error detail is retained only for internal classification.
- Exact binding checks prevent a token minted for a previous account, audience, or service account from being attached after sign-out, account switch, or workspace configuration change.
- An additive `GEMINI_ENTERPRISE` enum value is handled explicitly by the client's GraphQL and app conversions. Unknown future enum values still use the existing fallback behavior.
- New workspace fields are optional and serde-defaulted for old local caches. Server-side feature/min-client gating controls which clients receive GEAP host/model data; the client spec does not promise an enum/query deployment-ordering requirement.
- The client consumes a host-level GEAP configuration. Per-model location overrides are not implemented client behavior.

## Validation captured by the shipped implementation

This feature was validated with unit and app-harness coverage in the merged client PRs:

- Config round-tripping and enablement tests cover feature/auth/workspace gates, default-off `RESPECT_USER_SETTING`, `ENFORCE`, host-disabled/absent, logged-out users, and `gcpAudience`/`gcpSaEmail`.
- Credential-engine tests cover WIF response parsing, direct-WIF and service-account paths, STS/JWT expiry fallback, RFC 3339 expiry parsing, the five-minute lead and 60-second timer floor, in-flight deduplication, binding mismatch, gate changes during mint, previous-token preservation, parked-chain request safety net, and structured error classification.
- Request tests cover matching attachment, omission for each non-loaded state, previous-token attachment during refresh, binding mismatches, and expired-token attachment.
- UI/routing tests cover the `Unconfigured` status and admin action, retry-vs-admin error classification, scheduled-refresh copy, GEAP badge eligibility, Bedrock priority, and model details text.
- Local end-to-end validation used a local warp-server plus a real GCP test project to verify WIF minting, customer-project inference, and the configured service-account path.

This reconciliation is **testing-exempt: pure documentation/copy**. It changes no runtime code or behavior, so no new regression test or computer-use/UI verification is appropriate. Repository markdown/spec checks remain the applicable validation gate.

## Follow-ups and non-goals

- GEAP minting for Oz/cloud-agent runners.
- Per-model GCP location overrides.
- A client-owned model registry or admin configuration panel.
- A client-side OAuth/ADC setup flow, refresh-token persistence, or service-account key handling.
- Automatic conversation replay after an inference credential error.
