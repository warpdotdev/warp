---
name: gui-onboarding-verification-skill
description: GUI desktop app only. Launch two parallel Oz cloud agents with computer use to download and install the latest stable Linux Warp build, capture screenshots while walking through first-time onboarding in both logged-out and logged-in states, then selectively fan out follow-up cloud agents for distinct onboarding branches proposed by those initial explorers. Use this whenever the user asks to test, document, screenshot, or walk through the Warp first-time install/onboarding experience in a cloud Linux environment.
---

# Onboarding verification skill

**Scope — GUI desktop app only.** This skill applies to Warp's **GUI** desktop front-end (the `app/` crate on the WarpUI pixel/GPU framework). It does **not** apply to the headless **TUI** front-end (`crates/warp_tui`; cell-grid `TuiElement` library under `crates/warpui_core/src/elements/tui`), which has its own components, tests, and change-verification workflow. For TUI work, see the `tui-ui-guidelines`, `tui-testing`, and `tui-verify-change` skills instead.

Use this skill to verify the first-time Warp install and onboarding flow on Linux with broader branch coverage than a single linear walkthrough.

The parent agent should not perform the walkthrough locally. Launch two parallel Oz cloud agents with computer use. Both initial children install the latest stable Warp Linux package appropriate for their platform and capture screenshots at every visible onboarding step until Warp reaches a usable terminal session. One child verifies the login-free flow. The other child verifies the logged-in flow using the managed secret `ONBOARDING_AGENT_FTUE_REFRESH_TOKEN`.

Those two baseline explorers are also responsible for noticing meaningful alternate onboarding branches and returning concrete plans for follow-up cloud agents. The parent agent should synthesize those plans, deduplicate overlapping suggestions, and launch a bounded second wave of targeted follow-up agents to improve coverage of paths a real user might encounter.

## Parent workflow

1. Launch exactly two remote Oz cloud agents in a single parallel `run_agents` batch with computer use enabled.
2. Use no environment-specific assumptions unless the user provided an environment. If no environment was provided, omit the environment ID and let Warp choose the default remote environment.
3. Give both baseline child agents the shared child prompt below, plus the appropriate flow-specific prompt.
4. Wait for both baseline agents' reports. Each report must include:
   - The completed baseline walkthrough result and artifacts.
   - A concise list of observed UI quality issues, suspected bugs, error states, or rough edges, with screenshots when visible.
   - A prioritized follow-up coverage plan describing distinct onboarding paths worth exploring with additional cloud agents.
5. Treat the authenticated baseline child as blocked if `ONBOARDING_AGENT_FTUE_REFRESH_TOKEN` is missing or does not authenticate successfully.
6. Build a combined coverage map from the two baseline reports. Deduplicate suggestions that reach the same visible state or exercise the same decision surface.
7. Launch a second `run_agents` batch with computer use enabled for the most valuable follow-up onboarding branches:
   - Prefer branches that materially change visible UI, available controls, downstream screens, auth state, or setup outcomes.
   - Favor paths likely to expose correctness, polish, layout, truncation, loading, or validation problems.
   - Default to at most four follow-up agents total unless the user explicitly asked for exhaustive coverage or the baseline reports show more than four clearly distinct high-value branches.
   - Do not launch speculative follow-ups when the baseline agents did not observe a concrete branch point; report that coverage stopped after the baseline pass instead.
8. Give each follow-up child the shared child prompt, the follow-up flow prompt below, the logged-out or logged-in flow prompt that matches its assigned auth state, and one synthesized branch assignment from the baseline reports.
9. Wait for all follow-up reports before summarizing coverage, issues, artifacts, and any still-unexplored branches worth a later run.

## Managed FTUE auth secret

- `ONBOARDING_AGENT_FTUE_REFRESH_TOKEN` is an internal-team managed secret for cloud agents, not a repo file or prompt literal.
- The secret should authenticate as a dedicated non-employee, non-`warp.dev` FTUE test user.
- Rotate the secret with `oz-dev secret update --team --value-file <private-token-file> ONBOARDING_AGENT_FTUE_REFRESH_TOKEN`.
- Treat the private token file as local scratch material only. Do not read it into chat, print it, stage it, commit it, upload it, or include it in artifacts. Delete it after the managed secret is updated.
- Children should receive the secret only through the managed environment variable injected into the remote run.

