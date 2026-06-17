# Upstream PR Grounding Check

This document records the correction pass after the audit was tightened to XP scope control. The pass used `git show` commit bodies/diffs, public `gh pr view`/`gh issue view` metadata where available, and current Warper source reads; high-numbered security PRs that are not publicly resolvable are not treated as PR-grounded, and their upstream why comes only from commit bodies, advisory IDs in commit text, changed files, and tests.

## Corrections Made

| Area | Earlier problem | Correction |
| --- | --- | --- |
| XP threshold | The audit treated "retained path plus upstream bug" as enough to port. | `Port` now means unsafe-to-run, local data corruption, normal-use crash/corruption, or current build/package blockage. Rows without that evidence stay out of implementation scope. |
| Private security rows | Rows used a vague "security diff" label as if that were an upstream reason. | Each row now says whether the PR is publicly resolvable and gives the actual commit-body cause: RCE advisory, command injection, clipboard exposure, file overwrite, spoofed DCS hooks, or restore `cd` injection. |
| `0446a507` | The row claimed generic `CARGO_TARGET_DIR` breakage and missed the fork reason. | The row now cites PR `#12313` and issue `#11957`: shared Cargo target dirs made `cargo bundle` and post-bundle steps disagree. Warper relevance is the fork's need to test explicit target dirs/archs without launching or signing the wrong bundle. |
| Terminal/input | IME, zsh, PATH, and key fixes were over-promoted. | Kept only flat-storage underflow as `Port`; the rest are deferred until a retained-platform smoke test or user report proves breakage. |
| Repo metadata | Scale/fidelity improvements were treated as WARPER-005 necessities. | Moved to `Defer` except local file-edit corruption in `a1b76c28`. |
| Rule/skill/MCP | Rule, skill, MCP, and selected-context rows became a spec without a failing acceptance test. | The fake spec file was deleted. These rows stay in the audit until a WARPER-005 acceptance test fails. |
| Build/package | Packaging rows were grouped without proving current Warper paths. | Kept only current blockers: macOS target-dir bundle path, Linux launcher naming mismatch, and deb bundler references to absent common repo templates. |

## Dependency And CoreText Grounding

| Commit | Public evidence | Current Warper evidence | XP decision |
| --- | --- | --- | --- |
| `9d9972cb` | PR `#10263` says Diesel's SQLite backend can corrupt UTF-8 and that `2.3.8+` is patched. | `Cargo.toml:123` pins vulnerable Diesel `2.3.4`; `cargo tree -i diesel` resolves it into `app`, `persistence`, and `warp_server_client`; `app/src/persistence/sqlite.rs:762`, `:1209`, `:1266`, and `:2016` write local state through Diesel. | Port; this is reachable local SQLite data-corruption risk. |
| `64a0dfbe` | PR `#10060` says `rand 0.9.1` has `GHSA-cq8v-f236-94qc`, severity CVSS `0.0`, triggered by `rand::rng()` with a custom logger. | `crates/managed_secrets/Cargo.toml:31` pins `rand = "0.9"` and `cargo tree -i rand@0.9.1` finds transitive users, but this audit did not find the custom-logger trigger. | Defer; not runtime survival work from current evidence. |
| `ac091058` | PR `#10513` is a Dependabot OpenSSL bump with release-note fixes. | `Cargo.lock` contains `openssl`, but `cargo tree -i openssl` and escalated `cargo tree --target=all --all-features -i openssl` find no resolved package. | Skip; stale lockfile entry. |
| `cc1ee636` | PR `#12090` says `tar 0.4.46` fixes `GHSA-3cv2-h65g-fgmm`, a PAX header desync. | `cargo tree -i tar` shows `tar` through `crates/node_runtime`; `crates/node_runtime/src/lib.rs:205-232` downloads Node and `:282-287` unpacks the tarball. | Defer; retained remote-archive path exists, but the current input is a pinned Node distribution over HTTPS, not arbitrary tar data. |
| `2f84587a` | PR `#9665` says `CTFontCollection::get_descriptors` leaked descriptor arrays during startup font enumeration, font picker use, font loading, and CJK/emoji fallback chains. | `Cargo.toml:467-470` pins the older `core-foundation-rs` rev; `crates/warpui/src/platform/mac/fonts.rs:82-88` and `:336-358` call `get_descriptors()`. | Defer; real retained macOS leak path, but no measurement or crash evidence proving Warper dies without the bump. |

