# REV-1599 — Local agent GEAP credentials via Warp OIDC + Workload Identity Federation (client)

This spec covers the **Warp desktop client** half of GEAP (Gemini Enterprise Agent Platform) BYOLLM: minting the short-lived Google Cloud access token that local interactive agent requests carry. The server half (routing, redaction, billing) is specified in `warp-server/specs/REV-1599/TECH.md` and is merged on `develop`. This spec **supersedes the client-auth sections of that document**: the gcloud-ADC/`yup-oauth2` approach described there was prototyped and rejected in favor of Workload Identity Federation (WIF), so the client never reads local cloud credentials at all. That spec will be updated to point to this document for client credential logic. 

Scope: **local interactive agent requests only.** Cloud agents (Oz runners) are the next milestone and will reuse the same mint flow keyed off task identity. The lift from the current local interactive agent request to cloud agents *should* be lower than if we were to implement the gcloud-ADC/`yup-oauth` approach. The cloud agent implementation will be built off the same WIF machine to machine protocol. 

## Context

The approach: the client uses the user's **existing Warp auth token** to call warp-server's `IssueTaskIdentityToken` mutation for a **Warp-signed OIDC JWT**, exchanges that JWT at GCP STS for a federated token (RFC 8693), then impersonates a customer service account via the IAM Credentials API, and attaches the resulting ~1h access token to agent requests. No gcloud, no ADC files, no OAuth consent screen, no long-lived credential anywhere — the enterprise grants access by configuring a trust bridge (workload identity pool) in *their* GCP project, and revokes it the same way. The same bridge later serves cloud agents.

Existing machinery this builds on,

