# Product Spec: Start Warp for `warpctrl file open`

**Issue:** [warpdotdev/warp#13630](https://github.com/warpdotdev/warp/issues/13630)
**Figma:** none provided

## Summary

`warpctrl file open <path>` should start the matching Warp desktop app when no controllable same-channel instance is running, wait for that app to publish its local-control endpoint, and then perform the existing `file.open` action. This gives the existing file command a reliable cold-start path without committing to a new top-level `warp <path>` command while Warp's CLI surface is being reorganized.

## Problem

`warpctrl file open` can currently open a file only after a compatible Warp process is already running with Scripting enabled. When no process is discoverable, instance selection immediately returns `no_instance`. Callers must separately know how to launch the correct Stable, Preview, Dev, or Local app, wait for it to become ready, and retry the command.

That startup gap makes the existing file command unreliable in shell scripts and editor integrations. It also encourages platform-specific workarounds that bypass `warpctrl`'s normal channel selection, target selection, authentication, output, and error contracts.

## Goals

- Make `warpctrl file open <path>` work when it is the operation that starts Warp.
- Start the app for the same channel as the invoked `warpctrl` wrapper.
- Preserve the existing authenticated `file.open` request and response path after startup.
- Keep existing instance and target selection deterministic.
- Fail with a bounded, actionable error when Warp cannot become controllable.

## Non-goals

- Adding a top-level `warp <path>` or `warp` desktop-launcher command.
- Adding new directory-opening behavior or a directory-specific command.
- Starting Warp automatically for other `warpctrl` commands, including `instance list`.
- Changing the current interpretation, validation, or editor selection behavior of the `file.open` path.
- Enabling Settings > Scripting from the CLI or weakening the local-control security model.
- Enabling Warp Control on platforms where authenticated local-control publication is unsupported.
- Defining the proposed `file open --wait` lifecycle tracked separately by #8741.

## Behavior invariants

1. If one or more reachable same-channel instances already exist, `warpctrl file open` uses the current instance-selection behavior and does not launch another app process.
2. If no reachable same-channel instance exists and neither `--instance` nor `--pid` was supplied, `warpctrl file open` requests startup of the matching channel's desktop app exactly once, waits up to 10 seconds for a reachable instance, and then sends the existing `file.open` request.
3. An explicit `--instance` or `--pid` selector never launches a replacement process. If the selected process is unavailable, the command keeps the existing `no_instance` behavior.
4. Automatic startup is limited to `file open`. Every other `warpctrl` command keeps its current running-instance requirement; `instance list` still returns an empty list when none is reachable.
5. Startup does not carry the requested file through a URI or an unauthenticated app-open event. The file path, `--line`, `--column`, `--new-tab`, target selectors, and output format are sent only through the existing authenticated `file.open` request after discovery succeeds.
6. After startup, the normal selector rules remain authoritative. If discovery becomes ambiguous, the command returns `ambiguous_instance` instead of guessing which process to target.
7. Concurrent cold-starting `file open` commands for the same channel coordinate so only one of them requests app startup. Each command still sends its own authenticated `file.open` request after the instance becomes reachable.
8. A successful cold start returns the same human-readable or structured success payload as an already-running instance, including the resolved `instance_id`. The response does not expose a separate launcher-only success state.
9. Settings > Scripting remains authoritative. Automatic startup never enables or changes it. If no controllable instance appears within 10 seconds, the command exits non-zero with `no_instance` and explains that Warp was requested to start but may require Scripting to be enabled.
10. Failure to invoke the matching app, an app exit during startup, or timeout never falls back to a different channel, a file URI, or a weaker control transport.
11. The startup request is safe to deliver to an already-running but undiscoverable app: it must not create an extra window or perform the file operation outside `warpctrl`.
12. On platforms where `warpctrl` fails closed today, `file open` continues to fail closed and does not attempt an unsupported startup path.

## User experience

### Cold start

```shell
warpctrl file open AGENTS.md
```

1. No controllable same-channel Warp instance is found.
2. The matching Warp app is requested to start.
3. `warpctrl` waits for the app's authenticated local-control endpoint.
4. The existing `file.open` action opens `AGENTS.md` and focuses its resolved target.
5. The command prints the same success response it would have printed if Warp had already been running.

### Explicit unavailable instance

```shell
warpctrl file open AGENTS.md --instance stale-instance-id
```

The command returns `no_instance` without starting Warp because replacing an explicitly selected process would violate deterministic targeting.

### Scripting disabled

Warp may start or already be running, but it does not publish an actionable discovery record. After 10 seconds, the command returns `no_instance` with guidance to enable Settings > Scripting and retry. The CLI does not change that setting.

## Success criteria

1. With no Warp process running and Scripting enabled, one `warpctrl file open` invocation starts the matching app and opens the requested file.
2. With a reachable app already running, behavior and output are unchanged and no startup request occurs.
3. Explicit unavailable instance selectors do not start Warp.
4. Other `warpctrl` commands do not gain auto-start behavior.
5. Concurrent cold-start commands do not produce duplicate app startup requests or extra windows.
6. Scripting-disabled and startup-timeout paths fail closed with actionable errors.
7. Stable and Preview wrappers start only their corresponding app channel.

## Open questions

- After the broader CLI reorganization settles, should Warp still expose a simple top-level `warp <path>` desktop launcher?
- Should directory opening eventually be a dedicated launcher behavior, a separate `warpctrl` command, or an explicitly supported mode of `file open`?
