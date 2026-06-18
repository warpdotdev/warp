# WARPER-011: local save and package blockers

## Summary

Warper should port upstream changes that prevent local file saves from mutating user content unexpectedly and keep retained local bundle/package paths buildable and launchable. This spec is intentionally small: it covers `3f83932c`, `0446a507`, `6eefa4bb`, and `1244ffbe`.

## Why this matters for Warper

XP scope says to implement only when Warper dies without the change. These rows clear that bar because they affect local files or the ability to build and launch Warper artifacts. A save path that rewrites user files is local data mutation. A bundle script that signs or launches the wrong target blocks the fork's test workflow. A Linux package that points to the wrong launcher or references absent packaging templates cannot be shipped as a working local terminal.

## What goes wrong without this

1. A user saves a local code file and Warper can write changes the user did not make. The save path always asks the LSP server to format first, then saves the post-format buffer. A small manual edit can become a rewrite of imports, whitespace, wrapping, or final newlines across the file.
2. This is not preference polish. Files can intentionally require formatting to stay off: generated files, vendored files, patch fixtures, lock-like text, or projects whose formatter version differs from the user's team. Without a disabled format-on-save path, saving can destroy deliberate formatting or create noisy diffs that the user may commit without noticing.
3. A macOS bundle test can build one app and operate on another. When the developer sets Cargo's target directory or builds for a specific Apple target, `cargo bundle` can place `Warper.app` outside `./target`. The run script still cleans, patches, signs, launches, or opens the hardcoded `./target` app path.
4. The fork-specific failure is repeated wrong-architecture bundle churn. A developer trying to test an arm64 bundle can be pushed back through x86/default target artifacts because the script does not use the chosen target directory consistently. A launched app then no longer proves the just-built bundle works.
5. A Linux package can install enough files to show a desktop entry and still fail on launch. The desktop file says `Exec=warp-oss`, while the deb package creates `/usr/bin/warp-terminal...` pointing at the installed binary under `/opt/warper/...`. From the user's perspective the app-menu terminal entry is dead.
6. The launcher mismatch can hide during CLI-only checks. A package maintainer can verify that the installed binary exists from a shell while the desktop environment still points at a command the package did not install.
7. The deb bundler can fail before producing a package because it reads common repo maintainer-script templates that are absent from this fork. The script reaches the append step, the source template path does not exist, and the package build stops.
8. A manual workaround for the missing templates would make release output depend on local untracked edits. That is not a shippable path. Warper needs the checked-in bundler to either include the needed templates or stop referencing them.

## Source commits

| Commit | Upstream why | Current Warper evidence | Resolution |
| --- | --- | --- | --- |
| `3f83932c` | PR `#12254` says always-on LSP format-on-save caused unwanted formatter-driven diffs. | `app/src/code/local_code_editor.rs:984-1079` formats before saving and `:1542-1543` always calls that path. | Port. |
| `0446a507` | PR `#12313` and issue `#11957` say `script/macos/run` failed when Cargo's target dir was outside `./target`. | `script/macos/run:28`, `:62`, and `:69` hardcode relative target bundle paths. | Port. |
| `6eefa4bb` | PR `#9558` fixes upstream OSS desktop `Exec` not matching the installed launcher. | `app/channels/oss/dev.warper.Warper.desktop:10` says `Exec=warp-oss`; `resources/linux/debian/app/postinst.template:4-5` creates `warp-terminal...`; package scripts rename OSS app packages to `warper`. | Port manually. |
| `1244ffbe` | PR `#10019` subject says deb packaging avoids duplicate apt source entries when `.sources` exists; body has no details. | `script/linux/bundle_deb:105-116` reads absent `resources/linux/debian/common/postinst.repo.template` and `postrm.repo.template`. | Port manually. |

## Behavior

1. Local code editor saves do not request LSP formatting when the user disables format-on-save.
2. Format-on-save defaults to current behavior unless the user changes the local setting.
3. `script/macos/run` resolves Cargo's real target directory before computing the `.app` bundle path.
4. `script/macos/run` uses the resolved app path consistently for cleanup, `install_name_tool`, plist update, resources, icon compilation, codesign, launch, and `open`.
5. Warper Linux desktop `Exec` matches the launcher the package actually installs.
6. Deb packaging no longer references absent common repo templates.
7. None of these ports import upstream release automation, cloud update repositories, hosted services, telemetry, Oz, or branding.

## Validation

- Add or run a script-level check for `script/macos/run` with an explicit `CARGO_TARGET_DIR`.
- Add or run packaging checks that the Linux desktop `Exec` target exists after staging.
- Add or run a deb bundling dry-run that proves absent common repo templates are no longer referenced.
- Add local editor tests for format-on-save default behavior and disabled behavior.
