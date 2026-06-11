# REV-1599 — Local agent GEAP credentials via Warp OIDC + Workload Identity Federation (client)

This spec covers the **Warp desktop client** half of GEAP (Gemini Enterprise Agent Platform) BYOLLM: minting the short-lived Google Cloud access token that local interactive agent requests carry. The server half (routing, redaction, billing) is specified in `warp-server/specs/REV-1599/TECH.md` and is merged on `develop`. This spec **supersedes the client-auth sections of that document**: the gcloud-ADC/`yup-oauth2` approach described there was prototyped and rejected in favor of Workload Identity Federation (WIF), so the client never reads local cloud credentials at all. That spec will be updated to point to this document for client credential logic. 

Scope: **local interactive agent requests only.** Cloud agents (Oz runners) are the next milestone and will reuse the same mint flow keyed off task identity. The lift from the current local interactive agent request to cloud agents should be lower than if we were to implement the gcloud-ADC/`yup-oauth` approach. The cloud agent implementation will be built off the same WIF machine to machine protocol. 

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

The federation *parameters* are **admin-only**: `gcpAudience`, `gcpSaEmail`, `gcpProjectId`, and `gcpLocation` are configured once by the admin and sync to every team member with zero per-machine setup. A member's *only* interaction is a single on/off toggle in Settings, and the client enablement gate mirrors Bedrock's `is_aws_bedrock_credentials_enabled` (`app/src/workspaces/user_workspaces.rs:545`): after the auth and admin-availability checks it reads `enablementSetting` — `ENFORCE` mints/attaches for everyone (toggle hidden, via the `is_*_toggleable` helper), while `RESPECT_USER_SETTING` defers to the member's local **`AISettings`** toggle (`gemini_enterprise_credentials_enabled`, the GEAP analog of `aws_bedrock_credentials_enabled`, `app/src/settings/ai.rs:1020`). The toggle **defaults to `false` (opt-in)**, matching Bedrock — under `RESPECT_USER_SETTING` a member must enable it to route requests through GEAP. `AISettings` is the client's per-user (cloud-synced) settings store, distinct from the admin org settings in `UserWorkspaces`. All parts of the gate are in scope: the logged-out auth guard, admin-availability check, `enablementSetting` branch, and the new `AISettings` toggle ship together.

**2. Client in-memory credential state — never persisted.** `GeapCredentialsState` on the `ApiKeyManager` singleton, mirroring `AwsCredentialsState`: `Missing | Disabled | Refreshing | Loaded { credentials, loaded_at, minted_for } | Failed { message }`. `GeapCredentials` holds `{ access_token, expires_at }` with private fields; the only egress is the conversion into the wire type. `is_expired()` treats anything within 60s of expiry as expired so an in-flight request never carries a token that dies mid-stream. `minted_for` is the **mint binding**: the Warp user uid plus the `(audience, sa_email)` config the token was minted against. The refresh guard and the attach-time read both treat a binding mismatch as not-loaded, so a token minted for a different account (sign-out/account switch) or against a stale federation config (admin changed audience/SA) is never attached, and is replaced on the next trigger instead of surviving until expiry.

**3. The wire credential — request-scoped secret.** `Settings.ApiKeys.GoogleCloudCredentials { access_token }` on the multi-agent request proto. Intentionally the entire shape: no token type (bearer is the transport default), no expiry (the server cannot refresh; Google is the source of truth for staleness). Because it rides in `ApiKeys`, the server's existing extract-then-`ClearApiKeys` redaction covers it with no new logging surface.

## Data flow

Three flows — pseudocode is abbreviated and illustrative.

### 1. Config sync (admin → every client)

The federation config rides the existing workspace-settings sync.

