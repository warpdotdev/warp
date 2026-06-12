# REV-1599 — Local agent GEAP credentials via Warp OIDC + Workload Identity Federation (client)

This spec covers the **Warp desktop client** half of GEAP (Gemini Enterprise Agent Platform) BYOLLM: minting the short-lived Google Cloud access token that local interactive agent requests carry. The server half (routing, redaction, billing) is specified in `warp-server/specs/REV-1599/TECH.md` and is merged on `develop`. This spec **supersedes the client-auth sections of that document**: the gcloud-ADC/`yup-oauth2` approach described there was prototyped and rejected in favor of Workload Identity Federation (WIF), so the client never reads local cloud credentials at all. That spec will be updated to point to this document for client credential logic.

Scope: **local interactive agent requests only.** Cloud agents (Oz runners) are the next milestone and will reuse the same mint flow keyed off task identity. The lift from local interactive agents to cloud agents is lower than it would have been under the gcloud-ADC/`yup-oauth2` approach, since the cloud-agent implementation builds on the same WIF machine-to-machine protocol.

## Context

The approach: the client uses the user's **existing Warp auth token** to call warp-server's `IssueTaskIdentityToken` mutation for a **Warp-signed OIDC JWT**, exchanges that JWT at GCP STS for a federated token (RFC 8693), then impersonates a customer service account via the IAM Credentials API, and attaches the resulting ~1h access token to agent requests. No gcloud, no ADC files, no OAuth consent screen, no long-lived credential anywhere — the enterprise grants access by configuring a trust bridge (workload identity pool) in *their* GCP project, and revokes it the same way. The same bridge later serves cloud agents.

Existing machinery this builds on:

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

**2. Client in-memory credential state — never persisted.** `GeapCredentialsState` on the `ApiKeyManager` singleton, mirroring `AwsCredentialsState` in shape: `Missing | Disabled | Refreshing { previous } | Loaded { credentials, loaded_at, minted_for } | Failed { message }`. `GeapCredentials` holds `{ access_token, expires_at }` with private fields; the only egress is the conversion into the wire type, via `access_token_for_request()`. One deliberate divergence from Bedrock, taken from PR #12028's `grok_subscription`: `Refreshing` carries the previous credentials so requests keep authenticating during the ~1-3s re-mint — tokens stay until replaced. (Bedrock's `Refreshing` drops them, tolerable for its user-driven refreshes, not for a proactive re-mint firing mid-session every ~55 minutes.) `expires_at` is always known for GEAP and drives the three-layer expiry handling — proactive timer, request-time safety net, Google-401 backstop; tokens are **always sent, even past expiry**, never silently dropped — fully specified in "Token lifecycle: complete case enumeration" below. `minted_for` is the **mint binding**: the Warp user uid plus the `(audience, sa_email)` config the token was minted against. The attach-time read treats a binding mismatch as not-loaded, so a token minted for a different account (sign-out/account switch) or against a stale federation config (admin changed audience/SA) is never attached, and is replaced on the next trigger instead of surviving until expiry.

**3. The wire credential — request-scoped secret.** `Settings.ApiKeys.GoogleCloudCredentials { access_token }` on the multi-agent request proto. Intentionally the entire shape: no token type (bearer is the transport default), no expiry (the server cannot refresh; Google is the source of truth for staleness). Because it rides in `ApiKeys`, the server's existing extract-then-`ClearApiKeys` redaction covers it with no new logging surface.

## Data flow

Four flows — pseudocode is abbreviated and illustrative.

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
- **Leg 1 — Warp OIDC JWT:** a trigger calls `issue_task_identity_token`, which issues the `IssueTaskIdentityToken` GraphQL mutation to warp-server **authenticated by the regular Warp auth token**. The resolver derives the principal from that session and `IssueToken` returns a short-lived Warp-signed JWT (`aud` = `gcpAudience`; `sub`/`email`/`teams` from the principal). No GCP credential yet. **Every mint — initial or re-mint, timer/trigger/forced — starts here with a brand-new JWT:** the JWT is consumed exactly once by the immediately following STS exchange and then dropped, never cached or reused across mints, so an expired JWT can never be presented to Google (see "JWT expiry is a non-case" in the lifecycle section).
- **Leg 2 — STS exchange:** the JWT is exchanged at Google STS for a federated token; Google validates the issuer signature (public JWKS) and the audience against the pool's allowed audiences.
- **Leg 3 — SA impersonation:** if `gcpSaEmail` is set, the federated token mints an SA access token via IAM `generateAccessToken`; skipped entirely when empty.
- **Leg 4 — store:** the resulting ~1h access token lands in `GeapCredentialsState::Loaded` in memory for flow #3 to attach.

Legs 1–3 run off the request path in the async refresh; only leg 4's later attach touches a live request.

