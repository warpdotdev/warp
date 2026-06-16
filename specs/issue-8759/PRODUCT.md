# ChatGPT / Codex OAuth Subscription — Product Spec

**Issue:** [#8759 — OAuth Support for ChatGPT Plus/Pro (Bypass BYOK API Costs)](https://github.com/warpdotdev/warp/issues/8759)
**Status:** Draft
**Branch:** `feat/codex-oauth-subscription`

## Problem

Warp Agent users with ChatGPT Plus or Pro subscriptions pay per-token API costs (BYOK) to use OpenAI models, even though their flat-rate subscription already grants access to the same models through Codex / WHAM. The per-token cost is a barrier to heavy daily use of Warp as a full-time coding agent.

## Solution

Allow users to connect their ChatGPT account via OAuth, obtaining tokens that let agent requests route through OpenAI's **WHAM** (Workspace-Handled Agent Model) backend — the same backend the Codex CLI uses. Requests authenticated this way consume the user's ChatGPT subscription, not per-token API billing.

The experience mirrors the existing **SuperGrok** subscription flow: a "Connect ChatGPT account" button in Settings opens a browser-based OAuth consent screen, tokens are obtained via PKCE, stored securely in the macOS Keychain, proactively refreshed before expiry, and attached to outbound agent requests.

## User Experience

### Settings: "Connect ChatGPT account" row

In **Settings > Warp Agent > API Keys**, below the existing "Connect SuperGrok subscription" row, a new "Connect ChatGPT account" row appears when the `ChatGptAuth` feature flag is enabled:

| State | UI |
|---|---|
| **Disconnected** | Label "Connect ChatGPT account" + description + **Connect** button |
| **Connected** | Label "Connect ChatGPT account" + description + green check + "Connected on {date}." + **Disconnect** button |
| **OAuth in progress** | "Opening your browser to connect your ChatGPT account…" toast with "Copy URL" fallback |
| **OAuth failure** | Error toast with failure reason |

### What changes for the user

| Before | After |
||---|---|
| Manually paste OpenAI API key in the API keys editor | Click "Connect ChatGPT account" → browser OAuth → token auto-managed |
| Pay per-token for OpenAI models | **(Phase B — see below)** Requests will route through WHAM once warp-proto-apis and warp-server support is added |
| Token management is manual (rotate keys, track usage) | OAuth tokens auto-refresh before expiry; disconnect is one click |

### What doesn't change

- Model selection: the same OpenAI models appear in the model picker when a ChatGPT account is connected (same UX as SuperGrok)
- BYO API key field for OpenAI remains — users who prefer to paste a key still can
- Warp credit fallback behavior is unchanged
- **(Phase B)** Actual request routing through WHAM is blocked on warp-proto-apis changes

### Scope boundaries (MVP)
| In scope (Phase A) | Out of scope (Phase B + follow-ups) |
||---|---|
| Browser-based OAuth PKCE login | Token injection into WHAM-routed requests (blocked on warp-proto-apis `codex_oauth_access_token` field + warp-server WHAM routing) |
| Token refresh before expiry | Multi-account support |
| Settings Connect/Disconnect + status | Account switcher UI |
| Model picker shows OpenAI models when subscription is connected (same as SuperGrok) | Settings credential status widget (expiry, last-refresh) |
| Feature-flag gated | Cloud agent (Oz) WHAM routing |
| | Device-code auth for headless environments |

## Success Criteria

1. Clicking "Connect ChatGPT account" opens the OpenAI OAuth consent screen in the browser
2. After approving, the OAuth callback is received, tokens are stored, and a "Connected on..." status appears
3. **(Phase A)** OpenAI models appear as usable in the model picker when a ChatGPT subscription is connected (same UX as SuperGrok)
4. **(Phase B)** Agent requests using eligible OpenAI models route through WHAM with the OAuth token — blocked on `codex_oauth_access_token` and `chatgpt_account_id` fields in warp-proto-apis `ApiKeys` message
5. Disconnecting clears stored tokens from Keychain and stops sending them
6. Tokens are proactively refreshed before expiry (no user interruption)
7. Re-opening the app after restart shows the previously connected account as still connected

## Edge Cases

- **OAuth callback timeout (5 min):** user gets a toast with the failure reason; no dangling browser tab
- **Token refresh failure:** previous token continues being used (the server is the authority on validity); next request triggers a background retry
- **ChatGPT account revoked / password changed:** **(Phase B)** WHAM returns 401 → request fails with an auth error; user clicks Disconnect then re-connects
- **Network offline during OAuth:** bind failure on the loopback server surfaces immediately as a toast
- **App closed during OAuth:** OAuth attempt is abandoned; no state leaked
- **Multiple OAuth flows simultaneously:** loopback port bind failure → toast explaining another login is in progress
