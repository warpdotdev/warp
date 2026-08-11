# Product Spec: Support 'agy' (Antigravity) CLI Agent in Warp

**Issue:** [warpdotdev/warp#11368](https://github.com/warpdotdev/warp/issues/11368)
**Figma:** none provided

## Summary

Add native support for the `agy` (Antigravity) CLI agent in Warp. This enables the terminal to automatically identify `agy` command executions and transition the active pane into "Agent Mode" (with dedicated layouts, toolbars, and branding).

## Problem

The Antigravity CLI agent (`agy`) is an autonomous developer tool. While users can run it in Warp today, Warp currently lacks native support for it as a recognized agent:
1. Running `agy` does not trigger the terminal's Agent Mode layout or toolbar.
2. The UI does not brand the session with the Antigravity logo or colors.

Note: Richer notifications (OSC 777) and plugin installation are deferred until an official Antigravity plugin for Warp exists.

## Goals

- Native command detection: typing or running `agy` triggers Agent Mode immediately.
- Custom branding: render the dedicated Antigravity toolbar with the same monochrome treatment as Pi: a white brand tile and a black custom logo.

## Non-Goals

- Providing inline terminal chips for plugin installation or updates (deferred).
- Processing structured OSC 777 notifications (deferred).
- Reading local `.antigravitycli/skills` directories (deferred).

## Success Criteria

1. Running `agy` in Warp launches Agent Mode and styles the pane with Antigravity branding.
2. The command is properly classified as a one-off shell command.

## Validation

- **Manual verification**: Verify the end-to-end command execution inside a live terminal window.