Lives in `app/src/ai/geap_credentials.rs` (cfg'd out of wasm in `app/src/ai/mod.rs`); credential state is owned by the `ApiKeyManager` singleton (`crates/ai/src/api_keys.rs`, `set_geap_credentials_state` emits `KeysUpdated`). The subscription is registered at app init in `app/src/lib.rs`, beside the Bedrock equivalent:

```rust
// app/src/ai/geap_credentials.rs — triggers (GeapCredentialRefresher, subscribed in app/src/lib.rs)
// Six triggers total: the four below, the request-time safety net (trigger 5,
// flow #3), and the manual Settings Refresh button (trigger 6).
ctx.subscribe_to_model(&UserWorkspaces::handle(ctx), |manager, event, ctx| match event {
    UpdateWorkspaceSettingsSuccess => refresh_geap_credentials(manager, ctx), // mint binding re-mints iff audience/SA changed
    TeamsChanged => refresh_geap_credentials(manager, ctx),                   // startup / team or account switch
    _ => {}
});
// Trigger 3: member flips their own toggle under RESPECT_USER_SETTING.
ctx.subscribe_to_model(&AISettings::handle(ctx), |manager, event, ctx| {
    if matches!(event, AISettingsChangedEvent::GeminiEnterpriseCredentialsEnabled { .. }) {
        drop(refresh_geap_credentials(manager, ctx));
    }
});
// Trigger 4: targeted one-shot timer, self-rescheduling after each successful re-mint.
// Mirrors the Grok subscription pattern (crates/ai/src/grok_subscription/mod.rs).
// When credentials land in Loaded, schedule_geap_token_refresh arms a one-shot
// Timer::after(delay) that fires GEAP_REFRESH_LEAD_TIME (5min) before expiry. On the
// callback, refresh_geap_credentials (non-force) runs; the skip-if-valid guard decides
// whether a re-mint is actually needed (see the case matrix below). The new Loaded
// state arms the next timer. No periodic polling — wakes up exactly once per token.
const GEAP_REFRESH_LEAD_TIME: Duration = Duration::from_secs(5 * 60);
// Floor on the timer delay so a near-expired store (badly skewed local clock) cannot
// spin mint → store → re-mint as a hot loop; the floor rate-limits timer-driven
// re-mints to once per minute.
const GEAP_MIN_TIMER_DELAY: Duration = Duration::from_secs(60);

fn schedule_geap_token_refresh(manager: &mut ApiKeyManager, ctx: &mut ModelContext<ApiKeyManager>) {
    // Returns early when expires_at is None (unreachable for GEAP — see case matrix).
    let Some(expires_at) = manager.geap_credentials().expires_at() else { return; };
    let now = SystemTime::now();
    // Near-expired store ⇒ fire_at is in the past ⇒ delay clamps to the 60s floor.
    let fire_at = expires_at.checked_sub(GEAP_REFRESH_LEAD_TIME).unwrap_or(now);
    let delay = fire_at
        .duration_since(now)
        .unwrap_or(Duration::ZERO)
        .max(GEAP_MIN_TIMER_DELAY);
    ctx.spawn(
        async move { Timer::after(delay).await; },
        // Non-force: the skip-if-valid guard handles the case where another trigger
        // (e.g. UpdateWorkspaceSettingsSuccess) already minted a fresh token while
        // the timer slept. Where Grok uses a `still_current` refresh-token identity
        // check, GEAP's guard (!needs_refresh() + binding) is the equivalent dedupe.
        |manager, _, ctx| { drop(refresh_geap_credentials(manager, ctx)); },
    );
}
// Called from set_geap_credentials_state when transitioning to Loaded:
// schedule_geap_token_refresh(manager, ctx);

// refresh_geap_credentials_with_options(manager, force, ctx)
if !UserWorkspaces::as_ref(ctx).is_gemini_enterprise_credentials_enabled(ctx) {
    set_state(Disabled); return;                       // admin off, enforced-off, or member opted out
}
let Some(config) = GeapWifConfig::from_host_settings(...) // { audience, service_account_email: Option }
    else { set_state(Missing); return };               // enabled but unconfigured

// In-flight dedupe: one mint at a time, force included — the in-flight result
// lands in ~1-3s and KeysUpdated re-renders whoever asked.
if state is Refreshing { return; }
// Skip-if-valid, don't hammer STS. A minted_for mismatch (current user/config vs. the binding
// recorded at mint) falls through and re-mints under the fresh principal + config.
if !force && state is Loaded && minted_for matches (current user, config) && !credentials.needs_refresh() { return; }
// The previous token (if any) keeps serving requests while the re-mint is in
// flight — tokens stay until replaced, so a request landing in the
// ~1-3s mint window still authenticates.
set_state(Refreshing { previous: current Loaded credentials + binding, if any });
// Every mint starts at leg 1 with a fresh JWT — never cached or reused across
// mints, so an expired JWT can never enter the STS exchange.
let token_future = ManagedSecretManager::issue_task_identity_token(IdentityTokenOptions {
    audience: config.audience,                          // JWT aud = the WIF provider resource name
    requested_duration: 1h,
    subject_template: vec1!["principal"],               // sub = "user:<uid>"; email/teams claims always included
}); // -> IssueTaskIdentityToken GraphQL mutation (manager.rs), authed by the user's Warp session; resolver derives the principal
ctx.spawn(
    async move { exchange_identity_token_for_geap_credentials(token_future.await?, &config).await }, // background
    // Completion re-checks the gate (it may have flipped during the mint) and then:
    //   Ok(creds)              → Loaded { creds, minted_for } → arm next timer
    //   Err, previous exists   → restore Loaded { previous } (chain parks until the
    //                            next request's safety net re-arms it; see failure policy)
    //   Err, no previous       → Failed { per-leg message }
    |manager, result, ctx| manager.apply_geap_mint_result(result, ctx),                              // main thread
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

Failures map to `LoadGeapCredentialsError::{MintIdentityToken, ExchangeToken, ImpersonateServiceAccount}`, so the error pinpoints the broken leg with per-leg actionable copy; whether a failure lands as `Failed { message }` or quietly restores the previous token is decided by the failure policy in "Token lifecycle" below. Error bodies are capped at 512 chars and never contain the token. Each leg's outcome is also logged — leg name, audience, and sanitized error only, never token material — so a standard log bundle is enough for support to tell a Warp-session problem (leg 1) from a pool/provider misconfiguration (leg 2) from a missing IAM binding (leg 3); a future Settings status widget can surface the same per-leg copy in-app.

### Token lifecycle: complete case enumeration

Closely mirrors `crates/ai/src/grok_subscription/mod.rs` as amended by **PR #12516**, which corrected the original PR #12028 design after a field incident: a connected subscription silently stopped authenticating for 10.5 hours because (a) the proactive refresh loop never armed off a stale startup policy, (b) the attach-time expiry skew then silently dropped the token from every request, and (c) a failed refresh permanently killed the loop. GEAP adopts the corrected design from the start. Three layers keep requests authenticated, in order of preference:

1. **Proactive timer (primary).** A one-shot timer re-mints when 5 minutes remain (`GEAP_REFRESH_LEAD_TIME`). A healthy session never sends a stale token and never sees a Google 401 for plain expiry.
2. **Request-time safety net (secondary).** Every agent request's build path calls `refresh_geap_credentials_if_needed`: if the gate is on and the chain is parked, never armed, or the token is within the lead window (or already expired), it kicks the standard background re-mint. The triggering request is never delayed — it carries the currently stored token.
3. **Google 401 backstop (last resort).** Tokens are **always sent, even past expiry — never silently dropped.** Anything stale or locally invisible (revocation, IAM binding removed, pool/provider deleted) comes back from Vertex as `401/403`, maps to `InvalidGeminiEnterpriseCredentialsError`, and the inline error view (flow #4) force-remints and offers Retry.

`GEAP_REFRESH_LEAD_TIME` is purely about keeping the token *fresh*, never about when it stops being *sent*.

```rust
// crates/ai/src/geap_credentials.rs
const GEAP_REFRESH_LEAD_TIME: Duration = Duration::from_secs(5 * 60);

impl GeapCredentials {
    /// The access token whenever it is non-empty — regardless of expiry.
    /// Possibly-expired tokens are still sent so Google stays the final
    /// authority on validity (mirrors GrokTokens::access_token_for_request);
    /// the background refresh layers replace stale tokens.
    pub fn access_token_for_request(&self) -> Option<&str> {
        (!self.access_token.is_empty()).then_some(self.access_token.as_str())
    }

    /// Whether the token is within GEAP_REFRESH_LEAD_TIME of expiry — or
    /// already past it — and should be re-minted. Used by the skip-if-valid
    /// guard, the timer, and the request-time safety net. None never reports
    /// needing a refresh — no expiry signal to act on (mirrors
    /// GrokTokens::needs_refresh).
    pub fn needs_refresh(&self) -> bool {
        match self.expires_at {
            Some(exp) => exp <= SystemTime::now() + GEAP_REFRESH_LEAD_TIME,
            None => false,
        }
    }
}
```

**`expires_at` is always known for GEAP**:
- SA path (leg 3): `GenerateAccessTokenResponse.expire_time` is RFC 3339, always present.
- No-SA path (leg 2 direct): `StsTokenExchangeResponse.expires_in` is relative seconds, converted to an absolute time via checked math (as in PR #12028's `grok_tokens_from_response`); when omitted, fall back to the Warp JWT's `expires_at` from `TaskIdentityToken` (always set by the server) as a conservative bound.

So `needs_refresh()`'s `None` arm is an unreachable safe default whose behavior (refresh: never; timer: none) is Grok-parity by construction; `access_token_for_request()` never consults expiry at all.

**Nothing persists and there is no refresh token.** A GEAP mint is rooted in the live Warp session (leg 1), so there is no second long-lived credential to store, rotate, or carry over. Cold start rests at `Missing` until `TeamsChanged` fires the first mint — and if that event never arrives, the first agent request's safety net mints instead.

**JWT expiry is a non-case by construction.** JWTs expire too — but the Warp OIDC JWT (and the intermediate STS federated token) are single-use credentials scoped to one mint, not managed credentials with their own lifecycle. Every re-mint of the access token — timer, trigger, or forced — re-runs the chain from leg 1 and **grabs a brand-new JWT first**; no JWT is ever held across mints. There is therefore no JWT cache, no JWT expiry tracking, no JWT refresh timer, and no way to present an expired JWT to STS. The JWT only has to outlive the leg 1 → leg 2 gap (seconds); even the server's 5-minute minimum lifetime exceeds that by orders of magnitude. Its expiry is consulted exactly once, as the conservative no-SA fallback bound above. In the pathological case where a mint stalls long enough for the JWT to lapse before the STS call, Google rejects it → `ExchangeToken` failure → the standard failure policy applies (previous token kept / `Failed`); no special handling needed.

---

**How to read the matrices below.** Each one is a moment in the token's life where code makes a decision, listed in execution order:

1. **Attach** (`api_keys_for_request`) — a request is being built: does the stored token go on the wire? Pure read; runs on every request.
2. **Refresh guard** (`refresh_geap_credentials_with_options`) — something asked for a re-mint: should a mint actually start? All six triggers funnel through this one gate.
3. **Timer schedule** (`schedule_geap_token_refresh`) — a new token just landed: when should the alarm ring? The timer is a **one-shot alarm armed once per token** — set for 5 minutes before that token's expiry — not a recurring poll.
4. **Timer callback** — the alarm rings ~55 minutes later: given what changed while it slept, is a re-mint still needed? It just calls the refresh function; the guard (2) decides.
5. **Mint completion** — the background 3-leg mint finishes ~1-3s after starting: what gets stored, given the result and whether the gate/config changed mid-mint? Success stores the new token and arms the next alarm (back to 3), which is what makes the loop self-sustaining.

A healthy hour in table order: mint completes (5) → store `Loaded`, arm alarm (3) → every request attaches the token (1) while the safety net no-ops through the guard (2) → alarm rings at T−5min (4) → guard approves the re-mint (2) → mint completes (5) → new token, new alarm (3) — repeat.

---

**At attach time (`api_keys_for_request`) — binding check only; expiry is never consulted:**

| State | Binding | Result | Why |
|---|---|---|---|
| `Missing` | — | `None` | GEAP not configured |
| `Disabled` | — | `None` | Gate off (policy/toggle/logged out) |
| `Refreshing { previous: None }` | — | `None` | First mint in flight; nothing to serve yet |
| `Refreshing { previous: Some }` | matches | `Some(previous)` | Old token keeps serving while the re-mint lands (PR #12028: tokens stay until replaced) |
| `Refreshing { previous: Some }` | mismatch | `None` | Stale identity/config |
| `Failed` | — | `None` | First mint failed; nothing to send |
| `Loaded` | mismatch | `None` | Sign-out/account switch or admin config change; replaced by the next trigger |
| `Loaded` | matches | `Some(token)` — **even if expired** | Google is the authority on validity; a stale token produces a *visible, recoverable* 401, never a silent downgrade |

**What an omitted token means for that request** (server side, already on `develop`): the fallback chain drops the GEAP route when no token rides the request — under `RESPECT_USER_SETTING` that is a silent, policy-allowed Direct API fallback; under `ENFORCE`, a fast reauth error. That silence is exactly why omission is reserved for *configuration* states (gate off, binding mismatch, no token yet) and never used for expiry — that was the 10.5-hour Grok incident: the proactive loop never armed, the expiry skew then dropped the token from every request with no logs and no UI signal. The GEAP equivalent is worse: an enterprise that configured GEAP precisely so inference routes through *their* GCP project would silently route through Warp-managed inference instead. A stale-but-sent token instead yields `401 → InvalidGeminiEnterpriseCredentialsError → inline error view` — observable and self-healing in both modes.

Attach stays a pure `&self` read: no mutation, no timer scheduling, no I/O on the request path. The mutation lives one step earlier, at the request-build call site, where the safety net (trigger 5) runs with full context.

---

**At refresh-guard time (inside `refresh_geap_credentials_with_options`):**

| Condition | Result |
|---|---|
| Gate off (admin off, enforced off, member opted out, logged out) | → `Disabled` (any held token dropped), stop |
| Enabled but `gcpAudience` empty | → `Missing`, stop |
| Already `Refreshing` | **No-op** — one mint in flight at a time (force included); the result lands in ~1-3s and `KeysUpdated` re-renders whoever asked |
| `force = true` (Settings Refresh button, inline error Retry/auto-remint) | Re-mints unconditionally |
| State ≠ `Loaded` (`Missing`/`Failed`) | Falls through → re-mints |
| `Loaded`, binding mismatch | Falls through → re-mints under current identity/config |
| `Loaded`, binding matches, `needs_refresh() = true` (≤ 5min remaining) | Falls through → re-mints |
| `Loaded`, binding matches, `needs_refresh() = false` (> 5min remaining) | **Skip** — token is fresh, no network calls (don't hammer STS) |

The request-time safety net (`refresh_geap_credentials_if_needed`, trigger 5) funnels into this same function with `force = false`, so every row above applies to it unchanged; the only thing it adds is *when* the function runs — on every agent request build.

---

**At timer schedule time (`schedule_geap_token_refresh`, called on every transition into `Loaded`):**

| `expires_at` | `delay` computed as |
|---|---|
| `None` | Returns — no timer (safe default; unreachable for GEAP, Grok-parity) |
| > `now + 5min` | `Timer::after(expires_at - 5min - now)` — one shot, ~55min for a 1h token |
| ≤ `now + 5min` (near-expiry on store) | `checked_sub`/`duration_since` underflow to `Duration::ZERO`, then clamp up to the **60s floor** (`GEAP_MIN_TIMER_DELAY`) — never immediate, so a badly skewed local clock cannot spin mint → store → re-mint as a hot loop (the one place this spec deliberately tightens PR #12028, which fires immediately) |

No periodic polling and no heartbeat: the process wakes exactly once per token lifetime.

---

**At timer callback time (fires ~5min before expiry):**

Calls `refresh_geap_credentials` (non-force). The skip-if-valid guard re-evaluates the world that existed when the timer was armed — this is GEAP's equivalent of Grok's `still_current` refresh-token identity check.

| What happened while the timer slept | Guard result | Outcome |
|---|---|---|
| Nothing — same token still loaded, now within 5min of expiry | `needs_refresh() = true` → falls through | Re-mints → new `Loaded` → chain continues |
| Another trigger already minted a fresh token (e.g. admin saved settings) | `needs_refresh() = false` → guard skips | No duplicate re-mint; the stale timer silently no-ops, so stacked timers from rapid mints are harmless |
| A mint is currently in flight | `Refreshing` → no-op | The in-flight result reschedules on success |
| Account switched; `TeamsChanged` minted for new user | Binding mismatch → falls through | Re-mints under the new identity |
| GEAP disabled while sleeping (admin, toggle, sign-out) | Gate check → `Disabled`; no timer re-scheduled | Chain parks; restarts on re-enable |
| Machine slept past `expires_at` | Timer fires late on wake → `needs_refresh() = true` | Re-mints on wake; the first request after wake still carries the stale token (Google may 401 → inline view) and itself kicks the safety net — recovery never depends on the late timer |

**At mint-completion time (the spawn callback, any trigger):** the callback re-checks the world before storing, because the gate or config may have changed during the ~1-3s mint:

| Mint result | Stored state |
|---|---|
| Gate flipped off mid-mint | `Disabled` — result discarded; no token is retained while disabled |
| `Ok(credentials)` | `Loaded { credentials, minted_for: (user, config at mint start) }` → `schedule_geap_token_refresh`. If the admin changed audience/SA mid-mint, the binding mismatch surfaces at the next attach/trigger and self-heals via re-mint |
| `Err`, a previous token exists | Restore `Loaded { previous }` — keep serving it, even near/past expiry (the server remains the authority). **No reschedule — the chain parks until the next request's safety net re-arms it** (see failure policy) |
| `Err`, no previous (first mint) | `Failed { per-leg message }` — surfaced by Settings and the inline error view |

**Failure policy — a failed refresh never permanently kills the loop:** a failed proactive re-mint logs the failing leg, keeps any previous token, and parks the chain — but only until the next agent request, whose safety net re-attempts the mint in the background. What follows:
- The kept token keeps riding requests — even past expiry (always-send). If Google rejects it, the 401 maps to `InvalidGeminiEnterpriseCredentialsError` → inline error view auto-remints + one-click Retry, under **both** enablement modes. There is no silent path: the skew-drop alternative would silently fall back to Direct API under `RESPECT_USER_SETTING` — routing an enterprise's inference through Warp-managed infrastructure with zero signal — which is exactly the 10.5-hour silent-downgrade incident described in the section intro (Grok's version silently fell back to Warp's org-wide xAI key).
- `Failed { message }` is reserved for mints with no previous token to keep (the first mint) and for forced refreshes, where the user explicitly asked and needs visible feedback.
- There is still no background retry *loop* — retries are demand-driven (next request, next trigger), so a hard-down network cannot cause unbounded STS traffic.

**Clocks, sleep, and drift — correctness never depends on the timer firing on time.** Every layer re-derives from the wall clock at its own moment of use: the guard's `needs_refresh()` (true for near-expired *and* already-expired tokens), the request-time safety net, and Google's own validation are each evaluated independently. The timer is only an optimization hint. Local clock ahead → tokens refresh early (a wasted mint; the 60s timer floor bounds the worst case). Local clock behind → local checks under-refresh but the token is sent anyway → Google 401 backstop and inline recovery. Google is always the final authority; the local checks exist to make the common path fast and silent.

---

**Chain lifecycle:**

```
Initial mint (TeamsChanged / settings save / toggle on / force / first request via safety net)
  → Refreshing { previous: None } → 3-leg WIF chain
  → Loaded { credentials, expires_at, minted_for }
  → schedule_geap_token_refresh → one-shot Timer::after(expires_at - 5min)
       ⇓ fires ~5min before expiry
  → refresh_geap_credentials (non-force); guard: needs_refresh() = true
  → Refreshing { previous: old token — still attached to requests }
  → 3-leg WIF chain
  → Ok  → Loaded { new token } → reschedule        ← self-sustaining ~hourly loop
  → Err → Loaded { previous } kept; chain parks    ← until the next agent request's
                                                      safety net re-arms it
```

**A parked chain restarts via:** the request-time safety net on any agent request (primary), inline error view auto-remint/Retry, Settings Refresh button, member toggle flip, `TeamsChanged`, `UpdateWorkspaceSettingsSuccess`, or app restart.

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
// Trigger 5 — request-time safety net (the GEAP analog of Grok's
// refresh_grok_tokens_if_needed): re-arms a parked or never-armed refresh chain.
// No-ops unless the gate is on AND (state is Missing/Failed, the binding
// mismatches, or needs_refresh() — which includes already-expired). The mint
// runs in the background; this request is never delayed and carries the
// currently stored token.
ApiKeyManager::handle(app).update(app, |manager, ctx| {
    manager.refresh_geap_credentials_if_needed(geap_gate.as_ref(), ctx);
});
let api_keys = api_key_manager.api_keys_for_request(
    is_byo_enabled,
    user_workspaces.is_aws_bedrock_credentials_enabled(app),
    geap_gate,  // carries the expected binding; None ⇒ GEAP skipped
);

// crates/ai/src/api_keys.rs — pure in-memory read, no I/O on the request path.
// No expiry check at attach (access_token_for_request): a possibly-expired
// token is still sent — Google stays the authority on validity, and silently
// dropping it would silently downgrade the request (Direct API fallback)
// instead of surfacing a recoverable 401 → inline error view (flow #4).
let google_cloud_credentials = geap_gate
    .and_then(|gate| match self.geap_credentials_state {
        Loaded { ref credentials, ref minted_for, .. }
            if minted_for.matches(&gate) => credentials.access_token_for_request().map(into_wire),
        // A re-mint in flight keeps serving the previous token.
        Refreshing { previous: Some((ref credentials, ref minted_for)) }
            if minted_for.matches(&gate) => credentials.access_token_for_request().map(into_wire),
        _ => None, // Missing/Disabled/Failed/first-mint/binding-mismatch ⇒ field omitted
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

Security invariants: the access token lives only in memory, is never persisted, never logged (logs carry the audience — a public identifier — and outcomes only); no refresh token, ADC file, or SA key exists anywhere in the flow. Attach performs no local expiry gating: a possibly-expired token is still sent (and replaced in the background by the safety net) — Google is the sole authority on whether a token is valid.

### 4. Inline credential error view

When a GEAP turn fails due to credential state, the block renders an inline recovery view — `GeapCredentialsErrorView` in `app/src/ai/blocklist/inline_action/geap_credentials_error.rs` — modelled on `AwsBedrockCredentialsErrorView` but simpler (no configurable command, no auto-login checkbox).

Tokens last ~1h, but plain expiry is normally invisible — the lifecycle layers above keep the token fresh and keep serving the previous one during re-mints. This view is the visible surface for what they cannot prevent: a stale token rejected by Google (sent by design, never silently dropped), server-side revocation, IAM/pool changes mid-session, or a request racing the very first mint. When it does appear, the experience is a short automatic re-mint (~1-3s) followed by a one-click retry.

**States the view handles:**

- **Token expired or revoked (the `401` path):** The block detects `InvalidGeminiEnterpriseCredentialsError` from the server. The view immediately calls `force_refresh_geap_credentials` in the background and shows *"Gemini Enterprise credentials expired — refreshing..."*. The view subscribes to `ApiKeyManagerEvent::KeysUpdated`; when the state transitions to `Loaded`, it shows *"✓ Credentials refreshed"* and enables a **Retry** button. Retry calls `handle_resume_conversation` on the terminal view, replaying the failed turn with the fresh token.
- **Leg 2 / `ExchangeToken` failure (admin config):** *"Gemini Enterprise pool or provider configuration error — contact your workspace admin to verify the `gcpAudience` setting."* Retry button still present (the admin may have already pushed a fix), but copy directs admin action.
- **Leg 3 / `ImpersonateServiceAccount` failure (admin IAM):** *"Missing IAM binding on your workspace's service account — contact your workspace admin."* Same retry pattern.
- **Server 403 (IAM / API disabled):** *"Permission denied on your workspace's GCP project — contact your workspace admin to verify the Vertex AI API is enabled and the service account has the required role."*
- **Leg 1 / `MintIdentityToken` failure (Warp session / network):** *"Failed to authenticate with Warp — tap Retry or restart Warp."* Force-refresh fires automatically.

The view has no auto-login checkbox (the re-mint is always automatic) and no Configure button (there is nothing member-configurable in GEAP).

**Wiring**

**1. `RenderableAIError::GeapCredentialsExpiredOrInvalid { model_name: String }` — `app/src/ai/agent/mod.rs`.**
Add this variant to the `RenderableAIError` enum alongside `AwsBedrockCredentialsExpiredOrInvalid`. The server-side `InvalidGeminiEnterpriseCredentialsError` (from the fallback chain and from Google's `401` via the error taxonomy) maps to this variant in the response parsing pipeline — same path as the Bedrock analog. The `model_name` field is surfaced in the error copy.

**2. `Option<ViewHandle<GeapCredentialsErrorView>>` field on `AIBlock` — `app/src/ai/blocklist/block.rs`.**
Add the field (mirroring `aws_bedrock_credentials_error_view` at `block.rs:1054`) and a `maybe_create_geap_credentials_error_view` function with the same lazy-creation pattern as `maybe_create_aws_bedrock_credentials_error_view` (`block.rs:3978`). Call it from the same error-handling site where `maybe_create_aws_bedrock_credentials_error_view` is called — wherever the block processes a `RenderableAIError` on output completion. The function:
- Early-returns if `error` is not `GeapCredentialsExpiredOrInvalid`
- Early-returns if the view already exists
- Immediately calls `force_refresh_geap_credentials` (no auto-login concept)
- Creates and stores `GeapCredentialsErrorView`
- Subscribes to view events: `RetryRequest` → emits `AIBlockEvent::RetryGeapRequest { conversation_id }`

**3. `AIBlockEvent::RetryGeapRequest { conversation_id }` — `app/src/ai/blocklist/block.rs` + `app/src/terminal/view.rs`.**
Add the variant to the `AIBlockEvent` enum. In `terminal/view.rs`, handle it the same way as `ContinueConversation` / `ResumeConversation` — call `handle_resume_conversation(conversation_id, ctx)` to replay the failed turn with the freshly minted token.

## Enterprise setup

Two one-time setup surfaces (admin-owned) and one optional member toggle.

**Warp side (admin Models page).** Admins enter the GEAP host fields — `enabled`, `enablementSetting`, `gcpProjectId`, `gcpLocation`, `gcpAudience`, `gcpSaEmail` — on the admin Models page (the Gemini Enterprise card, mirroring Bedrock's). Misconfiguration degrades safely on the client: an enabled host with an empty `gcpAudience` rests at `Missing`, and a wrong pool/provider/SA value surfaces as the corresponding per-leg `Failed` state rather than affecting requests.

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

**Step 3 — Create the OIDC provider inside the pool.** This tells GCP to trust JWTs signed by Warp's identity service. The allowed audience is the provider's own resource name — this exact string is what the admin pastes into `gcpAudience` on the admin Models page.

**Tenant isolation is mandatory here, not optional hardening.** Warp is a *shared* OIDC issuer: `IssueTaskIdentityToken` mints a validly-signed JWT for **any authenticated Warp user**, and the audience is a public identifier (synced to every member's client; its format is guessable). Issuer + audience alone would therefore admit every Warp user, not just this workspace's members. The provider must additionally pin trust to the customer's Warp team UID (`$TEAM_UID`), carried in the JWT's `teams` claim (a JSON array of team UIDs; warp-server omits the claim for team-less users, which fails the condition below and rejects them). Two controls, defense in depth:

- `--attribute-condition="'$TEAM_UID' in assertion.teams"` — Google STS refuses the exchange for any JWT whose `teams` claim does not contain the workspace's team UID; identities outside the workspace never enter the pool at all.
- `attribute.team` in `--attribute-mapping` — maps to `$TEAM_UID` iff the member belongs to the workspace (CEL ternary; WIF attribute values must be strings, so the list-valued `teams` claim cannot be mapped directly). Step 4's scoped IAM binding keys on this attribute.

Remaining flags:

- `--issuer-uri`: Warp's production OIDC issuer. Staging/local-dev tokens use `https://staging.warp.dev` instead (the E2E test pool is configured against staging).
- `--attribute-mapping`: `google.subject` maps to `assertion.sub` (stable `user:<uid>`) for the IAM binding; `attribute.user_email` maps to `assertion.email` for human-readable audit logs. Do not swap subject to email — emails can change and would invalidate bindings.
- `$TEAM_UID`: the workspace's stable Warp team UID. The final setup docs / admin Models page must surface it beside the other GEAP fields so admins never have to decode a JWT to find it.

```bash
gcloud iam workload-identity-pools providers create-oidc "$PROVIDER_ID" \
  --location=global \
  --workload-identity-pool="$POOL_ID" \
  --issuer-uri="https://auth.warp.dev" \
  --allowed-audiences="//iam.googleapis.com/projects/$PROJECT_NUM/locations/global/workloadIdentityPools/$POOL_ID/providers/$PROVIDER_ID" \
  --attribute-mapping="google.subject=assertion.sub,attribute.user_email=assertion.email,attribute.team=('$TEAM_UID' in assertion.teams) ? '$TEAM_UID' : ''" \
  --attribute-condition="'$TEAM_UID' in assertion.teams" \
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

# Allow ONLY this workspace's identities to impersonate the SA, keyed on the
# attribute.team mapping from Step 3. Least privilege by default; paired with
# Step 3's attribute condition this is defense in depth — either control alone
# already blocks identities outside the workspace.
gcloud iam service-accounts add-iam-policy-binding \
  "$SA_NAME@$PROJECT_ID.iam.gserviceaccount.com" \
  --role="roles/iam.workloadIdentityUser" \
  --member="principalSet://iam.googleapis.com/projects/$PROJECT_NUM/locations/global/workloadIdentityPools/$POOL_ID/attribute.team/$TEAM_UID" \
  --project="$PROJECT_ID"
```

To restrict impersonation to named individuals instead, bind exact subjects — one binding per user, using the stable `google.subject` from Step 3: `--member="principal://iam.googleapis.com/projects/$PROJECT_NUM/locations/global/workloadIdentityPools/$POOL_ID/subject/user:SPECIFIC_UID"`.

> **Test pools only.** The pool-wide wildcard member (`principalSet://.../workloadIdentityPools/$POOL_ID/*`) lets **every identity the provider trusts** impersonate the SA — on a provider missing Step 3's attribute condition, that is any authenticated Warp user. Acceptable only for throwaway pools in isolated test projects (e.g. the E2E project below); never in a customer setup.

**After running these steps,** the admin pastes the audience string (`//iam.googleapis.com/projects/$PROJECT_NUM/locations/global/workloadIdentityPools/$POOL_ID/providers/$PROVIDER_ID`) into `gcpAudience`, and `$SA_NAME@$PROJECT_ID.iam.gserviceaccount.com` into `gcpSaEmail` on the admin Models page.

**Egress note.** Member machines need HTTPS egress to `sts.googleapis.com` and `iamcredentials.googleapis.com` in addition to existing Warp endpoints — relevant for enterprises with allowlisting or TLS-inspecting proxies.

Config-change propagation: the admin's own client re-mints on save; members pick up changed federation config via the existing workspace poll (~10 minutes) or on restart, at which point the mint binding (data model #2) forces the re-mint.

**Member toggle in Settings.** Under `RESPECT_USER_SETTING`, members must be able to opt in. This requires a visible toggle in Settings — without it the default-`false` `AISettings` field is inaccessible and `RESPECT_USER_SETTING` is functionally "GEAP disabled for everyone" in MVP.

Scope: add a `GeapCredentialsToggleRow` under **Settings > Warp Agent** (below the existing Bedrock section). The row is hidden when:
- GEAP is not enabled in the workspace (`is_gemini_enterprise_available_from_workspace()` is false)
- `enablementSetting` is `ENFORCE` (toggle replaced by an "Enabled by your workspace" note, via the same `is_*_toggleable` helper Bedrock uses at `user_workspaces.rs:538`)

When visible, the row shows:
- A labeled toggle bound to `AISettings::gemini_enterprise_credentials_enabled`
- A **"Refresh credentials"** button — calls `force_refresh_geap_credentials(manager, ctx)` directly, same path as the other triggers. This is the sixth, manual trigger, not a separate mechanism.

The refresh button is in MVP scope (not a Follow-up) because the WIF flow has no CLI command for Warp to detect completing. Bedrock has `register_model_event_dispatcher` which auto-remints when the user runs `aws sso login`; GEAP has no equivalent shell command, so the Settings button is the only explicit user-controlled recovery path outside the inline error view. It covers cases like: a `Failed` first mint, a parked refresh chain after a failed background re-mint, or wanting to force a fresh token before a long session.

The refresh button does not show credential status or expiry — that belongs to a future Settings status widget.

## Testing and validation

Maps to `warp-server/specs/REV-1599/PRODUCT.md` Goal 3 (seamless member credentials) and the Data Handling constraints.

- **Unit tests:** `crates/ai/src/api_keys_tests.rs` — token attached when gate+binding match, omitted when disabled, omitted when binding mismatches (uid/audience/sa_email), omitted when logged out, **expired token still attached** (mirroring Grok's `grok_access_token_near_expiry_still_sent` / `api_keys_for_request_includes_expired_grok_token`), previous token served during `Refreshing { previous: Some }` and omitted when `previous: None`; `app/src/ai/geap_credentials_tests.rs` — `GeapWifConfig` parsing/trimming/audience handling, STS response with/without `expires_in` (incl. the JWT-expiry fallback), impersonation response camelCase + RFC 3339 parsing, invalid-timestamp rejection, `needs_refresh` lead-time boundaries **including already-expired → true**, guard no-op while `Refreshing`, request-time safety net no-ops on a fresh token / gate off / mint in flight and re-mints on a parked chain, completion discards the result when the gate flipped off mid-mint, failed re-mint restores `Loaded { previous }` (never discards a working token), timer delay clamped to the 60s floor on a near-expired store; `app/src/workspaces/user_workspaces_tests.rs` — workspace gate on/off/absent-host, `ENFORCE` vs `RESPECT_USER_SETTING` crossed with the member toggle (default `false` → opt-in), logged-out user returns `false` regardless of workspace state.
- **E2E (performed, repeatable):** local warp-server (signs as staging per `local.yaml`) + local client + real GCP project `warp-geap-test-2026`. Verified: mint succeeds from synced host settings alone; GCP Data Access audit log shows the full chain — `serviceAccountDelegationInfo` = `principal://.../warp-geap-pool/subject/<user email>`, `principalEmail` = the SA, `granted: true` on `aiplatform.endpoints.predict`, resource in the customer project; utility-model calls (suggestion/classifier roles) also route to the customer project under `ENFORCE`.
- **Negative cases:** GEAP host disabled → `Disabled`, token absent from requests; empty `gcpAudience` → `Missing`; wrong pool/provider → `ExchangeToken` failure with config hints; missing `workloadIdentityUser` binding → `ImpersonateServiceAccount` 403; expired token → still attached and sent (never silently dropped) → Google rejects it → `InvalidGeminiEnterpriseCredentialsError` → inline error view auto-remints, in both enablement modes.
- **Pre-merge:** `./script/format` + presubmit clippy per repo rules; client release additionally gated on the server schema deploy (see Rollout and gating).

## Parallelization

Not proposed.

## Rollout and gating

No client `FeatureFlag`, deliberately: the rollout switch is the server-side admin config itself. The GEAP host is disabled by default for every team, all client behavior keys off the synced workspace settings, and with no `gcpAudience` present the state machine rests at `Disabled`/`Missing` — requests are byte-identical to today's. A compile-time flag also could not gate the one genuinely risky surface: the cynic `QueryFragment` bakes `gcpAudience`/`gcpSaEmail` into the workspace query text unconditionally, so the real constraint is **deployment ordering**, not feature gating — the server's schema field additions must be deployed to production before a client containing this change ships, or the entire workspace query fails. Mitigation: land and deploy the server schema first, verify the workspace query against staging, and state the ordering requirement in the client PR. Rollback is config-level — the admin disables the GEAP host and clients rest at `Disabled` on the next sync — with no client release needed.

## Risks and mitigations

- **Mid-session token expiry.** Covered by the three lifecycle layers (one-shot timer at `expires_at − 5min`; request-time safety net on every agent request; always-send + Google 401 → inline recovery — see "Token lifecycle: complete case enumeration"). `Refreshing { previous }` keeps serving the old token during re-mints, so healthy sessions never see a Google 401 for plain expiry.
- **Workspace saves and config drift.** `UpdateWorkspaceSettingsSuccess` does not distinguish GEAP fields, but the mint binding (data model #2) makes the refresh guard a no-op unless the audience/SA actually changed — unrelated admin saves cost no STS round-trip, while real federation-config changes re-mint immediately.
- **Customer-side misconfiguration.** The trust bridge is only as tight as the customer's IAM, and Warp is a shared issuer — so the setup steps above bake the tenant-isolation controls into the default commands: the provider `--attribute-condition` pinning the pool to the workspace's team UID, the `workloadIdentityUser` binding scoped to `attribute.team` (the pool `/*` wildcard is explicitly test-only), and a stable `google.subject` mapping (`assertion.sub`) with `attribute.user_email` for human-readable audit logs. Final customer-facing docs must carry the same defaults verbatim, plus a least-privilege (ideally custom) role on the SA.

## Follow-ups

- Cloud-agent (Oz runner) mint path: same exchange keyed off task identity, the GEAP analog of `AwsCredentialsRefreshStrategy::OidcManaged`.
- `warp-terraform` PR to enable Vertex models on Warp staging (requested in PR review): once the client work lands and E2E validation against the personal/test GCP project passes, enable Vertex models on staging infra so GEAP can be exercised end-to-end there.
