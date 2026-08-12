# Recognize Snowflake CoCo CLI as a CLI agent — Product Spec

GitHub issue: https://github.com/warpdotdev/warp/issues/14319

Figma: none provided

## Summary

Recognize a command whose executable token is `cortex`, Snowflake's documented CoCo CLI launch command, as a built-in third-party CLI agent in Warp. While that interactive process is running, Warp identifies it as **CoCo CLI** and uses a maintainer-approved CoCo graphical identity instead of the generic terminal identity on the GUI surfaces that already identify local CLI-agent sessions.

## Problem

Warp currently treats an interactive `cortex` process as an ordinary terminal command. Users therefore see the generic terminal identity rather than the identity of the CLI agent they are using, which makes a CoCo session less distinguishable from adjacent terminal tabs and panes.

Some official Snowflake integrations still use the earlier **Snowflake Cortex Code** or **Cortex Code CLI** names. This spec follows the current Snowflake documentation for the visible **CoCo CLI** label while retaining `cortex` as the documented executable name.

## Goals

- Recognize the documented `cortex` command through Warp's existing local CLI-agent detection lifecycle.
- Present the current official product name, **CoCo CLI**, consistently wherever Warp already presents a detected local CLI agent.
- Use only an official, approved graphical asset and color treatment for the CoCo identity.
- Preserve the behavior of CoCo itself and of every existing Warp CLI-agent integration.

## Non-goals

- Adding custom user-defined CLI-agent registries, executable mappings, display names, icons, or colors. That broader capability is tracked separately in issue #13519.
- Installing, launching, updating, authenticating to, or configuring CoCo CLI.
- Adding a Snowflake cloud harness or making CoCo available as a Warp-hosted agent.
- Adding a CoCo plugin, structured status listener, notification integration, managed prompt composer, or other CoCo-specific protocol integration. Existing generic CLI-agent actions may continue to write user-selected context to the active process.
- Discovering, indexing, exporting, or invoking CoCo skills, including content under CoCo's documented `.cortex/skills` and `.claude/skills` locations.
- Changing CoCo's input handling, output, shell mode, command-line options, account behavior, or network behavior.
- Adding this GUI identity treatment to Warp's headless TUI.

## Behavior

1. When the executable token or resolved executable basename is exactly `cortex` and the process remains active as an interactive terminal application, Warp recognizes that local session as **CoCo CLI**. Recognition identifies the command name; it does not authenticate the binary's publisher.

2. Warp still recognizes the session when `cortex` is started with arguments or an official invocation mode, including commands shaped like `cortex -c ...`, `cortex --continue`, `cortex --resume`, and `cortex --plan`.

3. Warp applies the same executable resolution rules it applies to other built-in CLI agents. An absolute or relative path whose executable basename is `cortex`, or a shell alias that resolves to that executable, is eligible for recognition.

4. Recognition is exact. Executable names such as `coco`, `cortex-code`, `cortex2`, and `my-cortex` are not recognized as CoCo CLI. Text containing the word `cortex` is not sufficient when `cortex` is not the command being executed.

5. A bare `cortex` command continues through Warp's shell-command path rather than being interpreted as a natural-language request.

6. For a recognized session, every existing GUI surface that presents a local CLI-agent identity uses the visible name **CoCo CLI** and the same approved CoCo graphical identity. The session does not use Warp's generic terminal icon or the identity of another agent.

7. The CoCo identity remains legible in the supported light and dark themes and at every size where existing CLI-agent icons appear. Warp uses the source, geometry, and color treatment accepted by the maintainers for this integration.

8. Warp presents a CoCo-branded icon and color only after maintainers select an acceptable asset source and treatment. The feature is not released with a contributor-invented or automatically traced logo, or with a generic icon presented as CoCo branding.

9. Recognition follows the existing long-running-command lifecycle. When the interactive `cortex` process exits, the CoCo identity is removed and the terminal returns to its ordinary identity; the next unrelated command does not inherit the CoCo identity.

10. Recognition itself is local and works without a new Warp or Snowflake network request. Any authentication or network activity performed by CoCo remains CoCo's existing behavior.

11. Recognizing CoCo does not add Warp-managed rich input, plugin controls, structured activity or completion status, or agent-specific notifications. Existing user-initiated generic CLI-agent actions, such as sending selected code or review context to the active process, continue to write plain text directly to the PTY; Warp does not submit that text or interpret a response through a CoCo-specific protocol.

12. Recognizing CoCo does not cause Warp to search, ingest, expose, or modify CoCo skill files or user configuration.

13. Existing CLI agents continue to be recognized and presented exactly as before, including their names, executable matching, icons, colors, input behavior, and lifecycle behavior.

14. When Warp encounters a command or agent identity it does not understand, it continues to use the ordinary unknown-agent fallback rather than labeling it as CoCo CLI or failing.

15. The behavior is available on supported Warp GUI platforms where local CLI-agent process detection and the relevant identity surfaces are available. It does not change Warp's headless TUI.
