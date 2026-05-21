# APP-4281: SSH into hosts with unsupported glibc

Linear: APP-4281

## Summary

When a user SSHes into a Linux host whose glibc (or libc family) is too old to run Warp's prebuilt remote-server binary, Warp must avoid offering or attempting an install that would never succeed. The user lands in the legacy SSH experience without seeing any error banner, modal, or install prompt — the SSH session feels indistinguishable from a normal SSH into a host where remote-server features simply aren't enabled.

## Figma

Figma: none provided. This feature is primarily about *suppressing* UI surfaces; there are no new visual states to design.

## Problem

The prebuilt Linux `oz` binary that powers Warp's remote-server SSH integration requires a recent glibc. When it lands on a host with an older glibc (RHEL/CentOS 7/8, Amazon Linux 2, Ubuntu 18.04, Debian 10, etc.) or a non-glibc libc (Alpine/musl, Termux/bionic), the dynamic loader refuses to launch it with errors like:

```
/lib64/libm.so.6: version `GLIBC_2.29' not found
```

Today, the user sees the install prompt, the install "succeeds," and the failure surfaces only when Warp tries to spawn the proxy — at which point the user sees a generic `SetupFailed` state. Worse, the failed install is left on disk, so every subsequent SSH session repeats the same failed cycle.

## Behavior

### Pre-detection: ideal path (host's libc is positively detected as unsupported)

1. When the user SSHes into a Linux host and Warp can positively detect that the host's libc is unsupported (glibc below the supported floor, or a non-glibc libc such as musl or bionic), Warp does **not** present the "Install Warp SSH Extension" choice block, regardless of the user's `SshExtensionInstallMode` setting (`AlwaysAsk`, `AlwaysInstall`, `NeverInstall`).

2. On an unsupported-libc host, Warp does **not** invoke the install script, does **not** download the binary, and does **not** attempt to launch the remote-server proxy.

3. On an unsupported-libc host, the SSH session falls back to the legacy SSH flow (the same flow used today when the user has chosen `NeverInstall`, or when the install is skipped). The user gets a working shell with normal command execution; remote-server-specific features (e.g. richer completions, repo metadata) are simply absent for that session.

4. While Warp is determining whether the host is supported, the prompt area shows the same loading state it shows today during the binary-check phase ("Starting shell..." / "Checking..."). When the determination completes and the host is unsupported, the loading state ends and the legacy SSH prompt appears with no error banner, no failure block, and no modal.

5. If the host has an existing remote-server binary on disk from a previous (now-incompatible) install, Warp removes that stale binary as part of the fall-back so the host does not accumulate unusable files. The cleanup is silent — the user is not asked to confirm and is not informed if it fails.

6. The fall-back is sticky for the duration of the SSH session: once Warp has decided the host is unsupported, it does not retry the install, does not show the choice block, and does not show a failure banner mid-session.

7. Every subsequent SSH into the same host repeats the same detection and reaches the same conclusion. The user is never re-prompted with the install choice block on a host known to be unsupported. If the host is later upgraded so that its libc becomes supported, the next SSH detects the change and the normal install/auto-update flow resumes — there is no client-side cache that would prevent recovery.

### Fallback path: install or launch fails despite supported detection

8. If pre-detection cannot positively classify the host (the libc probe didn't run, returned unparseable output, or otherwise failed), Warp proceeds with today's behavior: it offers or runs the install according to the user's `SshExtensionInstallMode` setting.

9. If the install itself fails, or the install succeeds but the remote-server proxy fails to launch (for example, because the binary's loader rejects the host's glibc), Warp must not leave the user stranded. The SSH session falls back to the legacy SSH flow and the user gets a working shell.

10. In the fallback-after-failure path, Warp may surface a single, dismissible failure banner explaining that the SSH extension could not be installed/launched on this host (consistent with today's `SshRemoteServerFailedBanner`). It must **not** loop on the failure: the user does not see a new banner, modal, or install prompt for the same host on subsequent SSH sessions in the same Warp run.

11. If a previous install attempt left an incompatible binary on disk, Warp cleans it up before falling back so the next SSH does not silently re-enter the auto-update / re-fail loop described in the Problem section.

### Cross-cutting invariants

12. macOS remote hosts are unaffected. Warp does not run a libc probe against macOS hosts and does not change any existing macOS SSH behavior.

13. Hosts with supported glibc are unaffected. The install prompt, auto-update, loading footer, and connect flow behave exactly as they do today.

14. The user's `SshExtensionInstallMode` setting is honored:
    - `AlwaysAsk`: the choice block is shown only when the host is supported (or when pre-detection was inconclusive). It is never shown on a host known to be unsupported.
    - `AlwaysInstall`: install runs only when the host is supported (or pre-detection was inconclusive). On a known-unsupported host, install is silently skipped and the session falls back to legacy SSH.
    - `NeverInstall`: behavior is unchanged — Warp falls back to legacy SSH regardless of host support.

15. Detection latency must be small enough that the user does not perceive an additional delay before the legacy SSH prompt appears on an unsupported host. The probe runs over the existing SSH connection and adds at most a single round-trip on Linux hosts.

16. SSH-level failures during detection (timeout, broken pipe, permission denied) do not block the SSH session. If detection cannot run, Warp treats the result as inconclusive and follows invariant 8.

17. Nothing about an unsupported-libc fall-back is presented as an error to the user. The legacy SSH session that results is a normal, working shell; the absence of remote-server features is the only observable difference, and it matches the experience the user already has today on hosts where they chose not to install the extension.

18. Detection and fall-back state is per-host, not global. Encountering one unsupported host does not change behavior for any other host the user SSHes into in the same Warp session.

## Open questions

- **User awareness:** is it acceptable that the user has *no* signal that Warp's SSH integration is intentionally inactive on this host? Some users may wonder why richer completions are missing. A future iteration may surface a one-time, dismissible explanation in the SSH choice area, but the default for this spec is silent fall-back.
