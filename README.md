<a href="https://www.warp.dev">
    <img width="1024" alt="Warp Agentic Development Environment product preview" src="https://github.com/user-attachments/assets/9976b2da-2edd-4604-a36c-8fd53719c6d4" />
</a>
&nbsp;
<p align="center">
  <a href="https://www.warp.dev"><img height="20" alt="Built with Warp" src="https://raw.githubusercontent.com/warpdotdev/brand-assets/main/Github/Built-With-Warp-Export@2x.png" /></a>
  &nbsp;
  <a href="https://oz.warp.dev"><img height="20" alt="Powered by Oz" src="https://raw.githubusercontent.com/warpdotdev/brand-assets/main/Github/Powered-By-Oz-Export@2x.png" /></a>
</p>

<p align="center">
  <a href="https://www.warp.dev">Website</a>
  ·
  <a href="https://www.warp.dev/code">Code</a>
  ·
  <a href="https://www.warp.dev/agents">Agents</a>
  ·
  <a href="https://www.warp.dev/terminal">Terminal</a>
  ·
  <a href="https://www.warp.dev/drive">Drive</a>
  ·
  <a href="https://docs.warp.dev">Docs</a>
  ·
  <a href="https://www.warp.dev/blog/how-warp-works">How Warp Works</a>
</p>

> [!NOTE]
> OpenAI is the founding sponsor of the new, open-source Warp repository, and the new agentic management workflows are powered by GPT models.

<h1></h1>

## About

