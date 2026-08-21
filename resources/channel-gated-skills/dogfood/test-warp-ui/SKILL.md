---
name: test-warp-ui
description: >
  Guides testing Warp UI features and changes using the computer use tool.
  Use this skill only when the computer_use tool is available to the agent.
  Covers launching Warp and verifying UI behavior.
user-invocable: false
---

# Computer Use for Warp UI Testing

Use the `computer_use` tool to visually test that Warp looks and behaves as intended after UI changes.

## Running Warp

Launch Warp from the repository root. The GUI requires a *user*-account key and rejects a service-account one, so the command depends on what kind of key you have and where it lives:

- A user-account key in `WARP_API_KEY`: omit the flag entirely — `--api-key` is bound to that variable, so Warp reads it automatically:

  ```bash
  cargo run --bin warp
  ```

- A user-account key anywhere else, or a *service-account* key in `WARP_API_KEY` — the usual cloud-sandbox case, and the reason a bare `cargo run --bin warp` lands on onboarding there: pass the user key explicitly.

  ```bash
  env -u WARP_API_KEY cargo run --bin warp -- --api-key "$YOUR_USER_ACCOUNT_KEY"
  ```

  The `env -u` is hygiene, not precedence: an explicit `--api-key` already beats the variable, since clap reads `env` only when the flag is absent. Dropping it keeps the service-account key out of the app and anything it spawns.

Always pass `--bin warp` explicitly. That target is the local internal channel (`app/src/bin/local.rs`, `Channel::Local` — it carries dogfood feature flags but is not the dogfood channel). A plain `cargo run` builds `warp-oss`, the workspace `default-run`, whose server config is baked to production (`app/src/bin/oss.rs`), so a staging key cannot authenticate it. The key itself is not channel-gated: `--api-key` is bound to `WARP_API_KEY` unconditionally (`crates/warp_cli/src/lib.rs`) and passed straight through on every channel (`app/src/lib.rs`). What differs is the server on the other end.

Key-based startup authentication is not known to work in a cloud sandbox. Read "If the app is logged out" below before you plan a verification that needs a logged-in GUI there.

Initial builds may take several minutes; subsequent incremental builds are faster.

### Verify the launch is authenticated

Check both of these on the running app before you test anything. Each is an observable state, so each can actually fail.

- **The window.** Authenticated Warp opens straight to the terminal. The logged-out onboarding/sign-in screen means the login did not happen — go to "If the app is logged out" below rather than driving the session anyway.
- **The target.** Startup logs `Starting warp with channel state ChannelState { channel: Local, ... }` (`app/src/lib.rs`), and that state carries the server root URL the app will talk to. `channel: Oss`, or a `server_root_url` of `https://app.warp.dev`, means the OSS/production binary came up and no staging key can authenticate it — stop and relaunch with `--bin warp`.
- **Where that line goes.** The GUI sets no log destination (`log_destination` returns `None` for `LaunchMode::App`, `app/src/lib.rs`), so it falls to a tty heuristic (`crates/warp_logging/src/native.rs`): stderr when stdout is a tty (or `CI`/`WARP_INTEGRATION` is set), otherwise the channel's log file — `logfile_name` under `~/Library/Logs/` on macOS, `warp_core::paths::state_dir()` on Linux and Windows. Backgrounding or redirecting the launch, the normal way to keep driving a shell, takes the file branch, so the line is absent from what you captured. An absent line fails the check: read the log file, or relaunch on a tty. Never read silence as a pass.

The log distinguishes nothing about the login itself. `Authenticating via pending API key` (`app/src/auth/auth_manager.rs`) is logged *before* the attempt, and neither outcome is logged after it; the IAP cache fast path (`crates/warp_server_client/src/iap.rs`) is silent on success as well. An absence of errors is not a successful login, which is why the window is the signal.

### If the app is logged out

In a cloud sandbox, expect it: **no API-key path is known to authenticate a locally built GUI there today.** Do this, in order:

1. **Pass a user-account key** if you have not already — see the launch commands above. `Unauthorized: Expected a user account` in the log means the key, not the launch, is the problem.
2. **Clear the IAP wall**, before debugging anything else, or a rejected login and a login never attempted look identical. The cache is channel-scoped: sandbox setup writes a valid hour-long token to `~/.warp-dev/staging/iap_cache.jwt`, and `Channel::Local` reads `~/.warp-local/staging/iap_cache.jwt`, which does not exist. Copy it across.
3. **Capture the surface without a login** rather than abandoning the proof: the integration-test harness (`crates/integration`) boots a real Warp app with no account and can construct the state under test — author or extend a test with the `gui-integration-test` skill, and record it with `gui-integration-test-video`. Hardcoding or mocking (below) is the last resort, not the first fallback, and a capture made against a mocked or hardcoded state is never a live Cloud Mode (or other authenticated-surface) verification — say plainly that it's mocked.

