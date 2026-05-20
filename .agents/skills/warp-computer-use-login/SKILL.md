---
name: warp-computer-use-login
description: Authenticate Warp in Oz cloud computer-use runs using a managed refresh-token secret and Warp's Paste Auth Token flow. Use when testing logged-in Warp behavior, AI features, onboarding, settings, or bug reports that require an authenticated user.
---

# Warp computer-use login

Use this skill when an Oz cloud computer-use agent needs to log in to Warp before testing authenticated behavior. This is intended for UI verification, onboarding verification, and bug reproduction workflows that require login, including AI features.

The default managed secret is `ONBOARDING_AGENT_FTUE_REFRESH_TOKEN`. It should authenticate as a dedicated non-employee, non-`warp.dev` FTUE test user. If the parent prompt provides a different managed secret name, use that secret instead and apply the same handling rules.

## Managed auth secret

- The managed auth secret is an environment variable injected into the remote run, not a repo file or prompt literal.
- Before doing auth work, verify the environment variable exists and is non-empty without printing it.
- Never echo, log, screenshot, upload, or report the secret value.
- Avoid shell tracing (`set -x`) and avoid commands that place the raw token in shell history or process lists.
- Treat every auth redirect URL containing the refresh token as secret-bearing material, even after URL-encoding.
- Do not pass a token-bearing redirect URL to a shell command, desktop URI handler, browser address bar, process argument, log, artifact, or report. In particular, do not use commands such as `xdg-open`, `gio open`, `open`, or equivalents with the redirect URL.
- Treat private token files as local scratch material only. Do not read them into chat, print them, stage them, commit them, upload them, or include them in artifacts. Delete private token files after use.

## Secure Paste Auth Token process

1. Launch Warp and start the normal login/sign-in flow.
2. Derive the current-run `state` value from Warp's generated login URL if the UI exposes a copied login URL or opens the browser. If the UI does not expose the state after reasonable effort, stop and report an auth blocker rather than bypassing state validation.
3. Normalize the managed secret privately:
   - Trim surrounding whitespace and one pair of surrounding single or double quotes if present.
   - If the secret parses as a URL with a `refresh_token` query parameter, extract that `refresh_token` value and ignore any stale `state` in the secret.
   - Otherwise, treat the trimmed secret as the raw refresh token.
4. URL-encode the extracted refresh token and current-run `state` separately as query parameter values.
5. Build a current-run redirect URL in this shape: `warp://auth/desktop_redirect?refresh_token=<url-encoded-normalized-refresh-token>&deleted_anonymous_user=true&state=<url-encoded-current-state>`.
   - Do not include `user_uid` unless it is already present in a provided desktop redirect URL; it is not required for this flow.
6. Keep the normalized redirect URL only in a clipboard value or a private temporary file with user-only permissions.
7. Return to Warp and use the visible Paste Auth Token path:
   - Click the `Click here to paste your token from the browser` link, `Paste Auth Token` button, or equivalent pasted-token control shown by Warp.
   - Focus the auth token text input that appears.
   - Paste the prepared redirect URL into that input and submit it through Warp's UI so Warp parses and validates it.
8. Delete any private temporary files immediately after use and clear the clipboard if the environment supports doing so safely.
9. If the Paste Auth Token UI cannot be reached or automated safely, stop and report an auth blocker instead of parsing the redirect in place of Warp, using a desktop URI handler, browser address bar, or shell command with the token-bearing URL.

## Preferred authenticated path

- Use Warp's built-in Paste Auth Token flow rather than visiting real OAuth providers, invoking a desktop URI handler, or asking the agent to parse or validate the redirect URI itself.
- Do not preflight the token with Firebase Secure Token before handing it to Warp. Warp's desktop redirect handler only requires `refresh_token` and `state`; `user_uid` is optional, and `deleted_anonymous_user=true` handles the anonymous-user override case.
- If Warp rejects the normalized redirect, report the non-sensitive user-visible error and classify whether the secret appeared to be a raw token or a desktop redirect URL, without reporting token contents.
- If auth succeeds, continue through any remaining onboarding screens with default or conservative options unless the parent prompt says otherwise.

## Reporting requirements

Report back:

- Whether login succeeded.
- Which managed secret environment variable name was used, but not its value.
- The non-sensitive step where auth blocked or failed, if applicable.
- Any user-visible error text with secrets redacted.
- Whether Warp reached a usable authenticated terminal session.
- Artifact directory path and screenshot list.

Do not copy the authenticated user's email address into logs, shell output, or final text reports unless the parent prompt explicitly asks. It may be visible in screenshots when the parent prompt requests account-state evidence.