- `crates/warp_graphql_schema/api/schema.graphql` — the client's schema copy adds `gcpAudience`/`gcpSaEmail` to `LlmHostSettings`; cynic validates query fragments against this at compile time.
- `crates/graphql/src/api/workspace.rs` — the `LlmHostSettings` `cynic::QueryFragment` adds `gcp_audience: Option<String>` / `gcp_sa_email: Option<String>`.
- `app/src/workspaces/gql_convert.rs` → `app/src/workspaces/workspace.rs` — the `From<warp_graphql::workspace::LlmHostSettings>` impl copies them onto the app-side `LlmHostSettings`, which is serde-persisted in the local workspace cache (old caches deserialize the new fields to `None`).
- `app/src/workspaces/user_workspaces.rs` — the two derivations everything downstream consumes:

```rust
// app/src/workspaces/user_workspaces.rs
pub fn gemini_enterprise_host_settings(&self) -> Option<&LlmHostSettings> {
    self.current_workspace()?.settings.llm_settings
        .host_configs.get(&LLMModelHost::GeminiEnterprise)
}
// Mirrors is_aws_bedrock_credentials_enabled (user_workspaces.rs:545).
// Also adds the is_anonymous_or_logged_out guard from is_byo_api_key_enabled (user_workspaces.rs:482).
pub fn is_gemini_enterprise_credentials_enabled(&self, app: &AppContext) -> bool {
    if AuthStateProvider::as_ref(app).get().is_anonymous_or_logged_out() { return false; } // no session → no mint
    if !self.is_gemini_enterprise_available_from_workspace() { return false; }              // admin on-switch
    match self.gemini_enterprise_host_enablement_setting() {
        Enforce => true,                                                        // forced on for all members
        RespectUserSetting => *AISettings::as_ref(app)                          // else the member's own toggle (default: false)
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
// Three triggers, mirroring the full Bedrock subscription set.
ctx.subscribe_to_model(&UserWorkspaces::handle(ctx), |manager, event, ctx| match event {
    UpdateWorkspaceSettingsSuccess => refresh_geap_credentials(manager, ctx), // mint binding re-mints iff audience/SA changed
    TeamsChanged => refresh_geap_credentials(manager, ctx),                   // startup / team or account switch
    _ => {}
});
// Trigger 3: member flips their own toggle under RESPECT_USER_SETTING.
// Mirrors AISettingsChangedEvent::AwsBedrockCredentialsEnabled (aws_credentials.rs:228).
ctx.subscribe_to_model(&AISettings::handle(ctx), |manager, event, ctx| {
    if matches!(event, AISettingsChangedEvent::GeminiEnterpriseCredentialsEnabled { .. }) {
        drop(refresh_geap_credentials(manager, ctx));
    }
});

// refresh_geap_credentials_with_options(manager, force, ctx) 
if !UserWorkspaces::as_ref(ctx).is_gemini_enterprise_credentials_enabled(ctx) {
    set_state(Disabled); return;                       // admin off, enforced-off, or member opted out
}
let Some(config) = GeapWifConfig::from_host_settings(...) // { audience, service_account_email: Option }
    else { set_state(Missing); return };               // enabled but unconfigured

// Skip-if-valid, don't hammer STS. A minted_for mismatch (current user/config vs. the binding
// recorded at mint) falls through and re-mints under the fresh principal + config.
if !force && state is Loaded && minted_for matches (current user, config) && expires_at > now + 5min { return; }
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
// Leg 1: STS token exchange (RFC 8693).
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

Failures map to `LoadGeapCredentialsError::{MintIdentityToken, ExchangeToken, ImpersonateServiceAccount}` → `Failed { message }`, so the state pinpoints the broken leg with per-leg actionable copy; error bodies are capped at 512 chars and never contain the token. Each leg's outcome is also logged — leg name, audience, and sanitized error only, never token material — so a standard log bundle is enough for support to tell a Warp-session problem (leg 1) from a pool/provider misconfiguration (leg 2) from a missing IAM binding (leg 3); the Settings status widget (Follow-ups) will surface the same per-leg copy in-app.

### 3. Request attachment (client → server → Vertex)

`crates/ai/src/geap_credentials.rs` holds the state machine and the **only token egress** (`From<GeapCredentials> for api::request::settings::api_keys::GoogleCloudCredentials`). Attachment happens in `crates/ai/src/api_keys.rs`, wired at the request build site:

```rust
// app/src/ai/agent/api.rs — RequestParams construction.
// The call site computes the expected binding so api_keys_for_request stays a pure &self read
// without needing AppContext. Option::None when the GEAP gate is off; skips attach entirely.
let geap_gate = user_workspaces
    .is_gemini_enterprise_credentials_enabled(app) // auth + admin + enablementSetting + member toggle
    .then(|| GeapRequestGate {
        user_uid:  current_user_uid(app),
        audience:  host_settings.gcp_audience.clone(),
        sa_email:  host_settings.gcp_sa_email.clone(),
    });