Why it is logged out: two walls, both reproduced against a real `cargo run --bin warp`.

- **The key is the wrong kind.** `WARP_API_KEY` in a cloud sandbox is a *service-account* key, and the GUI requires a user account. The app lands on onboarding with `Unauthorized: Expected a user account` and `invalid input syntax for type uuid: "serviceAccount:<uid>"`, carrying the key's own UID. A genuine user-account key clears those errors, and then the app's own startup calls — user settings, LLMs, available harnesses, request limits — all return `403 Forbidden` for reasons not yet understood. That is a surface-specific rejection rather than a dead key: the public REST API answers `200` to the same key from the same sandbox.
- **Staging dogfood gates the API-key login behind IAP.** Unlike the TUI (which authenticates immediately and resolves IAP out of band), the GUI withholds `--api-key`/`WARP_API_KEY` authentication until an IAP token is loaded (`authenticate_user_after_iap_access` in `app/src/lib.rs`). The sandbox self-mints one via Workload Identity Federation — a valid `OZ_RUN_ID` enables the runner-context mint path (`app/src/lib.rs`), which exchanges the injected `WARP_STAGING_IAP_BOOTSTRAP_JWT` for a token (`crates/warp_server_client/src/iap.rs`), no `gcloud` needed. That bootstrap JWT lives exactly 900 seconds from the start of the run. Past it the mint dead-ends (`Staging IAP access unavailable before startup user authentication`) and login is never attempted at all, so a run older than 15 minutes never reaches the key.

Copying the cache across makes the IAP failure disappear past the 900-second window, which moves the failure from "login never attempted" to "login attempted and rejected" — the evidence worth reporting.

Settings > Account carries a "Staging IAP credentials" status widget (`app/src/settings_view/main_page.rs`), but Settings is unreachable from the onboarding screen — no menu bar, no gear icon, and Ctrl+Comma does nothing — so it cannot be read in the state that needs it.

A non-API-key path does authenticate a GUI in a cloud sandbox: the `gui-onboarding-verification-skill` drives Warp's own Paste Auth Token flow with a Firebase refresh token, which never touches API-key auth or the IAP gate above. Two limits before you reach for it — it runs against an installed *stable* build, so it cannot show an unmerged diff, and its secret is not provisioned in every sandbox. Untested for verification work, but it is the first alternative to try.

## Testing Workflow

### 1. Hardcode or Mock Data (When Needed)

If you just need to verify that a specific UI looks correct, it can be useful to hardcode or mock data so the UI state is immediately reachable without navigating a full flow. This is optional — skip this step when testing end-to-end flows that should work naturally.

Examples of when to hardcode:

- **Conditional UI**: The feature only appears under certain conditions (e.g., a specific setting, a non-empty data set, an active subscription) — hardcode the condition so the UI always appears.
- **Feature flags**: The feature is behind a flag that isn't enabled yet — enable it directly.
- **Error states**: You want to test error handling UI — hardcode error responses or failure conditions.

Keep mocked changes minimal and focused — only change what's necessary to reach the UI state under test.

### 2. Invoke Computer Use

Call the `computer_use` tool with a task description that includes:

- The command to build and launch Warp from the repo root: `cargo run --bin warp` when `WARP_API_KEY` holds a user-account key, otherwise `env -u WARP_API_KEY cargo run --bin warp -- --api-key "$YOUR_USER_ACCOUNT_KEY"`. In a cloud sandbox expect the logged-out onboarding screen even then — read "If the app is logged out" above before you build a task around an authenticated surface
- Step-by-step instructions for navigating to the UI being tested
- **Specific observations to report**: describe exactly what elements, text, colors, layout, or states the tool should observe and describe back
- Do **not** include expected values in the task — the tool should report what it sees, not judge correctness

### 3. Verify Results

Compare the observations returned by `computer_use` against your expectations. If the UI doesn't match, investigate and adjust the code or mocks accordingly.

## Tips

- **Be specific in task descriptions**: Instead of "check if the dialog looks right," say "open Settings, click the General tab, and describe the text and layout of the first section."
- **Test one thing at a time**: Focused tests are easier to debug when observations don't match expectations.
- **Build before invoking**: Always confirm the build succeeds before calling `computer_use`. The tool cannot fix build errors.
