---
name: reproduce-bug-report
description: Launch Oz cloud agents with computer use to reproduce UI-focused bug reports, capture screenshots, and report reproduction evidence. Use when invoked from a GitHub issue, bug report, or prompt describing a specific visual or interactive bug.
---

# Reproduce bug report

Use this skill when the current context is a GitHub issue, support report, Linear ticket, or user prompt describing a specific bug that may be reproducible in the Warp app while logged out. This skill is primarily for bugs with a UI, rendering, windowing, onboarding, settings, editor, terminal-display, or other visual/interactive component where screenshots or recordings would make the result more actionable.

The parent agent should not try to manually reproduce the UI bug locally unless the user explicitly asks. Launch one or more Oz cloud agents with computer use enabled so they can run the app, interact with it, and capture visual evidence.

For now, use this skill only for non-logged-in reproduction attempts. If a report requires authentication, account state, cloud sync, logged-in onboarding, or AI behavior that cannot be exercised while logged out, do not launch a repro agent for it with this skill; report that it is out of scope for the current logged-out workflow.

## Parent workflow

1. Read the bug report carefully and extract:
   - reported behavior
   - expected behavior
   - reproduction steps, if provided
   - OS, exact Warp version/build and channel if provided, shell, feature flags, account state, or other environment constraints
   - attached screenshots, videos, logs, or comments that narrow the repro path
2. Decide whether this skill applies:
   - Use it for UI-visible bugs, interaction bugs, rendering/layout bugs, login-free onboarding bugs, and bugs where screenshot evidence would be useful.
   - Use it only when the relevant state can be reached while Warp remains logged out.
   - Do not use it for authenticated-user flows, account-specific state, cloud-synced state, or AI behaviors that require login.
   - Do not use it for purely backend, CI, build, dependency, or text-only code issues unless the prompt specifically asks for visual reproduction.
3. If the reproduction path is straightforward, launch one Oz cloud agent with computer use.
4. If there are multiple plausible repro paths, launch several Oz cloud agents in one `run_agents` batch. Give each child a distinct hypothesis or environment variant, such as:
   - different OS or desktop environment
   - fresh first-run logged-out state vs completed logged-out onboarding
   - stable vs dev build
   - fresh settings vs existing settings
   - different shells, prompts, pane layouts, or settings toggles
5. If steps are incomplete, use codebase knowledge to propose likely app states and assign children to investigate those states. Do not invent facts about the original reporter's environment.
6. Wait for all children to report before summarizing. Distinguish confirmed reproduction, partial reproduction, non-reproduction, blockers, and untested hypotheses.

## Version-matched app setup

- Prefer reproducing against the exact Warp version/build and channel reported by the user.
- Do not build Warp from source by default. Install the matching Linux package or binary release for the reporter's version/channel instead.
- If the bug report names a macOS or Windows build, use the corresponding Linux build from the same version/channel when a matching Linux artifact exists, and state that this is a Linux proxy for the reporter's platform.
- Use the repository's or Warp release tooling/docs available in the environment to find and install the exact versioned Linux artifact. Do not silently substitute the latest stable build when an exact matching version can be installed.
- If the exact version/build cannot be found or installed, report that clearly, explain what was attempted, and use the closest justified fallback only when it is useful for continuing the investigation.
- Record the requested reporter version, the installed Linux version, the source of the installed artifact, and any fallback decision in the manifest and final report.

Use a `run_agents` call shaped like this:

```text
summary: Launching Oz cloud computer-use agents to reproduce the reported UI bug and collect screenshots.
remote.computer_use_enabled: true
agent_run_configs:
- name: "repro-primary"
  prompt: the primary repro prompt
- name: "repro-variant-logged-out"
  prompt: optional variant prompt when useful
base_prompt: the shared child prompt below
```

Omit extra children when they would duplicate the same steps. Omit `model_id` unless the user requested a specific model.

## Shared child prompt

Give every child agent these shared instructions, then append the child-specific repro path or hypothesis.