Use the initial `run_agents` call shaped like this:

```text
summary: Launching two baseline cloud agents with computer use to compare logged-out and logged-in Warp onboarding screenshots and propose follow-up coverage branches.
remote.computer_use_enabled: true
agent_run_configs:
- name: "warp-onboarding-logged-out"
  prompt: the logged-out flow prompt below
- name: "warp-onboarding-logged-in"
  prompt: the logged-in flow prompt below
base_prompt: the shared child prompt below
```

When the baseline reports identify concrete follow-up branches, use a second `run_agents` call shaped like this:

```text
summary: Launching targeted cloud follow-up agents to explore distinct onboarding branches identified by the baseline onboarding explorers.
remote.computer_use_enabled: true
agent_run_configs:
- name: "warp-onboarding-followup-theme-choice"
  prompt: the follow-up flow prompt below, the logged-out flow prompt below, and one synthesized logged-out branch assignment
- name: "warp-onboarding-followup-model-choice"
  prompt: the follow-up flow prompt below, the logged-in flow prompt below, and one synthesized logged-in branch assignment
base_prompt: the shared child prompt below
```

## Shared child prompt

Give both cloud agents these shared instructions:

```text
You are verifying the first-time Warp install and onboarding experience on Linux.

Goal:
- Download and install the latest stable Warp Linux build appropriate for this cloud environment's distro and CPU architecture.
- Launch Warp in a fresh first-run state.
- Take a screenshot at every visible onboarding step.
- Continue until Warp reaches a usable terminal session, or stop and report a blocker if the assigned flow cannot proceed.
- Notice alternate onboarding decisions that lead to meaningfully different screens, states, or outcomes, and return concrete follow-up cloud-agent plans for the parent orchestrator.
- Treat visual polish, missing assets, misalignment, overlapping content, clipped text, poor contrast, broken loading states, unexpected errors, and confusing controls as verification findings rather than ignoring them.

Install requirements:
- Use official stable Warp downloads only.
- Do not use Warp Preview, Alpha, source builds, or a repository development build.
- Detect CPU architecture with `uname -m`.
- Detect the package manager or distro before choosing the package format.
- Prefer native packages over AppImage because they install dependencies and register the app normally.

Stable Linux package mapping:
- Debian/Ubuntu with amd64 or x86_64: https://app.warp.dev/download?package=deb
- Debian/Ubuntu with arm64 or aarch64: https://app.warp.dev/download?package=deb_arm64
- Fedora/RHEL/CentOS/openSUSE with amd64 or x86_64: https://app.warp.dev/download?package=rpm
- Fedora/RHEL/CentOS/openSUSE with arm64 or aarch64: https://app.warp.dev/download?package=rpm_arm64
- Arch with amd64 or x86_64: https://app.warp.dev/download?package=pacman
- Arch with arm64 or aarch64: https://app.warp.dev/download?package=pacman_arm64
- If no native package path is available, use the AppImage fallback:
  - amd64 or x86_64: https://app.warp.dev/download?package=appimage
  - arm64 or aarch64: https://app.warp.dev/download?package=appimage_arm64

Before launch:
- Create a flow-specific artifact directory such as `~/warp-onboarding-logged-out` or `~/warp-onboarding-logged-in`.
- Ensure the run starts from a fresh Warp first-run state by removing only Warp-specific config/data/cache/state directories for the test user, such as `~/.config/warp-terminal`, `~/.local/share/warp-terminal`, `~/.local/state/warp-terminal`, and `~/.cache/warp-terminal` if they exist.
- Do not delete unrelated user files or system directories.

Screenshot workflow:
- Take the first screenshot before interacting with the first visible Warp window.
- Take one screenshot before every user action.
- Take another screenshot after each action if the UI changes.
- Use sequential filenames with a flow prefix, such as `01-logged-out-initial-window.png` or `01-logged-in-initial-window.png`.
- If anything looks wrong, take an additional issue-focused screenshot that captures the problematic state as clearly as possible.
- Maintain a manifest file in the artifact directory with, for each screenshot:
  - filename
  - timestamp
  - what was visible
  - what action was about to happen or just happened
- For issue-focused screenshots, add the suspected issue category and the screen or step where it appeared.
- Do not include secret values, refresh tokens, ID tokens, auth redirect URLs, or Authorization headers in the manifest, logs, shell history, screenshots, or final report.

Onboarding behavior:
- Baseline children choose the default or most conservative option at each step unless the flow-specific prompt says otherwise, while recording branch points that deserve separate follow-up coverage.
- Follow-up children take the specifically assigned alternate branch, then use the default or most conservative option for unrelated decisions unless the branch assignment says otherwise.
- If telemetry, shell, theme, editor-import, or agent integration choices appear, use the default path and document the choice in the manifest.
- Continue until a normal terminal prompt is visible and usable.

UI quality review:
- Watch for screens that are visually broken, obviously unfinished, misaligned, truncated, clipped, crowded, low-contrast, unexpectedly blank, stuck loading, or inconsistent with adjacent steps.
- Watch for actionable errors or validation states that appear during normal flow exploration, including auth failures, failed button transitions, controls that do not respond, duplicated overlays, missing images, or broken post-selection states.
- For every suspicious state:
  - Capture a screenshot.
  - Record the screen, the action that led to it, what looked wrong, and whether it blocked progress.
  - Describe the issue factually. If expected behavior is uncertain, say it appears suspicious rather than claiming a confirmed bug.

Terminal verification:
- Once a terminal session is visible, run a harmless flow-specific command:
  - logged-out flow: `echo warp-onboarding-logged-out-ready`
  - logged-in flow: `echo warp-onboarding-logged-in-ready`
- Capture a final screenshot showing the usable terminal and command output.

Report back:
- Whether you were a baseline explorer or a follow-up branch explorer.
- Which flow you ran: logged-out or logged-in.
- OS and distro detected.
- CPU architecture detected.
- Package URL and install method used.
- Launch command used.
- Whether the walkthrough reached a usable terminal session.
- Ordered screenshot list with short descriptions.
- Artifact directory path.
- Any built-in artifact IDs or attachment names if the harness supports artifact upload.
- Any visual polish concern, suspected bug, error state, or unpolished/misaligned screen, including:
  - screenshot filename
  - screen or step
  - action taken immediately before it appeared
  - concise observed behavior
  - whether it blocked progress
- Any blocker, crash, missing dependency, display problem, auth failure, or step that required judgment.
- For baseline explorers, include a `Follow-up coverage plan` section with zero or more proposed child-agent branches. Each proposal must include:
  - suggested agent name
  - logged-out or logged-in flow
  - onboarding screen or decision point where the alternate branch begins
  - exact alternate choice or action sequence to explore
  - why it is materially distinct from the baseline path
  - what user-visible state, setup outcome, or failure mode it could reveal
  - any secret, auth, or environment dependency
  - priority: high, medium, or low
- For follow-up explorers, include whether the assigned branch was reachable and completed. If a new branch point appears while following the assigned path, record it as a later-run suggestion instead of recursively expanding the run yourself.

Do not upload screenshots or logs to public external services. If the harness provides a built-in artifact or screenshot attachment mechanism, use that. Otherwise, leave the files in the artifact directory and report their paths.
```