## Public PR Findings

| Area | Public evidence | XP decision |
| --- | --- | --- |
| macOS target dir | PR `#12313` and issue `#11957` document `script/macos/run` failing when Cargo writes bundles outside `./target`. | Port, because Warper's fork relies on explicit target-dir bundle testing. |
| Format on save | PR `#12254` documents always-on LSP formatting rewriting files on save. | Port, because local save must not mutate user content unexpectedly. |
| Linux desktop launcher | PR `#9558` documents upstream OSS desktop `Exec` mismatch. | Port manually, because Warper has a renamed but still inconsistent launcher/package path. |
| Linux bootstrap deps | PR `#9527` documents fresh Linux bootstrap missing `clang-format` and `libclang-dev`. | Defer; useful, but not app/build survival unless current Linux CI/release setup fails. |
| Terminal/input | PRs `#12438`, `#12473`, `#9514`, `#9711`, `#9730`, `#10443`, and `#12277` describe real terminal bugs. | Defer under XP until current Warper smoke tests or user reports prove stop-ship failure. |
| Flat storage | PR `#12085` fixes grid storage underflow. | Port, because crash/corruption in normal terminal rendering clears the XP bar. |
| Local repo/file tools | PRs include real file-tool and repo metadata improvements. | Port only `#9623` multiline diff corruption; defer scale/result-size/fidelity work until a current workflow fails. |
| MCP/rules/skills | PRs `#10238`, `#11978`, `#12040`, `#10874`, and `#11297` describe plausible retained-path fixes. | Defer until there is a failing WARPER-005 acceptance test. |

## Private Or Not Publicly Resolvable Security References

| Commit | Public PR status | Grounding used |
| --- | --- | --- |
| `4295ec08` | PR `#25398` not publicly resolvable. | Commit body names display-chip RCE `GHSA-hgvx-4xvm-39pw`; diff touches chip command construction/tests. |
| `7f0c4dd2` | PR `#25353` not publicly resolvable. | Commit body says markdown `OpenFileWithTarget` should only be emitted for trusted known-extension targets. |
| `43f4f483` | PR `#25351` not publicly resolvable. | Commit title and diff target command injection in code search tools. |
| `861dacea` | PR `#25365` not publicly resolvable. | Commit body explains `sh -c` external-editor command injection through filenames and desktop field-code expansion. |
| `0c1e2432` | PR `#25258` not publicly resolvable. | Commit body explains env-var prefix denylist bypass. |
| `b6caa957` | PR `#26138` not publicly resolvable. | Commit body explains unquoted `is_file_path`/`is_git_repository` shell predicates. |
| `b1a41d0b` | PR `#25339` not publicly resolvable. | Commit body cites OSC 52 clipboard advisory `GHSA-wgqj-4c26-7c4g`. |
| `f3b9ce1c` | PR `#25261` not publicly resolvable. | Commit body explains non-inline iTerm file payload overwrite. |
| `32d21d15` | PR `#25395` not publicly resolvable. | Commit body explains spoofable DCS lifecycle hooks and session ID integrity checks. |
| `c697c8f5` | PR `#25383` not publicly resolvable. | Commit body cites restore `cd` advisory `GHSA-8659-m852-gmfx`. |

## Remaining Limits

- SSH command-injection work needs an explicit retained-SSH decision before implementation.
- Public PR titles alone are never used as upstream why when the body or issue is unavailable.
