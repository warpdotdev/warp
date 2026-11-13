*Spec: warp client skips IAP when pointed at a local server (REMOTE-2540)*

Linear: https://linear.app/warpdotdev/issue/REMOTE-2540

## Problem
A Dev-channel (`oz-dev`) build compiles in an `IapConfig`, so the client builds
`IapState` and gates startup user authentication on IAP access purely by build
flavor — independent of which server it points at. When `warp-server`'s
`./script/oz-local` launches such a build against the local `oz-local` server
(`http://localhost:8080`, not behind IAP, no IAP credentials), the IAP token
mint/refresh fails and the client errors out ("IAP credential refresh failed")
instead of talking to the local server.

## Decision
Auto-disable IAP when the resolved `server_root_url` host is local. "Local" is an
exact-match allowlist: `localhost`, `127.0.0.1`, `::1` (`[::1]` as
`Url::host_str` yields it), and `host.docker.internal`. Every other host —
including staging and anything unrecognized — keeps IAP, so the classifier fails
safe toward IAP-enabled.

The resolved server URL is the **sole authority**: there is no env var, flag, or
other opt-out. This makes it structurally impossible to disable IAP against
staging — the only way to skip IAP is to point the URL at a local host. Skipping
IAP removes only the IAP pre-gate; normal user authentication still runs. Release
channels (Stable/Preview/Oss) have no `IapConfig` and ignore server-URL
overrides, so they never reach this path.

## Gate location
`initialize_app` builds `iap_state` in `app/src/lib.rs`. Gating that single
`Option` to `None` for a local URL disables IAP end to end:
`IapManager::is_enabled()` becomes `false` (startup auth runs immediately),
`IapManager::start_refresh` early-returns (the runner WIF self-mint stays inert),
and `ServerApi` gets no IAP state. No other call site changes. The classifier is
a pure `host_is_local` free function in `crates/warp_core/src/channel/state.rs`
with a `ChannelState::server_root_url_is_local()` wrapper, kept as a free
function so it is unit-testable without the global channel state.

## Validation & verification criteria
1. Unit tests in `crates/warp_core/src/channel/state_tests.rs`: `host_is_local`
   returns `true` for each local host (`localhost`, `127.0.0.1`, `[::1]`,
   `host.docker.internal`, with/without port).
2. Negative test (security-critical): `host_is_local` returns `false` for
   `staging.warp.dev` and `app.warp.dev`, so IAP stays enforced.
3. Edge cases: unparseable input and substring-only hosts
   (`localhost.evil.example.com`, `mylocalhost.dev`) are not local.
4. `./script/format --check`, `cargo clippy -p warp_core -p warp --all-targets
   --tests -- -D warnings`, and `cargo nextest run -p warp_core channel::state`
   all pass; CI is the full-suite backstop.
5. End-to-end (manual, dev machine): a Dev build launched by `./script/oz-local`
   reaches `http://localhost:8080` with no IAP error and normal user auth runs;
   pointed at `https://staging.warp.dev` IAP is still established.

Headless/backend startup change with no rendered UI surface, so per
factory-verification no computer-use visual proof is required.