[Warp](https://www.warp.dev) is an agentic development environment, born out of the terminal. Use Warp's built-in coding agent, or bring your own CLI agent (Claude Code, Codex, Gemini CLI, and others).

## Installation

You can [download Warp](https://www.warp.dev/download) and [read our docs](https://docs.warp.dev/) for platform-specific instructions.

## Warp Contributions Overview Dashboard

Explore [build.warp.dev](https://build.warp.dev) to:
- Watch thousands of Oz agents triage issues, write specs, implement changes, and review PRs
- View top contributors and in-flight features
- Track your own issues with GitHub sign-in
- Click into active agent sessions in a web-compiled Warp terminal

## Oz for OSS

Maintaining a popular open-source project? [Apply for Oz credits](https://tally.so/r/LZWxqG) to explore [Oz for OSS](https://github.com/warpdotdev/oz-for-oss).

Oz for OSS is our partner program for bringing the same agentic open-source management workflows used in this repository to select partner repositories. We work directly with maintainers to implement workflows for issue triage, PR review, community management, and contributor coordination in a way that fits each project.

## Licensing

Warp's UI framework (the `warpui_core` and `warpui` crates) are licensed under the [MIT license](LICENSE-MIT).

The rest of the code in this repository is licensed under the [AGPL v3](LICENSE-AGPL).

## Open Source & Contributing

Warp's client codebase is open source and lives in this repository. We welcome community contributions and have designed a lightweight workflow to help new contributors get started. For the full contribution flow, read our [CONTRIBUTING.md](CONTRIBUTING.md) guide.

> [!TIP]
> **Chat with contributors and the Warp team** in the [`#oss-contributors`](https://warpcommunity.slack.com/archives/C0B0LM8N4DB) Slack channel — a good place for ad-hoc questions, design discussion, and pairing with maintainers. New here? [Join the Warp Slack community](https://go.warp.dev/join-preview) first, then jump into `#oss-contributors`.

### Issue to PR

Before filing, [search existing issues](https://github.com/warpdotdev/warp/issues?q=is%3Aissue+is%3Aopen+sort%3Areactions-%2B1-desc) for your bug or feature request. If nothing exists, [file an issue](https://github.com/warpdotdev/warp/issues/new/choose) using our templates. Security vulnerabilities should be reported privately as described in [CONTRIBUTING.md](CONTRIBUTING.md#reporting-security-issues).

Once filed, a Warp maintainer reviews the issue and may apply a readiness label: [`ready-to-spec`](https://github.com/warpdotdev/warp/issues?q=is%3Aissue+is%3Aopen+label%3Aready-to-spec) signals the design is open for contributors to spec out, and [`ready-to-implement`](https://github.com/warpdotdev/warp/issues?q=is%3Aissue+is%3Aopen+label%3Aready-to-implement) signals the design is settled and code PRs are welcome. Anyone can pick up a labeled issue — mention **@oss-maintainers** on an issue if you'd like it considered for a readiness label.

### Building the Repo Locally

To build and run Warp from source:

```bash
./script/bootstrap   # platform-specific setup
./script/run         # build and run Warp
./script/presubmit   # fmt, clippy, and tests
```

See [WARP.md](WARP.md) for the full engineering guide, including coding style, testing, and platform-specific notes.

## Joining the Team

Interested in joining the team? See our [open roles](https://www.warp.dev/careers).

## Support and Questions

1. See our [docs](https://docs.warp.dev/) for a comprehensive guide to Warp's features.
2. Join our [Slack Community](https://go.warp.dev/join-preview) to connect with other users and get help from the Warp team — contributors hang out in [`#oss-contributors`](https://warpcommunity.slack.com/archives/C0B0LM8N4DB).
3. Try our [Preview build](https://www.warp.dev/download-preview) to test the latest experimental features.
4. Mention **@oss-maintainers** on any issue to escalate to the team — for example, if you encounter problems with the automated agents.

## Code of Conduct

We ask everyone to be respectful and empathetic. Warp follows the [Code of Conduct](CODE_OF_CONDUCT.md). To report violations, email warp-coc at warp.dev.

## Open Source Dependencies

We'd like to call out a few of the [open source dependencies](https://docs.warp.dev/help/licenses) that have helped Warp to get off the ground:

- [Tokio](https://github.com/tokio-rs/tokio)
- [NuShell](https://github.com/nushell/nushell)
- [Fig Completion Specs](https://github.com/withfig/autocomplete)
- [Warp Server Framework](https://github.com/seanmonstar/warp)
- [Alacritty](https://github.com/alacritty/alacritty)
- [Hyper HTTP library](https://github.com/hyperium/hyper)
- [FontKit](https://github.com/servo/font-kit)
- [Core-foundation](https://github.com/servo/core-foundation-rs)
- [Smol](https://github.com/smol-rs/smol)

## QEMU VM Helper Scripts

This repository includes two local PowerShell scripts for creating and starting simple QEMU virtual machines on Windows:

- `start-qemu-vm.ps1` creates and starts a general-purpose QEMU VM.
- `start-android-emulator.ps1` creates and starts an Android-x86 QEMU VM.

### Requirements

- Windows PowerShell.
- QEMU installed and available on `PATH`.
- `qemu-img` available on `PATH`.
- `qemu-system-x86_64` available on `PATH`.
- Optional but recommended: Windows Hypervisor Platform enabled for WHPX acceleration.
- An installer ISO for installation workflows.

To verify the QEMU commands are available:

```powershell
qemu-img --version
qemu-system-x86_64 --version
```

If either command is not found, restart the shell so it picks up the persisted user `PATH`.

### General VM Script

Use `start-qemu-vm.ps1` for a standard QEMU VM backed by a QCOW2 disk.

Create a disk and boot an installer ISO:

```powershell
.\start-qemu-vm.ps1 -Name test-vm -IsoPath .\installer.iso -Install
```

Start the installed VM later:

```powershell
.\start-qemu-vm.ps1 -Name test-vm
```

Customize CPU, memory, and disk size:

```powershell
.\start-qemu-vm.ps1 -Name test-vm -MemoryMB 8192 -CpuCount 4 -DiskSizeGB 60
```

Disable WHPX acceleration if it causes startup issues:

```powershell
.\start-qemu-vm.ps1 -Name test-vm -NoAccel
```

Defaults:

- VM name: `basic-vm`
- Disk size: `30` GB
- Memory: `4096` MB
- CPU count: `2`
- VM storage root: `vms`
- Disk path pattern: `vms\<name>\<name>.qcow2`

### Android Emulator Script

Use `start-android-emulator.ps1` with an Android-x86 ISO. It uses Android-friendly defaults, including USB tablet input and user-mode networking.

Create a disk and boot the Android installer:

```powershell
.\start-android-emulator.ps1 -IsoPath .\android-x86.iso -Install
```

Start the installed Android VM later:

```powershell
.\start-android-emulator.ps1
```

Boot an Android live session without creating or using a disk:

```powershell
.\start-android-emulator.ps1 -IsoPath .\android-x86.iso -Live
```

Customize resources:

```powershell
.\start-android-emulator.ps1 -Name android-test -MemoryMB 8192 -CpuCount 4 -DiskSizeGB 32
```

Disable WHPX acceleration if needed:

```powershell
.\start-android-emulator.ps1 -IsoPath .\android-x86.iso -Install -NoAccel
```

Defaults:

- VM name: `google-pixel-9-pro-fold-x86`
- Disk size: `16` GB
- Memory: `4096` MB
- CPU count: `4`
- VM storage root: `vms`
- Disk path pattern: `vms\<name>\<name>.qcow2`

### Common Parameters

- `-Name`: VM name and disk folder name.
- `-IsoPath`: Path to an installer or live ISO.
- `-DiskSizeGB`: QCOW2 disk size to create if the disk does not already exist.
- `-MemoryMB`: RAM assigned to the VM.
- `-CpuCount`: Number of virtual CPUs assigned to the VM.
- `-VmRoot`: Root directory for VM storage.
- `-Install`: Boot from the ISO and attach a VM disk for OS installation.
- `-NoAccel`: Skip WHPX acceleration.

The Android script also supports:

- `-Live`: Boot from the Android ISO without creating or attaching a disk.

### Notes and Troubleshooting

- Press `Ctrl+Alt+G` to release mouse and keyboard capture from the QEMU window.
- The scripts create a disk only when one does not already exist.
- Providing `-IsoPath` without `-Install` in the general VM script boots from disk only.
- In the Android script, `-Install` and `-Live` require `-IsoPath`.
- In the Android script, `-Install` and `-Live` cannot be used together.
- If `-accel whpx` fails, enable Windows Hypervisor Platform or rerun with `-NoAccel`.
- A dummy `.iso` file can test script argument handling, but a real bootable ISO is required to install or run an operating system.