let api_keys = api_key_manager.api_keys_for_request(
    is_byo_enabled,
    user_workspaces.is_aws_bedrock_credentials_enabled(app),
    geap_gate,  // carries the expected binding; None ⇒ GEAP skipped
);

// crates/ai/src/api_keys.rs — pure in-memory read, no I/O on the request path
let google_cloud_credentials = geap_gate
    .and_then(|gate| match self.geap_credentials_state {
        Loaded { ref credentials, ref minted_for, .. }
            if !credentials.is_expired() && minted_for.matches(&gate) => Some(credentials.clone().into()),
        _ => None, // Missing/Disabled/Refreshing/Failed/expired/binding-mismatch ⇒ proto field omitted entirely
    });
```

Server-side (already on `develop`): extract + redact at the `ApiKeys` boundary, the fallback chain keeps the GEAP route only when a token is present, and dispatch combines **admin policy** (project, location, model ref) with the **request token** (auth) to build the customer Vertex client. The token decides eligibility; the policy decides destination.

```mermaid
sequenceDiagram
  participant C as Warp client
  participant S as warp-server
  participant G as GCP STS / IAM
  participant V as Vertex AI (customer project)
  Note over C: trigger fires; client holds Warp auth token
  C->>S: IssueTaskIdentityToken(aud=gcpAudience, 1h) authed by Warp session
  Note over S: GetRequiredPrincipalFromContext - IssueToken
  S-->>C: Warp OIDC JWT (sub, email, teams)
  C->>G: STS token exchange (RFC 8693)
  G-->>C: federated token
  C->>G: generateAccessToken(gcpSaEmail)
  G-->>C: SA access token (~1h)
  Note over C: GeapCredentialsState::Loaded (in memory)
  C->>S: agent request + GoogleCloudCredentials.access_token
  S->>V: BackendVertexAI(gcpProjectId, gcpLocation) + token
  V-->>S: stream