## Logged-out flow prompt

Append this prompt to the shared child prompt for the logged-out child:

```text
You own the logged-out onboarding flow.

Flow-specific goal:
- Do not create an account, log in, or use a real user identity.
- Continue only through login-free or account-free paths until Warp reaches a usable terminal session.
- Stop and report a blocker if the flow requires login or account creation with no skip/continue-without-account option.

Flow-specific onboarding behavior:
- If there is a skip, "continue without account", "not now", "login later", or equivalent option, use it.
- Do not enter an email address, connect OAuth, paste an auth token, or create credentials.
- Be especially alert for logged-out branch points around choosing terminal-only versus agentic experiences, customization/layout options, third-party integration toggles, and terminal theme selection. If they appear, propose follow-up branches that exercise materially different choices rather than trying all alternates inline.
- Use the artifact directory `~/warp-onboarding-logged-out`.
```

## Logged-in flow prompt

Append this prompt to the shared child prompt for the logged-in child:

```text
You own the logged-in onboarding flow.

Flow-specific goal:
- Use the managed secret environment variable `ONBOARDING_AGENT_FTUE_REFRESH_TOKEN` to authenticate as the dedicated non-employee, non-`warp.dev` FTUE test user.
- Exercise onboarding screens that are available to an already-authenticated user.
- Continue through the authenticated onboarding path until Warp reaches a usable terminal session.

Secret handling requirements:
- Before doing auth work, verify that `ONBOARDING_AGENT_FTUE_REFRESH_TOKEN` exists and is non-empty without printing it.
- Never echo, log, screenshot, upload, or report the secret value.
- Avoid shell tracing (`set -x`) and avoid writing commands that place the raw token in shell history or process lists.
- Treat every auth redirect URL containing the refresh token as secret-bearing material, even after URL-encoding.
- Do not pass a token-bearing redirect URL to a shell command, desktop URI handler, browser address bar, process argument, log, artifact, or report. In particular, do not use commands such as `xdg-open`, `gio open`, `open`, or equivalent with the redirect URL.
- If you need to construct an auth redirect URL, keep it only in a clipboard value or a private temporary file with user-only permissions, paste it through Warp's visible Paste Auth Token flow, then delete the temporary file immediately after use.

Secure Paste Auth Token process:
1. Verify `ONBOARDING_AGENT_FTUE_REFRESH_TOKEN` exists and is non-empty without printing it.
2. Start Warp's normal login flow and derive the current-run `state` from Warp's generated login URL.
3. Normalize the managed secret privately:
   - Trim surrounding whitespace and one pair of surrounding single or double quotes if present.
   - If the secret parses as a URL with a `refresh_token` query parameter, extract that `refresh_token` value and ignore any stale `state` in the secret.
   - Otherwise, treat the trimmed secret as the raw refresh token.
4. URL-encode the extracted refresh token and current-run `state` separately as query parameter values.
5. Construct the redirect URL only in a clipboard value or private temporary file with user-only permissions.
6. Return to Warp and use the visible Paste Auth Token path:
   - Click the `Click here to paste your token from the browser` link, `Paste Auth Token` button, or equivalent pasted-token control shown by Warp.
   - Focus the auth token text input that appears.
   - Paste the prepared redirect URL into that input and submit it through Warp's UI so Warp parses and validates it.
7. Delete any private temporary files immediately after use and clear the clipboard if the environment supports doing so safely.
8. If the Paste Auth Token UI cannot be reached or automated safely, stop and report an auth blocker instead of parsing the redirect in place of Warp, using a desktop URI handler, browser address bar, or shell command with the token-bearing URL.

Preferred authenticated path:
- Launch Warp in a fresh first-run state and choose the login/sign-in path from onboarding.
- Use Warp's built-in Paste Auth Token flow rather than visiting real OAuth providers, invoking a desktop URI handler, or asking the agent to parse/validate the redirect URI itself.
- Derive `<state>` from the login URL generated by Warp if the UI exposes a copied login URL or opens the browser. If the UI does not expose the state after reasonable effort, report that as an auth blocker rather than bypassing state validation.
- Do not preflight the token with Firebase Secure Token before handing it to Warp. Warp's desktop redirect handler only requires `refresh_token` and `state`; `user_uid` is optional, and `deleted_anonymous_user=true` handles the anonymous-user override case.
- Treat `ONBOARDING_AGENT_FTUE_REFRESH_TOKEN` as either of these secret shapes:
  - a raw Firebase refresh token, or
  - a complete Warp desktop auth redirect URL containing a `refresh_token` query parameter.
- Normalize the secret into a current-run redirect URL without printing it:
  - Trim surrounding whitespace and one pair of surrounding single or double quotes if present.
  - If the secret parses as a URL with a `refresh_token` query parameter, extract that `refresh_token` value and ignore any stale `state` in the secret.
  - Otherwise, treat the trimmed secret as the raw refresh token.
  - URL-encode the extracted refresh token and the current-run `state` separately as query parameter values.
  - Build `warp://auth/desktop_redirect?refresh_token=<url-encoded-normalized-refresh-token>&deleted_anonymous_user=true&state=<url-encoded-current-state>`.
  - Do not include `user_uid` unless it is already present in a provided desktop redirect URL; it is not required for this flow.
