# Product Spec: Support Kiro CLI agent integration

**Issue:** [warpdotdev/warp#9066](https://github.com/warpdotdev/warp/issues/9066)

**Prior spec:** [warpdotdev/warp#12387](https://github.com/warpdotdev/warp/pull/12387)

**Figma:** none provided

## Summary

Warp recognizes Kiro CLI as a first-class CLI agent. An active Kiro session uses
Kiro branding in the CLI-agent footer, Rich Input, vertical tabs, settings, and
telemetry.

Warp also shows Kiro skills from `.kiro/skills` in the Rich Input slash menu. This
initial integration does not install a plugin or claim real-time Kiro status
support.

## Problem

Warp currently treats Kiro CLI commands as unknown terminal commands. Users do not
get the branded footer, Rich Input workflow, Kiro skill filtering, or Kiro
telemetry that supported CLI agents receive.

## Goals

- Detect the supported Kiro CLI executable and launcher names.
- Show consistent Kiro branding across CLI-agent surfaces.
- Enable Rich Input and Kiro skills for an active Kiro session.
- Preserve Kiro identity in telemetry and shared sessions.
- Add automated coverage for the user-visible flow.

## Non-goals

- Installing or updating Kiro CLI.
- Installing a Warp plugin into Kiro CLI.
- Adding Kiro-specific session event or status tracking.
- Adding Kiro support for Rich Input bash mode.
- Changing Kiro CLI behavior or its skill-file format.
- Changing the behavior of another CLI agent.

## Behavior invariants

1. Warp recognizes `kiro-cli`, `kiro`, `kiro-cli-chat`, and `kiro-cli-term` as
   Kiro CLI executable names.

2. Detection uses the existing CLI-agent command rules. It supports executable
   paths, arguments, shell aliases, and leading environment assignments where the
   existing parser supports them.

3. A recognized Kiro command maps to the first-class Kiro agent identity. It does
   not use the generic unknown-agent identity.

4. An active Kiro session shows the Kiro name, logo, and brand colors in the
   CLI-agent footer and other agent-identity surfaces.

5. The vertical-tabs panel uses the Kiro identity for a pane that has an active
   Kiro session.

6. A user can open Rich Input for an active Kiro session. The editor shows
   `Enter prompt for Kiro...` when the input is empty.

7. The Rich Input slash menu shows skills from the Kiro skill provider. This
   provider reads the existing home and project `.kiro/skills` directories.

8. Rich Input sends prompt text to the active Kiro process through the terminal
   PTY. It sends Enter after the existing short delay used by compatible agents.

9. Kiro does not enable Rich Input bash mode. A leading `!` does not receive the
   Kiro-specific shell-mode treatment.

10. CLI-agent telemetry identifies Kiro with a distinct Kiro value. Shared-session
    serialization preserves the Kiro identity with the name `Kiro`.

11. Kiro is available in settings surfaces that list known CLI agents, including
    the command-to-agent mapping selector.

12. Warp does not show Kiro-specific plugin installation or update instructions.
    Warp does not claim Kiro status updates until a compatible event integration
    exists.

13. Existing CLI-agent detection, branding, Rich Input, settings, telemetry, and
    shared-session behavior remain unchanged.

## Open questions

- Does the initial release require a feature flag for staged rollout?
- Are all four executable names part of the supported Kiro CLI distribution?