```

Security invariants: the access token lives only in memory, is never persisted, never logged (logs carry the audience — a public identifier — and outcomes only); no refresh token, ADC file, or SA key exists anywhere in the flow; expired tokens are never sent.

### 4. Inline credential error view

When a GEAP turn fails due to credential state, the block renders an inline recovery view — `GeapCredentialsErrorView` in `app/src/ai/blocklist/inline_action/geap_credentials_error.rs` — modelled on `AwsBedrockCredentialsErrorView` but simpler (no configurable command, no auto-login checkbox).

Tokens last ~1h; any user who works in a single Warp session longer than that will hit token expiry. With the inline recovery, the experience is a short automatic re-mint (~1-3s) followed by a one-click retry.

**States the view handles:**

- **Token expired (most common, ~hourly):** The block detects `InvalidGeminiEnterpriseCredentialsError` from the server. The view immediately calls `force_refresh_geap_credentials` in the background and shows *"Gemini Enterprise credentials expired — refreshing..."*. The view subscribes to `ApiKeyManagerEvent::KeysUpdated`; when the state transitions to `Loaded`, it shows *"✓ Credentials refreshed"* and enables a **Retry** button. Retry calls `handle_resume_conversation` on the terminal view, replaying the failed turn with the fresh token.
- **Leg 2 / `ExchangeToken` failure (admin config):** *"Gemini Enterprise pool or provider configuration error — contact your workspace admin to verify the `gcpAudience` setting."* Retry button still present (the admin may have already pushed a fix), but copy directs admin action.
- **Leg 3 / `ImpersonateServiceAccount` failure (admin IAM):** *"Missing IAM binding on your workspace's service account — contact your workspace admin."* Same retry pattern.
- **Server 403 (IAM / API disabled):** *"Permission denied on your workspace's GCP project — contact your workspace admin to verify the Vertex AI API is enabled and the service account has the required role."*
- **Leg 1 / `MintIdentityToken` failure (Warp session / network):** *"Failed to authenticate with Warp — tap Retry or restart Warp."* Force-refresh fires automatically.

**Creation:** lazily created in `AIBlock` (same `Option<ViewHandle<...>>` pattern as `aws_bedrock_credentials_error_view`). `maybe_create_geap_credentials_error_view` fires when `RenderableAIError::GeapCredentialsExpiredOrInvalid` hits the block. The Retry button emits `AIBlockEvent::RetryGeapRequest { conversation_id }`, which terminal view handles by calling `handle_resume_conversation`.

The view has no auto-login checkbox (the re-mint is always automatic) and no Configure button (there is nothing member-configurable in GEAP).

Alternatives to this approach are

- Keeping a timer from the mint of the access token, and automatically minting a new access token when the expiry approaches (regardless of a user making a request or not). This seems like a waste of resources, especially if the user is not actively using the app.
- When the user makes a request, first check if the access token is expired (whether curr time > mint time + 1hr). If it is not expired, continue as normal. If it is expired, remint BEFORE sending the request. The user will barely notice the delay, and this prevents confusion around having to resend commands. 


## Enterprise setup

Two one-time setup surfaces (admin-owned) and one optional member toggle.

**Warp side (workspace settings).** Until the admin Models-page card ships (Follow-ups), admins set the GEAP host fields — `enabled`, `enablementSetting`, `gcpProjectId`, `gcpLocation`, `gcpAudience`, `gcpSaEmail` — directly through the `updateWorkspaceSettings` mutation. Misconfiguration degrades safely on the client: an enabled host with an empty `gcpAudience` rests at `Missing`, and a wrong pool/provider/SA value surfaces as the corresponding per-leg `Failed` state rather than affecting requests.

**Customer GCP side (the trust bridge).** What the enterprise configures once in *their* project. All steps use the `gcloud` CLI; variables are listed at the top of each block.

**Step 1 — Enable required APIs.**

```bash
gcloud services enable \
  iam.googleapis.com \
  iamcredentials.googleapis.com \
  sts.googleapis.com \
  aiplatform.googleapis.com \
  --project="$PROJECT_ID"
```

**Step 2 — Create the workload identity pool.** The pool is the top-level trust container. A single pool per Warp workspace is sufficient.

```bash
gcloud iam workload-identity-pools create "$POOL_ID" \
  --location="global" \
  --project="$PROJECT_ID"
```

**Step 3 — Create the OIDC provider inside the pool.** This tells GCP to trust JWTs signed by Warp's identity service. The allowed audience is the provider's own resource name — this exact string is what the admin pastes into `gcpAudience` in the Warp workspace settings.

- `--issuer-uri`: Warp's production OIDC issuer. Staging/local-dev tokens use `https://staging.warp.dev` instead (the E2E test pool is configured against staging).
- `--attribute-mapping`: `google.subject` maps to `assertion.sub` (stable `user:<uid>`) for the IAM binding; `attribute.user_email` maps to `assertion.email` for human-readable audit logs. Do not swap subject to email — emails can change and would invalidate bindings.