- Construct the normalized redirect URL in a clipboard value or private temporary file only, then hand it to Warp through the Paste Auth Token UI. Do not parse, validate, or route the redirect outside of Warp.
- If the Paste Auth Token flow cannot be reached or automated safely, stop and report an auth blocker instead of using a desktop URI handler or any shell command that contains the token-bearing URL.

Fallback authenticated path:
- If Warp rejects the normalized redirect, report the non-sensitive user-visible error and classify whether the secret appeared to be a raw token or a desktop redirect URL, without reporting any token contents.
- If the Paste Auth Token flow is blocked by UI automation issues, report the blocker and include the exact non-sensitive step where automation failed.
- Do not switch to a logged-out path for this child.

Flow-specific onboarding behavior:
- Choose login/sign-in rather than skip/login-later when presented with an auth choice.
- After auth succeeds, continue through the remaining onboarding screens with default or conservative options.
- Be especially alert for logged-in branch points around model selection, account-aware onboarding screens, AI/agent setup, workspace or project setup, and any decision that changes available product capability. If they appear, propose follow-up branches that exercise materially different choices rather than trying all alternates inline.
- After the terminal verification succeeds, click the upper-right avatar/account control, open Settings from that menu, and capture an additional screenshot that clearly shows the logged-in user's email address in Warp settings or account/profile settings.
- Include the account/settings email screenshot in the manifest and final report. The email address itself may be visible in the screenshot, but do not copy the email into logs, shell output, or the final text report unless the user explicitly asks for it.
- Use the artifact directory `~/warp-onboarding-logged-in`.
```

## Follow-up flow prompt

Append this prompt to the shared child prompt for every second-wave child, followed by the matching logged-out or logged-in flow prompt and one branch assignment synthesized from the baseline reports:

```text
You own one follow-up onboarding branch selected by the parent orchestrator from an earlier baseline exploration report.

