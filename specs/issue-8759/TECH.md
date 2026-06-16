# ChatGPT / Codex OAuth Subscription — Tech Spec

**Issue:** [#8759](https://github.com/warpdotdev/warp/issues/8759)
**Status:** Draft
**Branch:** `feat/codex-oauth-subscription`

## Architecture Overview

This feature adds **client-side OAuth** for OpenAI's WHAM backend, mirroring the SuperGrok subscription pattern almost 1:1. The architecture is additive: a new `crates/ai/src/codex_subscription/` module + fields on existing types. No existing code paths are modified for users who don't connect a ChatGPT account.

**Phase A** (this PR): OAuth flow, token storage/refresh, and model picker wiring.
**Phase B** (blocked on warp-proto-apis + warp-server): Token injection into agent requests and WHAM routing.

### Data flow (high level)

```
User clicks "Connect ChatGPT account"
  → Browser-based OAuth 2.0 PKCE flow (client-side)
  → Tokens persisted to macOS Keychain (secure_storage)
  → Proactive refresh timer established
  → (Phase B) On agent request: token injected as codex_oauth_access_token in ApiKeys proto
  → (Phase B) warp-server routes Codex models to WHAM using this token
```

### Key difference from SuperGrok

| | SuperGrok | Codex OAuth |
|---|---|---|
| OAuth endpoints | `auth.x.ai` | `auth.openai.com` |
| Token endpoint | xAI's standard OAuth | OpenAI's standard OAuth |
| API backend | xAI API | WHAM (`chatgpt.com/backend-api/wham`) |
| Extra headers needed | None | `ChatGPT-Account-Id` (extracted from JWT — Phase B) |
| Extra authorize params | `plan=generic`, `referrer=warp` | `id_token_add_organizations=true`, `codex_cli_simplified_flow=true`, `originator=warp` |
| Expiry format | Unix seconds (standard) | Unix milliseconds |
| Proto field | `grok_oauth_access_token` | New: `codex_oauth_access_token` (Phase B — needs proto changes) |
| Server routing | xAI models → xAI API | Codex-tagged models → WHAM (Phase B) |

## Proto API Changes (warp-proto-apis)

**This is the most critical dependency.** The `warp_multi_agent_api` crate (generated from `warp-proto-apis.git`) needs a new field on the `ApiKeys` message:

```protobuf
message ApiKeys {
  // ... existing fields ...
  string codex_oauth_access_token = N;
  string chatgpt_account_id = N+1;  // optional, for ChatGPT-Account-Id header
}
```

**Approach:** Fork `warp-proto-apis`, add the fields, publish to Git, update the rev in `Cargo.toml`. The `patch` section in `Cargo.toml` (line 548) already shows this is a known workflow — we can also use the `[patch]` mechanism to point at a local checkout during development.

**Alternative:** Reuse the existing `openai` field and pass the OAuth token through there, letting the server differentiate WHAM models from standard OpenAI models by the model ID/slug in the request. This avoids proto changes but is semantically leaky. **Not recommended** — the server needs to know whether to send the token to `api.openai.com` (standard) or `chatgpt.com/backend-api/wham` (WHAM), and a separate field makes that unambiguous.

## Client-Side Modules

### 1. `crates/ai/src/codex_subscription/oauth.rs` — OAuth protocol

New file. Based on `grok_subscription/oauth.rs` with these modifications:

**Constants:**
| Constants | SuperGrok value | Codex value |
|---|---|---|
| `CLIENT_ID` | `b1a00492-...` | `app_EMoamEEZ73f0CkXaXp7hrann` |
| `AUTHORIZE_URL` | `https://auth.x.ai/oauth2/authorize` | `https://auth.openai.com/oauth/authorize` |
| `TOKEN_URL` | `https://auth.x.ai/oauth2/token` | `https://auth.openai.com/oauth/token` |
| `SCOPE` | `openid profile email offline_access grok-cli:access api:access` | `openid profile email offline_access` |
| `REDIRECT_PORT` | `56121` | `1455` |
| `REDIRECT_HOST` | `127.0.0.1` | `localhost` |

**OpenAI-specific authorize params** (added to `authorize_url()`):
```
id_token_add_organizations=true
codex_cli_simplified_flow=true
originator=warp
```

**TokenResponse:** Same shape but `expires_in` is milliseconds. When converting to absolute `SystemTime`, divide by 1000 for the Duration calculation.

**Note:** The `plan=generic` param from the Grok copy was intentionally removed — OpenAI's auth server does not recognise it. The `redirect_uri` host was changed from `127.0.0.1` to `localhost` because OpenAI's Auth0 tenant does exact-string redirect URI matching against its allowlist.

**Account ID extraction (Phase B):** After a successful token response, decode the `id_token` (JWT, no signature verification needed) and extract the `chatgpt_account_id` claim. This field is required as a `ChatGPT-Account-Id` header on WHAM requests. Not yet implemented — deferred to Phase B alongside warp-proto-apis changes.

### 2. `crates/ai/src/codex_subscription/mod.rs` — Refresh orchestration

New file. Copy `grok_subscription/mod.rs` verbatim with type renames:

| SuperGrok type | Codex type |
|---|---|
| `GrokTokens` | `CodexTokens` |
| `grok_refresh_allowed` | `codex_refresh_allowed` |
| `grok_refresh_in_flight` | `codex_refresh_in_flight` |
| `store_grok_tokens` | `store_codex_tokens` |
| `set_grok_refresh_allowed` | `set_codex_refresh_allowed` |
| `refresh_grok_tokens_if_needed` | `refresh_codex_tokens_if_needed` |

The refresh logic is provider-agnostic and works as-is: proactive timer fires 5 min before expiry, request-time safety net on every agent request, in-flight deduplication, always-send-expired-tokens policy.

One difference: OpenAI may return `expires_in` in milliseconds vs seconds. The `grok_tokens_from_response` equivalent must handle this.

### 3. `crates/ai/src/api_keys.rs` — Token storage + injection

**New types (alongside `GrokTokens`):**

```rust
const CODEX_SECURE_STORAGE_KEY: &str = "CodexOAuthTokens";

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct CodexTokens {
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
    /// Absolute time at which `access_token` expires.
    /// OpenAI returns this in milliseconds; converted to SystemTime on receipt.
    #[serde(default)]
    pub expires_at: Option<SystemTime>,
    /// When the user originally connected via OAuth. Carried across refreshes.
    #[serde(default)]
    pub connected_at: Option<SystemTime>,
    /// (Phase B) ChatGPT account ID extracted from the id_token JWT.
    /// Required as ChatGPT-Account-Id header on WHAM requests.
    /// Not yet implemented — deferred until chatgpt_account_id header is needed.
}
```

`CodexTokens::access_token_for_request()` and `needs_refresh()` — same as `GrokTokens`.

**`ApiKeyManager` additions:**
```rust
pub struct ApiKeyManager {
    // ...existing fields...
    codex_tokens: Option<CodexTokens>,
    #[cfg(not(target_family = "wasm"))]
    pub(crate) codex_refresh_allowed: bool,
    #[cfg(not(target_family = "wasm"))]
    pub(crate) codex_refresh_in_flight: bool,
    codex_secure_storage_write_version: u64,
}
```

**`api_keys_for_request()` (Phase B):** The spec originally showed injection code here, but the `codex_oauth_access_token` field does not yet exist in the `ApiKeys` proto message. The code explicitly marks this as deferred:
```rust
// Phase A: Codex tokens are stored and refreshed client-side, but not
// yet sent on the wire — blocked on a `warp-proto-apis` fork that adds
// `codex_oauth_access_token` to the `ApiKeys` proto message.
```

**Secure storage:** Add `load_codex_tokens_from_secure_storage()` and `write_codex_tokens_to_secure_storage()` following the same deferred-write pattern as the Grok equivalents.

### 4. `app/src/ai/llms.rs` — Provider awareness

Add `LLMProvider::Codex` variant (or map to existing `OpenAI` — see design decision below).

**Design decision:** The LLM models served through WHAM are the same OpenAI models (o3, o4-mini, GPT-4o, etc.). The provider is `LLMProvider::OpenAI`. The differentiation happens at the **routing host** level — the server knows to route these models to WHAM when a `codex_oauth_access_token` is present in the `ApiKeys`. The client does not need a separate provider enum; it just needs to attach the token when a ChatGPT account is connected and the user hasn't explicitly pasted an OpenAI API key.

The `is_using_api_key_for_provider()` check for `LLMProvider::OpenAI` should also consider `CodexTokens` as having an "API key" for OpenAI (same logic as SuperGrok for xAI).

### 5. `app/src/settings_view/ai_page.rs` — Settings UI

**New button handles** (alongside `grok_connect_button` / `grok_disconnect_button`):
- `codex_connect_button` — "Connect" button
- `codex_disconnect_button` — "Disconnect" button

**New action handlers in `AISettingsPageAction`:**
- `ConnectChatGptAccount` → calls `start_codex_oauth(ctx)` (same pattern as `start_grok_oauth`)
- `DisconnectChatGptAccount` → calls `manager.set_codex_tokens(None, ctx)`

**New render method:** `render_codex_subscription_row()` — parallel to `render_grok_subscription_row()` with different label/description copy:

- Label: "Connect ChatGPT account"
- Description: "Connect your ChatGPT Plus or Pro account to use OpenAI models through your subscription instead of paying per-token API costs."

**Feature gating:**
```rust
if FeatureFlag::ChatGptAuth.is_enabled() {
    column.add_child(/* codex row */);
}
```

### 6. `app/src/lib.rs` — Initialization wiring

Parallel to the Grok subscription wiring:
```rust
#[cfg(not(target_family = "wasm"))]
if FeatureFlag::ChatGptAuth.is_enabled() {
    ctx.subscribe_to_model(&UserWorkspaces::handle(ctx), |manager, event, ctx| {
        if matches!(event, UserWorkspacesEvent::TeamsChanged) {
            let allowed = UserWorkspaces::as_ref(ctx).is_byo_api_key_enabled(ctx);
            manager.set_codex_refresh_allowed(allowed, ctx);
        }
    });
    let allowed = UserWorkspaces::as_ref(ctx).is_byo_api_key_enabled(ctx);
    manager.set_codex_refresh_allowed(allowed, ctx);
}
```

### 7. `app/src/features.rs` / `app/Cargo.toml` — Feature flag

Add:
- `FeatureFlag::ChatGptAuth` in `warp_core/src/features.rs`
- `chatgpt_auth = []` in `app/Cargo.toml` features (not in default — flagged off initially)
- The flag in `app/src/features.rs` in the flag-to-compile-time list

## Server-Side Changes (warp-proto-apis + warp-server)

**Two separate repos need changes:**

1. **warp-proto-apis:** Add `codex_oauth_access_token` and `chatgpt_account_id` fields to the `ApiKeys` protobuf message. Re-generate the Rust client.

2. **warp-server:** Route models tagged for Codex/WHAM to `https://chatgpt.com/backend-api/wham` with:
   - `Authorization: Bearer {codex_oauth_access_token}`
   - `ChatGPT-Account-Id: {chatgpt_account_id}`
   - `Content-Type: application/json`
   - WHAM-specific request shaping: `store=false`, `input_text` content type, `instructions` (system prompt), stateless conversation history

## Files Changed (Complete List)

### Client (this repo)
| File | Change |
|---|---|
| `crates/ai/src/codex_subscription/oauth.rs` | **New** — OAuth PKCE flow for OpenAI |
| `crates/ai/src/codex_subscription/mod.rs` | **New** — refresh orchestration |
| `crates/ai/src/codex_subscription/oauth_tests.rs` | **New** — OAuth tests |
| `crates/ai/src/lib.rs` | Add `pub mod codex_subscription` |
| `crates/ai/src/api_keys.rs` | Add `CodexTokens`, secure storage (_injection deferred to Phase B — see code comment_) |
| `crates/ai/src/api_keys_tests.rs` | Add Codex token tests |
| `app/src/ai/llms.rs` | Update `is_using_api_key_for_provider` for OpenAI + CodexTokens |
| `app/src/settings_view/ai_page.rs` | Add Connect/Disconnect UI + action handlers |
| `app/src/lib.rs` | Wire Codex refresh subscription |
| `app/src/features.rs` | Register `ChatGptAuth` flag |
| `app/Cargo.toml` | Add `chatgpt_auth` feature |
| `crates/warp_features/src/lib.rs` | Add `ChatGptAuth` variant |

### Proto API (separate repo — warp-proto-apis) — Phase B
| File | Change |
|---|---|
| `apis/multi_agent/v1/settings.proto` | Add `codex_oauth_access_token` + `chatgpt_account_id` fields |

### Server (separate repo — warp-server) — Phase B
| File | Change |
|---|---|
| `logic/ai/llm/model_fallback_chain.go` | Add WHAM route when `codex_oauth_access_token` present |
| `router/handlers/generate_multi_agent_output.go` | Extract + redact Codex token + account ID |

## Testing Plan

### Unit tests (client)
| Test | What it covers |
|---|---|
| `codex_tokens_from_response` | TokenResponse → CodexTokens conversion, ms expiry handling |
| `needs_refresh` boundary tests | Lead-time boundaries, already-expired → true |
| `access_token_for_request` | Non-empty returns Some, empty returns None |
| `store_and_load_codex_tokens` | Round-trip through secure storage |
| `codex_disconnect_clears_tokens` | Setting None clears storage |
| `codex_tokens_persist_across_new` | Previously stored tokens survive a fresh ApiKeyManager |
| `oauth_authorize_url_includes_openai_params` | OpenAI-specific params are present |
| `oauth_callback_code_exchange` | Code exchange succeeds with valid response |
| `oauth_authorize_url_no_plan_param` | `plan=generic` is not sent (OpenAI doesn't recognise it) |
| `oauth_redirect_is_localhost` | Uses `localhost` not `127.0.0.1` (OpenAI redirect URI matching) |

_Note: `api_keys_for_request` token injection tests are Phase B — the proto field doesn't exist yet._

### Integration testing
- Manual E2E (Phase A): run local build, click Connect, complete OAuth, verify "Connected on..." status appears in Settings, verify OpenAI models appear in the model picker dropdown
- Disconnect flow: click Disconnect, verify tokens cleared from Keychain, verify Status returns to "Disconnected" in Settings
- Manual E2E (Phase B): blocked on warp-proto-apis + warp-server changes — no WHAM routing without the proto fields

### Verification against PRODUCT.md
- [ ] Success criteria 1–6 pass
- [ ] Edge cases from PRODUCT.md are exercised

## Rollout

| Phase | Scope | Gating |
|---|---|---|
| 1. Client (Phase A — this PR) | OAuth flow, token storage/refresh, model picker | Feature-flag gated (`chatgpt_auth` Cargo feature) |
| 2. Dogfood | WarpDev build with feature flag on | `FeatureFlag::ChatGptAuth` in DOGFOOD_FLAGS (Phase B also needed) |
| 3. Preview | Friends of Warp | `FeatureFlag::ChatGptAuth` in PREVIEW_FLAGS |
| 4. Stable | All users | Default-on via Cargo feature flag |

Given this is a fork+build project, only phase 1 applies initially.

## Risks

| Risk | Mitigation |
|---|---|
| OpenAI changes OAuth flow or revokes third-party use | The flow is the same one Codex CLI and OpenCode use; changes would break those too. Monitor `codex-oauth` repo for upstream changes. |
| WHAM API changes | Stateless API with standard Responses contract; warp-server routes through it like any other provider backend. |
| Proto API changes in upstream warp-proto-apis | Use `[patch]` in Cargo.toml to pin to our fork. Rebase periodically. |
| ChatGPT-Account-Id extraction breaks | JWT payload parsing without signature verification is safe (the token was just issued by OpenAI); three-level fallback covers format changes. |
| Port 1455 conflicts with another app | Rare — Codex CLI already uses this port. Handle bind failure with a descriptive toast. |