```bash
gcloud iam workload-identity-pools providers create-oidc "$PROVIDER_ID" \
  --location=global \
  --workload-identity-pool="$POOL_ID" \
  --issuer-uri="https://auth.warp.dev" \
  --allowed-audiences="//iam.googleapis.com/projects/$PROJECT_NUM/locations/global/workloadIdentityPools/$POOL_ID/providers/$PROVIDER_ID" \
  --attribute-mapping="google.subject=assertion.sub,attribute.user_email=assertion.email" \
  --project="$PROJECT_ID"
```

**Step 4 — Create a service account and grant permissions.** The service account is what ultimately calls Vertex AI; pool identities impersonate it.

```bash
# Create the service account.
gcloud iam service-accounts create "$SA_NAME" --project="$PROJECT_ID"

# Grant the SA permission to call Vertex AI.
gcloud projects add-iam-policy-binding "$PROJECT_ID" \
  --member="serviceAccount:$SA_NAME@$PROJECT_ID.iam.gserviceaccount.com" \
  --role="roles/aiplatform.user"

# Allow pool identities to impersonate the SA.
# The example below uses the pool wildcard (/*) for simplicity.
# For tighter security, scope to specific subjects:
#   --member="principal://iam.googleapis.com/.../subject/user:SPECIFIC_UID"
# or to an email attribute condition.
gcloud iam service-accounts add-iam-policy-binding \
  "$SA_NAME@$PROJECT_ID.iam.gserviceaccount.com" \
  --role="roles/iam.workloadIdentityUser" \
  --member="principalSet://iam.googleapis.com/projects/$PROJECT_NUM/locations/global/workloadIdentityPools/$POOL_ID/*" \
  --project="$PROJECT_ID"
```

**After running these steps,** the admin pastes the audience string (`//iam.googleapis.com/projects/$PROJECT_NUM/locations/global/workloadIdentityPools/$POOL_ID/providers/$PROVIDER_ID`) into `gcpAudience`, and `$SA_NAME@$PROJECT_ID.iam.gserviceaccount.com` into `gcpSaEmail` in the Warp workspace settings.

**Egress note.** Member machines need HTTPS egress to `sts.googleapis.com` and `iamcredentials.googleapis.com` in addition to existing Warp endpoints — relevant for enterprises with allowlisting or TLS-inspecting proxies.

Config-change propagation: the admin's own client re-mints on save; members pick up changed federation config via the existing workspace poll (~10 minutes) or on restart, at which point the mint binding (data model #2) forces the re-mint.

**Member toggle in Settings.** Under `RESPECT_USER_SETTING`, members must be able to opt in. This requires a visible toggle in Settings — without it the default-`false` `AISettings` field is inaccessible and `RESPECT_USER_SETTING` is functionally "GEAP disabled for everyone" in MVP.

Scope: add a `GeapCredentialsToggleRow` under **Settings > Warp Agent** (below the existing Bedrock section). The row is hidden when:
- GEAP is not enabled in the workspace (`is_gemini_enterprise_available_from_workspace()` is false)
- `enablementSetting` is `ENFORCE` (toggle replaced by an "Enabled by your workspace" note, via the same `is_*_toggleable` helper Bedrock uses at `user_workspaces.rs:538`)

When visible, the row shows a labeled toggle bound to `AISettings::gemini_enterprise_credentials_enabled`. This is the **only** member-facing UI in MVP for GEAP — no status indicator, no expiry display, no refresh button. Those belong to the Settings status widget (Follow-ups).

## Testing and validation

Maps to `warp-server/specs/REV-1599/PRODUCT.md` Goal 3 (seamless member credentials) and the Data Handling constraints.