- [`crates/ai/src/aws_credentials.rs:38 @ a90be74`](https://github.com/warpdotdev/warp/blob/a90be740b2416c91728a0d7bd172169e4b5ab5a0/crates/ai/src/aws_credentials.rs#L38) — `AwsCredentialsState`, the Bedrock credential state machine this feature mirrors 1:1.
- [`app/src/ai/aws_credentials.rs:303 @ a90be74`](https://github.com/warpdotdev/warp/blob/a90be740b2416c91728a0d7bd172169e4b5ab5a0/app/src/ai/aws_credentials.rs#L303) — `refresh_aws_credentials_oidc`, the existing Warp-JWT → STS pattern (AWS flavor, cloud agents only). GEAP brings the same shape to local users.
- [`crates/ai/src/api_keys.rs:292 @ a90be74`](https://github.com/warpdotdev/warp/blob/a90be740b2416c91728a0d7bd172169e4b5ab5a0/crates/ai/src/api_keys.rs#L292) — `ApiKeyManager::api_keys_for_request`, the pure in-memory request-time read that credentials attach through.
- [`crates/managed_secrets/src/manager.rs:215 @ a90be74`](https://github.com/warpdotdev/warp/blob/a90be740b2416c91728a0d7bd172169e4b5ab5a0/crates/managed_secrets/src/manager.rs#L215) — `issue_task_identity_token`, the client API for minting Warp OIDC JWTs (registered unconditionally; works for local users).
- [`crates/managed_secrets/src/gcp.rs:198 @ a90be74`](https://github.com/warpdotdev/warp/blob/a90be740b2416c91728a0d7bd172169e4b5ab5a0/crates/managed_secrets/src/gcp.rs#L198) — the Oz cloud-task GCP WIF audience format this feature reuses verbatim.

Server-side facts the client design depends on (pinned to `warp-server @ 6a56987`):

- [`graphql/v2/resolvers/issue_task_identity_token.resolvers.go:18 @ 6a56987`](https://github.com/warpdotdev/warp-server/blob/6a5698727bf200d6dd1689a2c34c76813b60bdbc/graphql/v2/resolvers/issue_task_identity_token.resolvers.go#L18) — `IssueTaskIdentityToken`, the GraphQL mutation the client calls. Authenticated by the user's **regular Warp auth token** (the session already on the GraphQL request); it derives the caller via `GetRequiredPrincipalFromContext` (`:26`), then hands off to `IssueToken` (`:31`). No GCP credential is involved in this hop — the Warp session is the root credential that bootstraps the whole chain.
- [`logic/federated_token/issue_token.go:100 @ 6a56987`](https://github.com/warpdotdev/warp-server/blob/6a5698727bf200d6dd1689a2c34c76813b60bdbc/logic/federated_token/issue_token.go#L100) — `IssueToken` works for any authenticated principal (task claims optional); JWTs carry `sub`/`email`/`teams` ([claims @ 6a56987](https://github.com/warpdotdev/warp-server/blob/6a5698727bf200d6dd1689a2c34c76813b60bdbc/logic/federated_token/claims.go#L235)); lifetime 5m–3h; `warp.dev` audiences are rejected, so the JWT audience is the WIF provider resource path.
- [`logic/ai/llm/model_fallback_chain.go:181 @ 6a56987`](https://github.com/warpdotdev/warp-server/blob/6a5698727bf200d6dd1689a2c34c76813b60bdbc/logic/ai/llm/model_fallback_chain.go#L181) — GEAP routes are filtered out when the request carries no token; priority `AWS_BEDROCK → GEMINI_ENTERPRISE → DIRECT_API`; `ENFORCE` removes Direct API fallback.
- [`router/handlers/generate_multi_agent_output.go:1497 @ 6a56987`](https://github.com/warpdotdev/warp-server/blob/6a5698727bf200d6dd1689a2c34c76813b60bdbc/router/handlers/generate_multi_agent_output.go#L1497) — the token rides the existing `ApiKeys` redaction boundary.
- [`config/local.yaml:625 @ 6a56987`](https://github.com/warpdotdev/warp-server/blob/6a5698727bf200d6dd1689a2c34c76813b60bdbc/config/local.yaml#L625) — local dev servers sign OIDC tokens with **staging's KMS key and `iss: https://staging.warp.dev`**, so Google can validate locally-minted JWTs against staging's public JWKS. This is what makes full local E2E testing possible.

## Data model

Three kinds of data:

**1. Admin federation config — per-team, persisted server-side, synced to every client.** Lives on the `GEMINI_ENTERPRISE` entry of `LlmHostSettings` (org settings JSON column → workspace GraphQL → client workspace model). All values are public identifiers, never secrets:

- `gcpProjectId` + `gcpLocation` — consumed by the **server** to build the Vertex endpoint/quota target. The client never reads them.
- `gcpAudience` — consumed by the **client**. The full workload identity provider resource name (`//iam.googleapis.com/projects/{num}/locations/global/workloadIdentityPools/{pool}/providers/{provider}`). Stored as one opaque string rather than three fields because it is exactly the value both the JWT `aud` claim and the STS `audience` parameter require, and it matches GCP's own `external_account` config shape.
- `gcpSaEmail` — consumed by the **client**. The service account to impersonate; empty means "use the federated token directly" (mirrors `GcpFederationConfig`'s optional impersonation).
- `enabled` — the admin's team-level on-switch (is the GEAP host available for this workspace at all). `enablementSetting` is read on **both** sides: the **client** gate consults it to decide whether to mint/attach (`ENFORCE` → on for every member; `RESPECT_USER_SETTING` → defer to the member's local toggle), and the **server** uses it for fallback policy (whether Direct API stays available).

The federation *parameters* are **admin-only**: `gcpAudience`, `gcpSaEmail`, `gcpProjectId`, and `gcpLocation` are configured once by the admin and sync to every team member with zero per-machine setup. A member's *only* interaction is a single on/off toggle in Settings, and the client enablement gate mirrors Bedrock's `is_aws_bedrock_credentials_enabled` (`app/src/workspaces/user_workspaces.rs:545`): after the admin-availability check it reads `enablementSetting` — `ENFORCE` mints/attaches for everyone (toggle hidden, via the `is_*_toggleable` helper), while `RESPECT_USER_SETTING` defers to the member's local **`AISettings`** toggle (`gemini_enterprise_credentials_enabled`, the GEAP analog of `aws_bedrock_credentials_enabled`, `app/src/settings/ai.rs:1020`). `AISettings` is the client's per-user (cloud-synced) settings store, distinct from the admin org settings in `UserWorkspaces`: the admin owns the federation *config*, the member owns only this on/off switch. The integration branch currently implements the admin-availability half only — adding the `enablementSetting` branch and the new `AISettings` field is the remaining client wiring (see Follow-ups).

**2. Client in-memory credential state — never persisted.** `GeapCredentialsState` on the `ApiKeyManager` singleton, mirroring `AwsCredentialsState`: `Missing | Disabled | Refreshing | Loaded { credentials, loaded_at } | Failed { message }`. `GeapCredentials` holds `{ access_token, expires_at }` with private fields; the only egress is the conversion into the wire type. `is_expired()` treats anything within 60s of expiry as expired so an in-flight request never carries a token that dies mid-stream.

**3. The wire credential — request-scoped secret.** `Settings.ApiKeys.GoogleCloudCredentials { access_token }` on the multi-agent request proto. Intentionally the entire shape: no token type (bearer is the transport default), no expiry (the server cannot refresh; Google is the source of truth for staleness). Because it rides in `ApiKeys`, the server's existing extract-then-`ClearApiKeys` redaction covers it with no new logging surface.

## Data flow

Three flows. File references are to the integration branch (`jaiden/wif-oidc-gcp @ 545186b1`); pseudocode is abbreviated from the real implementation.

### 1. Config sync (admin → every client)

The federation config rides the existing workspace-settings sync — no new transport. The hop chain:

- `crates/warp_graphql_schema/api/schema.graphql` — the client's schema copy adds `gcpAudience`/`gcpSaEmail` to `LlmHostSettings`; cynic validates query fragments against this at compile time.
- `crates/graphql/src/api/workspace.rs` — the `LlmHostSettings` `cynic::QueryFragment` adds `gcp_aud: Option<String>` / `gcp_sa_email: Option<String>`, which puts the fields into the workspace query text (this is what creates the server-deploy ordering constraint in Risks).
- `app/src/workspaces/gql_convert.rs` → `app/src/workspaces/workspace.rs` — the `From<warp_graphql::workspace::LlmHostSettings>` impl copies them onto the app-side `LlmHostSettings`, which is serde-persisted in the local workspace cache (old caches deserialize the new fields to `None`).
- `app/src/workspaces/user_workspaces.rs` — the two derivations everything downstream consumes:

```rust
// app/src/workspaces/user_workspaces.rs
pub fn gemini_enterprise_host_settings(&self) -> Option<&LlmHostSettings> {
    self.current_workspace()?.settings.llm_settings
        .host_configs.get(&LLMModelHost::GeminiEnterprise)
}
// Target shape, mirroring is_aws_bedrock_credentials_enabled. The integration branch currently
// returns only the admin-availability half; the enablement_setting branch + toggle is pending.
pub fn is_gemini_enterprise_credentials_enabled(&self, app: &AppContext) -> bool {
    if !self.is_gemini_enterprise_available_from_workspace() { return false; } // admin on-switch
    match self.gemini_enterprise_host_enablement_setting() {
        Enforce => true,                                                        // forced on for all members
        RespectUserSetting => *AISettings::as_ref(app)                          // else the member's own toggle
            .gemini_enterprise_credentials_enabled.value(),
    }
}
```

### 2. Credential mint (client ↔ warp-server ↔ Google)

The mint is a fixed sequence, each leg gating the next:
- **Leg 0 — precondition:** the user is already signed into Warp. There is no GCP login step, so the existing Warp auth token is the *only* credential the client starts with.
- **Leg 1 — Warp OIDC JWT:** a trigger calls `issue_task_identity_token`, which issues the `IssueTaskIdentityToken` GraphQL mutation to warp-server **authenticated by the regular Warp auth token**. The resolver derives the principal from that session and `IssueToken` returns a short-lived Warp-signed JWT (`aud` = `gcpAudience`; `sub`/`email`/`teams` from the principal). No GCP credential yet.
- **Leg 2 — STS exchange:** the JWT is exchanged at Google STS for a federated token; Google validates the issuer signature (public JWKS) and the audience against the pool's allowed audiences.
- **Leg 3 — SA impersonation:** if `gcpSaEmail` is set, the federated token mints an SA access token via IAM `generateAccessToken`; skipped entirely when empty.
- **Leg 4 — store:** the resulting ~1h access token lands in `GeapCredentialsState::Loaded` in memory for flow #3 to attach.

Legs 1–3 run off the request path in the async refresh; only leg 4's later attach touches a live request.

Lives in `app/src/ai/geap_credentials.rs` (cfg'd out of wasm in `app/src/ai/mod.rs`); credential state is owned by the `ApiKeyManager` singleton (`crates/ai/src/api_keys.rs`, `set_geap_credentials_state` emits `KeysUpdated`). The subscription is registered at app init in `app/src/lib.rs`, beside the Bedrock equivalent:

```rust
// app/src/ai/geap_credentials.rs — triggers (GeapCredentialRefresher, subscribed in app/src/lib.rs)
ctx.subscribe_to_model(&UserWorkspaces::handle(ctx), |manager, event, ctx| match event {
    UpdateWorkspaceSettingsSuccess => force_refresh_geap_credentials(manager, ctx), // audience/SA may have changed
    TeamsChanged => refresh_geap_credentials(manager, ctx),                         // startup / team switch
    _ => {}
});

// refresh_geap_credentials_with_options(manager, force, ctx) — gate order matters:
if !UserWorkspaces::as_ref(ctx).is_gemini_enterprise_credentials_enabled(ctx) {
    set_state(Disabled); return;                       // admin off, enforced-off, or member opted out
}
let Some(config) = GeapWifConfig::from_host_settings(...) // { audience, service_account_email: Option }
    else { set_state(Missing); return };               // enabled but unconfigured
if !force && state is Loaded && expires_at > now + 5min { return; } // skip-if-valid, don't hammer STS
set_state(Refreshing);
let token_future = ManagedSecretManager::issue_task_identity_token(IdentityTokenOptions {
    audience: config.audience,                          // JWT aud = the WIF provider resource name
    requested_duration: 1h,
    subject_template: vec1!["principal"],               // sub = "user:<uid>"; email/teams claims always included
}); // -> IssueTaskIdentityToken GraphQL mutation (manager.rs), authed by the user's Warp session; resolver derives the principal
ctx.spawn(
    async move { exchange_identity_token_for_geap_credentials(token_future.await?, &config).await }, // background
    |manager, result, ctx| manager.set_geap_credentials_state(Loaded{..} or Failed{..}, ctx),        // main thread
);
```

The exchange (same file) is two typed HTTP calls via `http_client::Client` (the repo's Compat-wrapped reqwest, required to run off-Tokio on the warpui executor):

```rust
// app/src/ai/geap_credentials.rs — exchange_identity_token_for_geap_credentials
// Leg 1: STS token exchange (RFC 8693). serde rename_all = "camelCase" request; OAuth snake_case response.
POST https://sts.googleapis.com/v1/token
  StsTokenExchangeRequest { grant_type: token-exchange, audience: config.audience,
      subject_token: <Warp JWT>, subject_token_type: id_token,
      scope: cloud-platform, requested_token_type: access_token }
  -> StsTokenExchangeResponse { access_token, expires_in: Option<u64> }
     // Google validated issuer signature (public JWKS) + allowed audiences here.
     // expires_in == None → fall back to the JWT's own expiry as a conservative bound.

// Leg 2: SA impersonation — skipped entirely when config.service_account_email is None.
POST https://iamcredentials.googleapis.com/v1/projects/-/serviceAccounts/{sa_email}:generateAccessToken
  bearer = federated token
  GenerateAccessTokenRequest { scope: [cloud-platform], lifetime: "3600s" }
  -> GenerateAccessTokenResponse { access_token, expire_time: <RFC 3339> }
     // IAM authorizes only if the pool identity holds roles/iam.workloadIdentityUser on the SA —
     // the customer's control point for who may become the SA.
```

Failures map to `LoadGeapCredentialsError::{MintIdentityToken, ExchangeToken, ImpersonateServiceAccount}` → `Failed { message }`, so the state pinpoints the broken leg with per-leg actionable copy; error bodies are capped at 512 chars and never contain the token.

### 3. Request attachment (client → server → Vertex)

`crates/ai/src/geap_credentials.rs` holds the state machine and the **only token egress** (`From<GeapCredentials> for api::request::settings::api_keys::GoogleCloudCredentials`). Attachment happens in `crates/ai/src/api_keys.rs`, wired at the request build site:

```rust
// app/src/ai/agent/api.rs — RequestParams construction
let api_keys = api_key_manager.api_keys_for_request(
    is_byo_enabled,
    user_workspaces.is_aws_bedrock_credentials_enabled(app),
    user_workspaces.is_gemini_enterprise_credentials_enabled(app), // GEAP gate (enablementSetting + member toggle), re-checked per request
);

// crates/ai/src/api_keys.rs — pure in-memory read, no I/O on the request path
let google_cloud_credentials = include_geap_credentials
    .then(|| match self.geap_credentials_state {
        Loaded { ref credentials, .. } if !credentials.is_expired() => Some(credentials.clone().into()),
        _ => None, // Missing/Disabled/Refreshing/Failed/expired ⇒ proto field omitted entirely
    })
    .flatten();
```

Server-side (already on `develop`): extract + redact at the `ApiKeys` boundary, the fallback chain keeps the GEAP route only when a token is present, and dispatch combines **admin policy** (project, location, model ref) with the **request token** (auth) to build the customer Vertex client. The token decides eligibility; the policy decides destination.

```mermaid
sequenceDiagram
  participant C as Warp client
  participant S as warp-server
  participant G as GCP STS / IAM
  participant V as Vertex AI (customer project)
  Note over C: trigger (startup / settings change); client already holds the user's Warp auth token
  C->>S: IssueTaskIdentityToken(aud = gcpAudience, 1h) — authed with regular Warp auth token
  Note over S: GetRequiredPrincipalFromContext → IssueToken
  S-->>C: Warp OIDC JWT (sub, email, teams from the principal)
  C->>G: STS token exchange (RFC 8693)
  G-->>C: federated token
  C->>G: generateAccessToken(gcpSaEmail)
  G-->>C: SA access token (~1h)
  Note over C: GeapCredentialsState::Loaded (in memory)
  C->>S: agent request + GoogleCloudCredentials.access_token
  S->>V: BackendVertexAI(gcpProjectId, gcpLocation) + request token
  V-->>S: stream (audit log: pool subject = user email, principal = SA)
```

Security invariants: the access token lives only in memory, is never persisted, never logged (logs carry the audience — a public identifier — and outcomes only); no refresh token, ADC file, or SA key exists anywhere in the flow; expired tokens are never sent.

## Testing and validation

Maps to `warp-server/specs/REV-1599/PRODUCT.md` Goal 3 (seamless member credentials) and the Data Handling constraints.

- **Unit tests (exist on the integration branch):** `crates/ai/src/api_keys_tests.rs` — token attached when enabled, omitted when disabled, omitted when expired; `app/src/ai/geap_credentials_tests.rs` — `GeapWifConfig` parsing/trimming/audience handling, STS response with/without `expires_in`, impersonation response camelCase + RFC 3339 parsing, invalid-timestamp rejection; `app/src/workspaces/user_workspaces_tests.rs` — workspace gate on/off/absent-host.
- **E2E (performed, repeatable):** local warp-server (signs as staging per `local.yaml`) + local client + real GCP project `warp-geap-test-2026`. Verified: mint succeeds from synced host settings alone; GCP Data Access audit log shows the full chain — `serviceAccountDelegationInfo` = `principal://.../warp-geap-pool/subject/<user email>`, `principalEmail` = the SA, `granted: true` on `aiplatform.endpoints.predict`, resource in the customer project; utility-model calls (suggestion/classifier roles) also route to the customer project under `ENFORCE`.
- **Negative cases:** GEAP host disabled → `Disabled`, token absent from requests; empty `gcpAudience` → `Missing`; wrong pool/provider → `ExchangeToken` failure with config hints; missing `workloadIdentityUser` binding → `ImpersonateServiceAccount` 403; expired token → dropped from request (under `ENFORCE` the server surfaces reauth guidance; under `RESPECT_USER_SETTING` it falls back to Direct API).
- **Pre-merge:** `./script/format` + presubmit clippy per repo rules; client release additionally gated on the server schema deploy (see Risks).

## Parallelization

Not proposed. The implementation is complete on `jaiden/wif-oidc-gcp`; remaining work is landing it through review, which is sequential. Sub-agents would add coordination overhead with no wall-clock benefit. (The server-side spec parallelized its four workstreams; the client side is one tightly coupled path.)

## Risks and mitigations

- **Schema deployment ordering (the one hard external dependency).** The client's cynic fragment puts `gcpAudience`/`gcpSaEmail` into the workspace query text; a deployed server without those fields rejects the entire query. The client change must not reach users before the server's field additions (branch `jaiden/geap-wif-host-settings`, in flight) are deployed to prod. Mitigation: land server first, verify with a staging query, state the ordering in the client PR.
- **Mid-session token expiry.** Tokens last ~1h and there is no proactive re-mint timer yet; after expiry the token is dropped from requests until a trigger fires. Under `ENFORCE` users see the server's reauth error; under `RESPECT_USER_SETTING` requests silently fall back to Warp-managed inference (policy-correct, but weakens the BYO guarantee). Mitigation: expired-token guard prevents bad sends today; scheduled refresh is the named follow-up.
- **Forced re-mint on any workspace settings save.** `UpdateWorkspaceSettingsSuccess` does not distinguish GEAP fields, so unrelated admin saves cause an extra STS round-trip. Accepted (hourly-scale cost); could diff old/new host config later.
- **Customer-side misconfiguration.** The trust bridge is only as tight as the customer's IAM: setup docs must recommend scoping the `workloadIdentityUser` binding to team/user attributes (not the pool `/*` wildcard), a least-privilege (ideally custom) role on the SA, and a stable `google.subject` mapping (`assertion.sub`) with `attribute.user_email` for human-readable audit logs.

## Follow-ups

- Proactive re-mint before expiry (background timer), removing the mid-session expiry gap.
- Settings > Warp Agent status/refresh widget mirroring `AwsBedrockWidget` (`user_facing_components()` copy already exists).
- Cloud-agent (Oz runner) mint path: same exchange keyed off task identity, the GEAP analog of `AwsCredentialsRefreshStrategy::OidcManaged`.
- Admin Models page card for the GEAP fields (warp-server repo), replacing direct `updateWorkspaceSettings` edits.
- Enterprise setup docs/Terraform: pool + provider creation against the prod issuer (`https://app.warp.dev`), scoped SA binding, optional per-model BigQuery request-response logging for content audit.
- Finish the client enablement gate to Bedrock parity: add the per-user `AISettings.gemini_enterprise_credentials_enabled` setting and the `enablementSetting` branch to `is_gemini_enterprise_credentials_enabled` (which then takes `app`), so members can opt in/out under `RESPECT_USER_SETTING`. The integration branch currently implements the admin-availability half only; this also threads `app` through the call sites (the mint gate and `agent/api.rs`).
