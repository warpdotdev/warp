# PRODUCT.md — Support warpctrl (local control) on Windows

Issue: https://github.com/warpdotdev/warp/issues/16158

## Summary

`warpctrl` lets scripts and tools control a running Warp app (windows, tabs, panes, input staging, surfaces, settings). It works on macOS and Linux, but on Windows local-control publication is disabled, so `warpctrl` never finds a running instance. This change enables it on Windows with the same user-facing behavior and the same trust boundary as the Unix implementation: only processes running as the same OS user can obtain credentials.

Figma: none (no UI changes).

## Goals / Non-goals

Goals:

- `warpctrl` works on Windows with the same catalog, selectors, output, and errors as on macOS and Linux.
- The Windows prerequisites listed in `specs/warp-control-cli/SECURITY.md` are met: discovery-record ACL enforcement and an authenticated broker transport. Protected Scripting storage already uses DPAPI.

Non-goals:

- Terminal command execution or input submission. The catalog is unchanged, and `input.insert` / `input.replace` still only stage text.
- Installing a `warpctrl` command on `PATH` on Windows (packaging follow-up). Until then, invoke the hidden control mode: `warp.exe --warpctrl …`.
- Remote control, and distinguishing trusted Warp code from other software running as the same user. Both remain out of scope, as on Unix.

## Behavior

1. With **Settings > Scripting** set to Enabled, a running Warp instance on Windows publishes a discovery record in `%LOCALAPPDATA%\warp\local-control`, and `warpctrl instance list` lists it.
2. With Scripting set to Disabled (the default on public channels), no usable endpoint or broker is published, and `warpctrl` reports that no instances were found. Changing the setting while Warp runs publishes or withdraws the endpoint without a restart.
3. Every allowlisted action behaves the same as on macOS and Linux: same names, parameters, selectors, JSON output, error codes, and exact-action credential scoping.
4. The discovery directory and every record have a protected DACL (not inherited from the parent) that grants access only to the current user, `SYSTEM`, and `Administrators`, and is owned by one of them.
5. `warpctrl` ignores a record whose ACL is inherited, grants any other account, or whose owner is another account. It deletes that record the same way it deletes malformed records. If the directory itself fails this check, no instances are listed.
6. Each instance's credential broker is a named pipe derived from its instance ID. Only the current OS user can open it, and connections from other machines are rejected.
7. A process running as a different OS user cannot obtain a credential. The broker answers `unauthorized_local_client` before reading the request.
8. Before sending a request, `warpctrl` checks that the broker pipe is served by the process ID in the discovery record and that this process runs as the current user. If either check fails, it stops with `unauthorized_local_client` and sends nothing.
9. The broker can identify `warpctrl` but cannot impersonate it.
10. The broker rejects a credential request that is larger than 64 KiB, or that ends before its terminator, with `invalid_request`.
11. If every broker pipe instance is busy, `warpctrl` retries for up to about one second before reporting `transport_unavailable`.
12. If another process already owns an instance's broker pipe name, that Warp instance does not publish local control; it does not share the pipe.
13. macOS and Linux behavior is unchanged. Other platforms without an owner-authenticated broker still fail closed.

## Open questions

- Should a Windows `warpctrl.cmd` wrapper be installed next to `warp.exe` in the same release, or in a separate packaging change as the README currently plans?