- **Unit tests:** `crates/ai/src/api_keys_tests.rs` — token attached when gate+binding match, omitted when disabled, omitted when expired, omitted when binding mismatches (uid/audience/sa_email), omitted when logged out; `app/src/ai/geap_credentials_tests.rs` — `GeapWifConfig` parsing/trimming/audience handling, STS response with/without `expires_in`, impersonation response camelCase + RFC 3339 parsing, invalid-timestamp rejection; `app/src/workspaces/user_workspaces_tests.rs` — workspace gate on/off/absent-host, `ENFORCE` vs `RESPECT_USER_SETTING` crossed with the member toggle (default `false` → opt-in), logged-out user returns `false` regardless of workspace state.
- **E2E (performed, repeatable):** local warp-server (signs as staging per `local.yaml`) + local client + real GCP project `warp-geap-test-2026`. Verified: mint succeeds from synced host settings alone; GCP Data Access audit log shows the full chain — `serviceAccountDelegationInfo` = `principal://.../warp-geap-pool/subject/<user email>`, `principalEmail` = the SA, `granted: true` on `aiplatform.endpoints.predict`, resource in the customer project; utility-model calls (suggestion/classifier roles) also route to the customer project under `ENFORCE`.
- **Negative cases:** GEAP host disabled → `Disabled`, token absent from requests; empty `gcpAudience` → `Missing`; wrong pool/provider → `ExchangeToken` failure with config hints; missing `workloadIdentityUser` binding → `ImpersonateServiceAccount` 403; expired token → dropped from request (under `ENFORCE` the server surfaces reauth guidance; under `RESPECT_USER_SETTING` it falls back to Direct API).
- **Pre-merge:** `./script/format` + presubmit clippy per repo rules; client release additionally gated on the server schema deploy (see Rollout and gating).

## Parallelization

Not proposed.

## Rollout and gating

No client `FeatureFlag`, deliberately: the rollout switch is the server-side admin config itself. The GEAP host is disabled by default for every team, all client behavior keys off the synced workspace settings, and with no `gcpAudience` present the state machine rests at `Disabled`/`Missing` — requests are byte-identical to today's. A compile-time flag also could not gate the one genuinely risky surface: the cynic `QueryFragment` bakes `gcpAudience`/`gcpSaEmail` into the workspace query text unconditionally, so the real constraint is **deployment ordering**, not feature gating — the server's schema field additions must be deployed to production before a client containing this change ships, or the entire workspace query fails. Mitigation: land and deploy the server schema first, verify the workspace query against staging, and state the ordering requirement in the client PR. Rollback is config-level — the admin disables the GEAP host and clients rest at `Disabled` on the next sync — with no client release needed.

## Risks and mitigations

- **Mid-session token expiry.** Tokens last ~1h and there is no proactive re-mint timer yet; after expiry the token is dropped from requests until the inline error view detects the error, auto-fires `force_refresh_geap_credentials`, and presents a one-click Retry. Under `ENFORCE` this replays the failed turn with the fresh token (~1-3s). Under `RESPECT_USER_SETTING` requests silently fall back to Direct API when no token is present (policy-correct). Mitigation: the inline error view handles the expiry recovery path; a proactive background timer is the named follow-up to eliminate the failed-turn entirely.
- **Workspace saves and config drift.** `UpdateWorkspaceSettingsSuccess` does not distinguish GEAP fields, but the mint binding (data model #2) makes the refresh guard a no-op unless the audience/SA actually changed — unrelated admin saves cost no STS round-trip, while real federation-config changes re-mint immediately.
- **Customer-side misconfiguration.** The trust bridge is only as tight as the customer's IAM: setup docs must recommend scoping the `workloadIdentityUser` binding to team/user attributes (not the pool `/*` wildcard), a least-privilege (ideally custom) role on the SA, and a stable `google.subject` mapping (`assertion.sub`) with `attribute.user_email` for human-readable audit logs.

## Follow-ups

- Cloud-agent (Oz runner) mint path: same exchange keyed off task identity, the GEAP analog of `AwsCredentialsRefreshStrategy::OidcManaged`.
- Admin Models page card for the GEAP fields (warp-server repo), replacing direct `updateWorkspaceSettings` edits. This is not a big lift. Just some changes to connect the admin page to the server. 
