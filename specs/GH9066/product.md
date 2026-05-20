# PRODUCT.md — Support Kiro CLI Agent Integration in Warp

Issue: https://github.com/warpdotdev/warp/issues/9066

## Summary

Add Kiro CLI (`kiro`) as a first-class supported CLI agent in Warp, alongside
Claude Code, Codex, Gemini CLI, and the other agents already integrated. When a
user runs `kiro` in a Warp terminal, Warp detects the session, displays the
Kiro-branded agent footer, enables the rich input composer (Ctrl-G), and tracks
session status (in-progress, blocked, success) using the same plugin event
protocol used by other agents.

Figma: none provided.

## Goals / Non-goals

In-scope:

- Detecting `kiro` as a CLI agent session in the terminal.
- Displaying the Kiro-branded footer and toolbar chip while a `kiro` session is
  active.
- Enabling the rich input composer (Ctrl-G / footer button) for composing
  prompts to send to Kiro.
- Showing plugin install/update instructions when the Warp plugin is not
  installed or is out of date.
- Tracking session status (InProgress, Blocked, Success) via the Warp plugin
  event protocol.
- Surfacing Kiro in the "Third party CLI agents" settings page.
- Telemetry for Kiro sessions consistent with other agents.
- macOS support (primary). Linux support follows the same code path and is
  included. Windows support is not in scope for this spec.

Out of scope:

- Changes to the Warp built-in agent, execution profiles, or onboarding.
- A new `SkillProvider::Kiro` variant — Kiro CLI supports the existing
  `SkillProvider::Agents` skill format; no Kiro-specific skill provider is
  needed at this time.
- Custom toolbar commands or user-configured regex patterns for Kiro.
- Remote (SSH) session support beyond what the existing plugin infrastructure
  already provides generically.

## Behavior

1. When a user runs `kiro` (or a command whose first token is `kiro`) in a Warp
   terminal pane, Warp detects an active Kiro CLI session and begins tracking it
   in `CLIAgentSessionsModel`.

2. While a Kiro session is active, the Kiro-branded agent footer is displayed at
   the bottom of the terminal pane. The footer shows the Kiro logo, the session
   status, and the rich input button. Layout, interaction model, and dismiss
   behavior are identical to the Claude Code and Gemini footers.

3. Pressing Ctrl-G (or clicking the rich input button in the footer) opens the
   rich input composer. The composer accepts free-form text and slash-command
   skills from `SkillProvider::Agents`. The skill command prefix is `/`.

4. Submitting a prompt from the rich input composer sends it to the running
   `kiro` process via the terminal PTY, exactly as it does for other agents.

5. When the Warp plugin for Kiro is installed and active, session status updates
   (InProgress, Blocked, Success) are reflected in the footer in real time.
   When the plugin is not installed, the footer still appears (via command
   detection) but status tracking is unavailable.

6. When the Warp plugin is not installed, Warp displays plugin install
   instructions in the footer's plugin pane. The instructions guide the user
   through installing the plugin so that status tracking becomes available.

7. _(Deferred)_ When the installed plugin version is below the minimum required
   version, Warp displays plugin update instructions in the same pane. This
   behavior is not delivered in the initial release: no versioned Kiro Warp
   plugin exists yet, so version detection (`is_installed()`, `needs_update()`)
   is not implemented and `supports_update()` returns `false`. The update pane
   will never appear. This requirement is carried as a follow-up for when a
   versioned Kiro plugin is published.

8. The Kiro agent entry appears in the "Third party CLI agents" settings page
   under Settings → Agents → Third party CLI agents, alongside Claude Code,
   Codex, Gemini, and the other supported agents, when
   `FeatureFlag::KiroCLIAgent` is enabled. The entry shows the Kiro logo and
   name. When the flag is disabled, Kiro is hidden from the selector, and any
   persisted command mapping to Kiro is displayed as "Other" in the settings UI
   (without exposing Kiro as a selectable option) until the flag is re-enabled.
   A documentation link is not included in the initial implementation because
   the current settings UI does not render per-agent documentation links for any
   agent; adding that capability is a follow-up item.

9. Session start, status changes, and session end are reported to telemetry
   using the same `CLIAgentType` telemetry event structure used by other agents,
   with `CLIAgentType::Kiro` as the agent discriminant.

10. In a remote SSH terminal pane, the Kiro footer and rich input still appear
    when a `kiro` command is detected (via command detection, same as local).
    The plugin install and update instructions pane also appears in remote
    sessions, consistent with the existing generic plugin-chip behavior for all
    other CLI agents. If the Warp plugin is already running on the remote system
    and emitting events, real-time status tracking works normally. If it is not,
    the footer shows the install instructions pane (same as a local session with
    no plugin installed).

11. Kiro sessions are included in shared session state sync (the `cli_agent`
    field in the shared session protocol) using the serialized name `"Kiro"`.

12. The `kiro` command is not treated as a custom/unknown agent — it maps
    directly to `CLIAgent::Kiro` and receives the full first-class treatment
    (branded footer, logo, install instructions) rather than the generic
    "CLI Agent" fallback.

13. No existing agent's behavior, settings, or telemetry is changed by this
    addition.
