---
name: onboarding-verification-skill
description: Launch two parallel Oz cloud agents with computer use to download and install the latest stable Linux Warp build, then capture screenshots while walking through first-time onboarding in both logged-out and logged-in states. Use this whenever the user asks to test, document, screenshot, or walk through the Warp first-time install/onboarding experience in a cloud Linux environment.
---

# Onboarding verification skill

Use this skill to verify the first-time Warp install and onboarding flow on Linux.

The parent agent should not perform the walkthrough locally. Launch two parallel Oz cloud agents with computer use. Both children install the latest stable Warp Linux package appropriate for their platform and capture screenshots at every visible onboarding step until Warp reaches a usable terminal session. One child verifies the login-free flow. The other child verifies the logged-in flow using the managed secret `ONBOARDING_AGENT_FTUE_REFRESH_TOKEN`.

## Parent workflow

1. Launch exactly two remote Oz cloud agents in a single parallel `run_agents` batch with computer use enabled.
2. Use no environment-specific assumptions unless the user provided an environment. If no environment was provided, omit the environment ID and let Warp choose the default remote environment.
3. Give both child agents the shared child prompt below, plus the appropriate flow-specific prompt. Attach or explicitly reference `.agents/skills/warp-computer-use-login/SKILL.md` for the logged-in child.
4. Wait for both child agents' reports before summarizing results.
5. Treat the authenticated child as blocked if `ONBOARDING_AGENT_FTUE_REFRESH_TOKEN` is missing or does not authenticate successfully.

## Logged-in auth dependency

The logged-in child uses `.agents/skills/warp-computer-use-login/SKILL.md` to authenticate with Warp through Oz cloud computer use.

- Use the default managed secret `ONBOARDING_AGENT_FTUE_REFRESH_TOKEN`.
- Treat the logged-in child as blocked if the managed secret is missing, invalid, expired, revoked, or cannot be routed through Warp's Paste Auth Token flow.
- Children should receive the secret only through the managed environment variable injected into the remote run.

Use a `run_agents` call shaped like this:

```text
summary: Launching two cloud agents with computer use to compare logged-out and logged-in Warp onboarding screenshots.
remote.computer_use_enabled: true
skills:
- spec: ".agents/skills/warp-computer-use-login/SKILL.md"
agent_run_configs:
- name: "warp-onboarding-logged-out"
  prompt: the logged-out flow prompt below
- name: "warp-onboarding-logged-in"
  prompt: the logged-in flow prompt below
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
- Maintain a manifest file in the artifact directory with, for each screenshot:
  - filename
  - timestamp
  - what was visible
  - what action was about to happen or just happened
- Do not include secret values, refresh tokens, ID tokens, auth redirect URLs, or Authorization headers in the manifest, logs, shell history, screenshots, or final report.

Onboarding behavior:
- Choose the default or most conservative option at each step unless the flow-specific prompt says otherwise.
- If telemetry, shell, theme, editor-import, or agent integration choices appear, use the default path and document the choice in the manifest.
- Continue until a normal terminal prompt is visible and usable.

Terminal verification:
- Once a terminal session is visible, run a harmless flow-specific command:
  - logged-out flow: `echo warp-onboarding-logged-out-ready`
  - logged-in flow: `echo warp-onboarding-logged-in-ready`
- Capture a final screenshot showing the usable terminal and command output.

Report back:
- Which flow you ran: logged-out or logged-in.
- OS and distro detected.
- CPU architecture detected.
- Package URL and install method used.
- Launch command used.
- Whether the walkthrough reached a usable terminal session.
- Ordered screenshot list with short descriptions.
- Artifact directory path.
- Any built-in artifact IDs or attachment names if the harness supports artifact upload.
- Any blocker, crash, missing dependency, display problem, auth failure, or step that required judgment.

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
- Follow `.agents/skills/warp-computer-use-login/SKILL.md` exactly.
- Use the default managed secret environment variable `ONBOARDING_AGENT_FTUE_REFRESH_TOKEN`.
- Choose login/sign-in rather than skip/login-later when presented with an auth choice.
- Do not switch to a logged-out path for this child.

Flow-specific onboarding behavior:
- After auth succeeds, continue through the remaining onboarding screens with default or conservative options.
- After the terminal verification succeeds, click the upper-right avatar/account control, open Settings from that menu, and capture an additional screenshot that clearly shows the logged-in user's email address in Warp settings or account/profile settings.
- Include the account/settings email screenshot in the manifest and final report. The email address itself may be visible in the screenshot, but do not copy the email into logs, shell output, or the final text report unless the user explicitly asks for it.
- Use the artifact directory `~/warp-onboarding-logged-in`.
```

## Success criteria

The walkthrough is successful when both child agents report:

- Warp stable was installed from an official Linux package or AppImage for the detected architecture.
- Screenshots were captured for each onboarding screen and the final usable terminal.
- The logged-out child reached a usable terminal without login, account creation, or a real user identity.
- The logged-in child authenticated using `ONBOARDING_AGENT_FTUE_REFRESH_TOKEN` and reached a usable terminal in the authenticated FTUE path.
- The logged-in child captured an additional post-login screenshot from the avatar/settings flow showing the logged-in user's email address.
- Each terminal session was usable enough to run its flow-specific `echo` command.

## Common failure handling

- If the package manager prompts for confirmation, use the non-interactive confirmation flag supported by that package manager.
- If launching `warp-terminal` fails because of display setup, inspect the cloud environment's display variables and try launching from the desktop/app launcher if computer use provides one.
- If the logged-out flow blocks on login with no skip path, stop at that screen, capture a screenshot, and report that as the terminal point for the logged-out flow.
- If the logged-in flow cannot authenticate because the secret is missing, invalid, expired, revoked, or cannot be routed through Warp's auth redirect flow, stop at that screen, capture a screenshot, and report the non-sensitive blocker.
- If the native package cannot be installed because dependencies are unavailable, fall back to the matching AppImage and clearly report the fallback.
