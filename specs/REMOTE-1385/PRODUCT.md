# Harness Auth Preflight + Runtime Failure Detection

## Summary
Cloud agent runs using third-party harnesses (Claude Code, Codex) should detect authentication and runtime API failures early and report them with actionable error messages instead of letting the harness drift mid-run with an opaque failure.

This feature has two parts:
1. A lightweight **authentication preflight check** that runs before the main harness CLI command starts. Same UX as a normal setup-commands block.
2. A **background output scanner** that watches the running harness block for known runtime failure substrings (e.g. invalid API key, exhausted credits) and terminates the run on the first hit. This replaces the original billing preflight check, which burned a test API request and told us nothing the real run wouldn't surface seconds later.

## Behavior

### Preflight sequence
1. After harness environment config is written (auth.json, config files, etc.) but before the main harness CLI command starts, the driver runs the **authentication check** for the harness (if one is defined).
2. If the authentication check fails (non-zero exit code), the task is marked as failed on the server with a `HarnessAuthFailed` error indicating that login credentials are invalid, and the process exits.
3. If the authentication check passes (exit code 0), the driver proceeds to start the main harness CLI command. The runtime failure scanner (see "Runtime failure detection" below) starts at the same time.
4. The authentication check is optional per-harness. A harness that returns no command skips this step silently.

### Per-harness commands
5. **Claude Code**: Authentication check: `claude auth status --json`
6. **Codex**: Authentication check: `codex login status`
7. **Gemini**: No authentication check. Skipped silently.
8. Future harnesses opt in by implementing the `auth_check_command` trait method; the default is no check.

### Success / failure semantics
9. Authentication check success is determined solely by exit code. Exit code 0 = pass; any non-zero exit code = fail.
10. The authentication check is executed as a visible block in the shared session UI and is classified as part of the existing **Set up environment commands** collapsible group, in line with the other environment-setup blocks. The block is individually expandable to reveal the CLI's captured output, and the whole group collapses to "Ran setup commands" once the harness session begins. On preflight failure, the group remains expanded so the user can inspect the failing block directly.
11. The captured block output is also stashed in the driver-side error `detail` so the same text reaches server logs and the failure status message.

### Runtime failure detection (replaces the billing preflight)
12. While the main harness CLI command is running, the driver runs a background scanner that watches the harness's output block for substrings indicating an API request failure (invalid key, exhausted credits, no billing access, quota exceeded, etc.). The set of substrings is supplied per-harness via the `runtime_error_patterns` trait method.
13. The scanner uses an **adaptive polling schedule**: 5-second polls for the first 30 seconds, then 15-second polls for the next 60 seconds (total budget: 90 seconds, ~10 polls). After the schedule completes without a hit, the scanner stops and the harness continues uninterrupted.
14. Each poll reads the harness block's visible output (no ANSI escape sequences, secrets obfuscated) and scans for the first matching pattern via the same DFA infrastructure used by the find feature (`RegexDFAs::new_many`). Matching is case-insensitive.
15. On the first hit, the scanner enters a **stall-confirmation loop** before failing the task (see "Stall confirmation" below). Only after the harness output has stabilized does the driver send `/exit` to the harness and report the failure to the server.
16. The scanner is a no-op for harnesses that return an empty `runtime_error_patterns` slice (e.g. Gemini today). Adding new patterns is a one-line change in the per-harness file.

### Stall confirmation (false-positive guard for harness retries)
17. Some third-party harnesses (Claude Code, Codex) print transient API errors and then automatically retry. A naive "first-hit wins" scanner would false-positive in that case and kill a recovering run. The runtime scanner therefore confirms quiescence before failing:
    - On a pattern hit, the scanner snapshots the harness block's visible plaintext.
    - Every 10 seconds for up to 60 seconds, it takes another snapshot and compares.
    - As long as the snapshots differ — i.e. the harness is producing new bytes, including spinner frames — the loop keeps running.
    - The first time two consecutive snapshots are byte-identical *and* the originating pattern is still present in the block, the detection is confirmed and the task is failed.
    - If the 60-second budget elapses without stabilization, the detection is dropped. The outer scan schedule resumes normal polling, and a later tick may re-detect.
    - If output stabilizes but the matched line has scrolled out of the visible window (i.e. the harness recovered far enough that the error is gone), the detection is also dropped.
18. The confirmation time counts toward the outer 90-second scan-schedule budget so a harness that keeps retripping the same pattern can't extend the watch window indefinitely.

### Error reporting
19. When the authentication preflight fails, the driver sends an `updateAgentTask` mutation to the server with:
    - `taskState: FAILED`
    - `statusMessage`: "Harness '{harness}' authentication check failed: login credentials are invalid or expired. Verify that the authentication secret configured for this harness is correct."
    - `errorCode: AUTHENTICATION_REQUIRED`
20. When the runtime scanner detects a failure, the driver sends an `updateAgentTask` mutation to the server with:
    - `taskState: FAILED`
    - `statusMessage`: "Harness '{harness}' could not make a successful API request. Matched failure pattern '{pattern}' in harness output: \"{excerpt}\". This usually means the API key is invalid, out of credits, or the account is misconfigured."
    - `errorCode: AUTHENTICATION_REQUIRED` (same code as the auth preflight failure; runtime failures are an extension of the same class of error)
21. After reporting the failure to the server, the driver exits the harness and terminates the process. No retry is attempted.

### Timeouts
22. The authentication preflight command has a timeout of 30 seconds. If it does not exit within the timeout, it is treated as a failure with a message indicating the check timed out.
23. The runtime scanner has a fixed 90-second observation window after the harness command starts (including any time spent in the stall-confirmation loop). It does not impose a timeout on the harness; it simply stops watching after the window elapses.

### Interaction with existing checks
24. Preflight checks run after `ThirdPartyHarness::validate()` (which checks CLI installation) and after `build_runner()` (which writes auth config files). This ordering ensures the CLI binary exists and credentials are on disk before we attempt to use them.
25. The authentication preflight runs before `HarnessRunner::start()`, which launches the main CLI command. The main command is never started if the authentication check fails.
26. The runtime scanner starts after `HarnessRunner::start()` returns the `BlockId` for the harness's main block. It races against the harness command, the periodic conversation save, and the idle-on-complete signal in the existing select loop.

### Idempotency and side effects
27. The authentication check commands (`claude auth status`, `codex login status`) are read-only status queries with no side effects.
28. The runtime scanner is purely observational — it reads the harness block's existing output. It never injects commands into the terminal and never modifies harness state. On a detected failure it issues the existing `/exit` slash command (the same call used by the idle-on-complete path).

### Edge cases
29. If the harness CLI is installed but the auth config file was not written successfully (e.g. disk full), the authentication check will fail with a non-zero exit code. This is the correct behavior — the run cannot proceed without valid auth.
30. If network connectivity is unavailable, the authentication check will fail with the generic auth-failure message; the stderr logs will contain the CLI's own diagnostic output for further debugging.
31. If the harness CLI version is too old to support the preflight command (e.g. `claude auth status` not available), the command will fail with a non-zero exit code. This is acceptable — the subsequent main command would also likely fail.
32. If the runtime scanner detects a failure but the harness exits successfully on its own before the `/exit` signal is processed, the detected failure still wins: the driver reports `HarnessRuntimeFailureDetected` rather than the harness's own (likely successful-looking) exit code. This avoids the confusing case where the harness reports success after the API call already failed.