```text
You are trying to reproduce a reported Warp UI bug using Oz cloud computer use.

Goal:
- Reproduce the reported behavior as faithfully as possible.
- Capture screenshots before and after each meaningful interaction.
- If the provided steps are unclear or incomplete, use codebase and product knowledge to identify plausible app states that could produce the reported behavior, then test the assigned hypothesis.
- Report clear reproduction evidence, not just opinions.

Inputs:
- Bug report context: <paste or summarize the issue body, comments, screenshots/video descriptions, labels, and relevant metadata>
- Assigned repro path or hypothesis: <specific steps, environment, logged-out app state, settings, feature flags, or code path to test>
- Reporter Warp version/build/channel: <exact value from the report, or unknown>
- Build/app target: <exact versioned Linux package/binary to install, or the justified fallback if exact artifact is unavailable>

Safety and privacy:
- Do not ask the public reporter for credentials, tokens, private repos, private workspace names, or private account identifiers.
- Do not include secrets, auth tokens, private URLs, Authorization headers, or refresh tokens in screenshots, logs, manifests, or final reports.
- Keep Warp logged out. Do not create an account, sign in, paste auth tokens, or use real user credentials.
- If the assigned report cannot be exercised without authentication, stop and report that the current logged-out workflow is blocked for this issue.
- Do not post comments to GitHub, Linear, Slack, or external services unless explicitly instructed.
- Avoid destructive actions. If a repro requires deleting app state, delete only Warp-specific test state for the current test user and report exactly what was reset.

Artifact workflow:
- Create a dedicated artifact directory named for your variant, such as `~/warp-bug-repro-primary`.
- Save screenshots with ordered filenames, such as `01-initial-state.png`, `02-before-click-settings.png`, and `03-after-click-settings.png`.
- Maintain a short manifest in the artifact directory with:
  - screenshot filename
  - timestamp
  - visible app state
  - action just taken or about to be taken
  - whether the screenshot shows the reported bug
- If the harness supports built-in screenshot or artifact upload, use it. Otherwise leave artifacts in the directory and report the paths.

Reproduction workflow:
1. Confirm the environment you are testing: OS, architecture, display/session type, shell if relevant, and app/build/version if visible.
2. Identify the exact Warp version/build/channel from the report when available, then install the corresponding Linux package or binary release instead of building from source.
3. If no exact reporter version is available, record that the version is unknown and choose the most defensible install target for the report; state the fallback explicitly.
4. Start from the cleanest state that matches the report. Do not reset user state if the bug depends on existing settings or persisted logged-out state.
5. Launch Warp and complete the login-free / continue-without-account onboarding path until a normal logged-out terminal session is usable.
6. Capture the post-onboarding baseline screenshot before attempting the bug-specific reproduction.
7. Follow the exact provided bug reproduction steps first, when available.
8. If exact steps do not reproduce, test the assigned hypothesis and document where it diverges from the report.
9. If the bug appears, stop changing variables and capture enough evidence to make the reproduction actionable.
10. If the bug does not appear, make at most two targeted variations that are directly supported by the report or code-path hypothesis.
11. If the app crashes, hangs, or blocks progress, capture a screenshot and collect non-sensitive logs or terminal output that explain the blocker.

Code-path investigation for unclear steps:
- Search the codebase for UI strings, labels, feature names, settings keys, telemetry names, route names, and components mentioned in the report.
- Identify the likely component, model, feature flag, or state transition that could produce the reported behavior.
- Use that investigation to choose targeted UI actions rather than broad exploratory clicking.
- Report the files or symbols that informed your hypothesis, but keep the final report focused on reproduction evidence.

Report back:
- A brief bug summary before the verdict, including the issue/report identifier if available, the reported behavior, and the expected behavior.
- Reproduction status: confirmed, partially confirmed, not reproduced, or blocked.
- The exact steps you performed.
- Environment and app/build information.
- Reporter-requested Warp version/build/channel, installed Linux version/build/channel, and the artifact source or fallback explanation.
- Whether the observed behavior matched the report, and how closely.
- Screenshot list with short descriptions and artifact paths or attachment names.
- Any logs, crash output, or diagnostics collected, with secrets redacted.
- The most likely code path or state involved, if investigated.
- Suggested next debugging step or follow-up question, only if it would materially change the next action.
```

## Child prompt patterns

### Primary repro child

Use this for a report with clear steps:

```text
You own the primary reproduction attempt.

Follow the bug report's steps exactly before trying variants. Prioritize matching the reporter's OS, app channel, logged-out app state, settings, shell, and layout. If those details are missing, choose the most common path and explicitly list assumptions.
```

### Variant child

Use this when there is a specific alternate condition worth testing:

```text
You own this reproduction variant: <variant name>.

Test only this variant's assigned environment or state. Do not duplicate the primary child's full search space. Report whether this variant changes the outcome and include screenshots for any difference.
```

### Code-path hypothesis child

Use this when repro steps are missing or ambiguous:

```text
You own code-path-guided reproduction.

Start by tracing likely code paths from strings, UI labels, settings names, feature names, or screenshots in the report. Then choose a targeted UI path that should exercise the suspected state. Report the code paths you used to form the hypothesis and the visual result of testing it.
```

## Success criteria

A successful use of this skill produces:

- A confirmed reproduction with screenshots and exact steps, or a well-scoped non-reproduction with tested assumptions.
- Clear artifact paths or attachments for visual evidence.
- A concise summary of which variants were tested and which were not.
- Enough environment detail for an engineer to repeat the test.
- No leaked secrets, credentials, private account details, or unnecessary public comments.

## Summary format

When the children finish, summarize in this structure:

```text
Bug summary:
- Issue/report: <identifier or source>
- Reported behavior: <what bug the child attempted to reproduce>
- Expected behavior: <what should have happened instead>
Reproduction status: <confirmed | partially confirmed | not reproduced | blocked>

What was tested:
- <variant/child>: <steps and environment>

Evidence:
- <screenshot/artifact path>: <what it shows>

Findings:
- <observed behavior vs reported behavior>
- <likely state/code path, if known>

Next step:
- <one concrete debugging action or follow-up question>
```