Follow-up branch behavior:
- Start from a fresh first-run Warp state and install the same latest stable Linux build using the shared instructions.
- Respect the assigned auth state: remain logged out for logged-out assignments, or use the managed authenticated flow for logged-in assignments.
- Follow the exact alternate onboarding choice or action sequence in the branch assignment.
- Capture screenshots before and after each assigned branch decision, then continue to a usable terminal session if the path allows it.
- Apply the same UI quality review standard as the baseline explorers and call out anything that looks broken, rough, misaligned, confusing, or unexpectedly error-prone.
- If the assigned branch is not reachable, capture the closest relevant screen, report why it was unreachable, and do not silently substitute a different branch.
- If the assigned branch reveals another interesting alternate path, record it as a later-run suggestion rather than recursively launching more agents yourself.

Final report additions:
- Repeat the exact branch assignment you attempted in concise non-sensitive terms.
- State whether it was reachable, completed, blocked, or not applicable.
- Compare the branch outcome against the likely baseline behavior when that comparison is visible from the UI.
```

## Success criteria

The run is successful when:

- Warp stable was installed from an official Linux package or AppImage for the detected architecture.
- Screenshots were captured for each onboarding screen and the final usable terminal.
- The logged-out child reached a usable terminal without login, account creation, or a real user identity.
- The logged-in child authenticated using `ONBOARDING_AGENT_FTUE_REFRESH_TOKEN` and reached a usable terminal in the authenticated FTUE path.
- The logged-in child captured an additional post-login screenshot from the avatar/settings flow showing the logged-in user's email address.
- Each terminal session was usable enough to run its flow-specific `echo` command.
- Both baseline explorers returned either concrete follow-up coverage proposals or an explicit explanation that they did not observe meaningful additional branch points.
- The parent orchestrator launched targeted second-wave agents for the highest-value concrete branch proposals, unless there were no such proposals or a prerequisite blocker made them infeasible.
- Every reported visual polish concern, suspected bug, or error state includes a screenshot reference whenever the issue was visible on screen.

## Common failure handling

- If the package manager prompts for confirmation, use the non-interactive confirmation flag supported by that package manager.
- If launching `warp-terminal` fails because of display setup, inspect the cloud environment's display variables and try launching from the desktop/app launcher if computer use provides one.
- If the logged-out flow blocks on login with no skip path, stop at that screen, capture a screenshot, and report that as the terminal point for the logged-out flow.
- If the logged-in flow cannot authenticate because the secret is missing, invalid, expired, revoked, or cannot be routed through Warp's auth redirect flow, stop at that screen, capture a screenshot, and report the non-sensitive blocker.
- If the native package cannot be installed because dependencies are unavailable, fall back to the matching AppImage and clearly report the fallback.
