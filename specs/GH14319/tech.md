# Recognize Snowflake CoCo CLI as a CLI agent — Tech Spec

Product spec: [`specs/GH14319/product.md`](product.md)

GitHub issue: https://github.com/warpdotdev/warp/issues/14319

Code references inspected at commit: `69254d73db0c568db55333cad1d3090041cd334a`

## Context

Issue #14319 asks Warp to recognize Snowflake's CLI agent, whose current official product name is **Snowflake CoCo** and whose user-facing command-line surface is **CoCo CLI**. Snowflake's official [CoCo CLI documentation](https://docs.snowflake.com/en/user-guide/cortex-code/cortex-code-cli) says that the installer places a `cortex` executable in `~/.local/bin` on macOS, Linux, and WSL and instructs users to launch it with `cortex`. The official [CLI reference](https://docs.snowflake.com/en/user-guide/cortex-code/cli-reference) documents `cortex` and argument forms including `-c`, `--continue`, `--resume`, and `--plan`. A commit-pinned Snowflake-Labs [installer implementation](https://github.com/Snowflake-Labs/snowflake-ai-kit/blob/df14567511849307f124f09397b8426f3ae78293/actions/cortex-code/src/cortex/install.ts#L3-L24) independently identifies the default executable as `cortex`. Current distribution artifacts may use `coco` in their package names, but Snowflake has not documented `coco` as a launch executable, so this feature must not recognize it as an alias.

The current Warp code already centralizes the recognition and presentation of local third-party CLI agents:

- [`CONTRIBUTING.md:92-107 @ 69254d73d`](https://github.com/warpdotdev/warp/blob/69254d73db0c568db55333cad1d3090041cd334a/CONTRIBUTING.md#L92-L107) defines the issue-linked product- and tech-spec workflow used here.
- [`app/src/terminal/cli_agent.rs:138-190 @ 69254d73d`](https://github.com/warpdotdev/warp/blob/69254d73db0c568db55333cad1d3090041cd334a/app/src/terminal/cli_agent.rs#L138-L190) defines `CLIAgent` variants and exact executable prefixes.
- [`app/src/terminal/cli_agent.rs:193-272 @ 69254d73d`](https://github.com/warpdotdev/warp/blob/69254d73db0c568db55333cad1d3090041cd334a/app/src/terminal/cli_agent.rs#L193-L272) owns serialized-name fallback, cloud-harness conversion, display names, icons, and supported skill providers.
- [`app/src/terminal/cli_agent.rs:308-368 @ 69254d73d`](https://github.com/warpdotdev/warp/blob/69254d73db0c568db55333cad1d3090041cd334a/app/src/terminal/cli_agent.rs#L308-L368) declares agent-specific skill-command, shell-mode, rich-input-footer, brand-color, and icon-color capabilities.
- [`app/src/terminal/cli_agent.rs:371-435 @ 69254d73d`](https://github.com/warpdotdev/warp/blob/69254d73db0c568db55333cad1d3090041cd334a/app/src/terminal/cli_agent.rs#L371-L435) parses the top-level executable, normalizes a path to its basename, resolves aliases, and detects a known agent.
- [`app/src/terminal/cli_agent.rs:611-633 @ 69254d73d`](https://github.com/warpdotdev/warp/blob/69254d73db0c568db55333cad1d3090041cd334a/app/src/terminal/cli_agent.rs#L611-L633) exhaustively converts detected agents to telemetry's `CLIAgentType`.
- [`app/src/terminal/cli_agent_tests.rs:255-435 @ 69254d73d`](https://github.com/warpdotdev/warp/blob/69254d73db0c568db55333cad1d3090041cd334a/app/src/terminal/cli_agent_tests.rs#L255-L435) provides table-driven recognition, arguments, whitespace, near-match, alias, path, and environment-variable coverage.
- [`app/src/terminal/cli_agent_tests.rs:531-545 @ 69254d73d`](https://github.com/warpdotdev/warp/blob/69254d73db0c568db55333cad1d3090041cd334a/app/src/terminal/cli_agent_tests.rs#L531-L545) round-trips the opaque serialized name for every `CLIAgent` variant.
- [`crates/input_classifier/src/util.rs:14-87 @ 69254d73d`](https://github.com/warpdotdev/warp/blob/69254d73db0c568db55333cad1d3090041cd334a/crates/input_classifier/src/util.rs#L14-L87) preserves recognized one-word CLI-agent commands as shell commands before heuristic natural-language classification.
- [`crates/input_classifier/src/util_tests.rs:19-44 @ 69254d73d`](https://github.com/warpdotdev/warp/blob/69254d73db0c568db55333cad1d3090041cd334a/crates/input_classifier/src/util_tests.rs#L19-L44) supplies the shared one-off keyword cases exercised by both heuristic profiles.
- [`app/src/terminal/view.rs:12078-12137 @ 69254d73d`](https://github.com/warpdotdev/warp/blob/69254d73db0c568db55333cad1d3090041cd334a/app/src/terminal/view.rs#L12078-L12137) turns a detected long-running command into a `CLIAgentSession` and separately gates agent-specific listeners and rich-input auto-toggle behavior.
- [`app/src/terminal/view.rs:11908-11925 @ 69254d73d`](https://github.com/warpdotdev/warp/blob/69254d73db0c568db55333cad1d3090041cd334a/app/src/terminal/view.rs#L11908-L11925) removes the local CLI-agent session and closes rich input when the command completes.
- [`app/src/terminal/view.rs:23263-23326 @ 69254d73d`](https://github.com/warpdotdev/warp/blob/69254d73db0c568db55333cad1d3090041cd334a/app/src/terminal/view.rs#L23263-L23326) routes existing user-selected code/review context to any active CLI agent, using rich input when open and otherwise writing plain text to the PTY.
- [`app/src/workspace/view/right_panel.rs:1352-1368 @ 69254d73d`](https://github.com/warpdotdev/warp/blob/69254d73db0c568db55333cad1d3090041cd334a/app/src/workspace/view/right_panel.rs#L1352-L1368) sends review comments to an active CLI agent, while [`app/src/workspace/view/right_panel.rs:1400-1459 @ 69254d73d`](https://github.com/warpdotdev/warp/blob/69254d73db0c568db55333cad1d3090041cd334a/app/src/workspace/view/right_panel.rs#L1400-L1459) treats an active long-running CLI agent as an available review destination.
- [`app/src/ui_components/agent_icon.rs:23-98 @ 69254d73d`](https://github.com/warpdotdev/warp/blob/69254d73db0c568db55333cad1d3090041cd334a/app/src/ui_components/agent_icon.rs#L23-L98) resolves the current terminal's local or cloud agent identity for shared UI consumers.
- [`app/src/ui_components/agent_icon_tests.rs:1-335 @ 69254d73d`](https://github.com/warpdotdev/warp/blob/69254d73db0c568db55333cad1d3090041cd334a/app/src/ui_components/agent_icon_tests.rs#L1-L335) models plain-terminal and command-detected CLI-session inputs and checks the shared icon resolver across surfaces.
- [`app/src/ui_components/icon_with_status.rs:228-255 @ 69254d73d`](https://github.com/warpdotdev/warp/blob/69254d73db0c568db55333cad1d3090041cd334a/app/src/ui_components/icon_with_status.rs#L228-L255) renders a CLI-agent icon over its brand-color background and otherwise falls back to the generic terminal icon.
- [`app/src/workspace/view/vertical_tabs.rs:3295-3324 @ 69254d73d`](https://github.com/warpdotdev/warp/blob/69254d73db0c568db55333cad1d3090041cd334a/app/src/workspace/view/vertical_tabs.rs#L3295-L3324) consumes the shared terminal-agent icon resolver for vertical tabs.
- [`crates/warp_core/src/ui/icons.rs:276-290 @ 69254d73d`](https://github.com/warpdotdev/warp/blob/69254d73db0c568db55333cad1d3090041cd334a/crates/warp_core/src/ui/icons.rs#L276-L290) declares existing third-party agent icon variants, and [`crates/warp_core/src/ui/icons.rs:624-638 @ 69254d73d`](https://github.com/warpdotdev/warp/blob/69254d73db0c568db55333cad1d3090041cd334a/crates/warp_core/src/ui/icons.rs#L624-L638) maps them to bundled vector assets.
- [`app/src/server/telemetry/events.rs:451-472 @ 69254d73d`](https://github.com/warpdotdev/warp/blob/69254d73db0c568db55333cad1d3090041cd334a/app/src/server/telemetry/events.rs#L451-L472) defines the analytics-facing CLI-agent enum.
- [`app/src/terminal/cli_agent_sessions/listener/mod.rs:38-80 @ 69254d73d`](https://github.com/warpdotdev/warp/blob/69254d73db0c568db55333cad1d3090041cd334a/app/src/terminal/cli_agent_sessions/listener/mod.rs#L38-L80) limits structured activity listeners to agents with verified protocols.
- [`app/src/terminal/cli_agent_sessions/listener/mod_tests.rs:116-260 @ 69254d73d`](https://github.com/warpdotdev/warp/blob/69254d73db0c568db55333cad1d3090041cd334a/app/src/terminal/cli_agent_sessions/listener/mod_tests.rs#L116-L260) contains focused support and handler tests for the listener policy.
- [`app/src/terminal/cli_agent_sessions/plugin_manager/mod.rs:239-305 @ 69254d73d`](https://github.com/warpdotdev/warp/blob/69254d73db0c568db55333cad1d3090041cd334a/app/src/terminal/cli_agent_sessions/plugin_manager/mod.rs#L239-L305) limits plugin installation and discovery to explicit integrations.
- [`app/src/terminal/cli_agent_sessions/plugin_manager/mod_tests.rs:1-38 @ 69254d73d`](https://github.com/warpdotdev/warp/blob/69254d73db0c568db55333cad1d3090041cd334a/app/src/terminal/cli_agent_sessions/plugin_manager/mod_tests.rs#L1-L38) verifies that unsupported agents receive no plugin manager.
- [`crates/ai/src/skills/skill_provider.rs:31-154 @ 69254d73d`](https://github.com/warpdotdev/warp/blob/69254d73db0c568db55333cad1d3090041cd334a/crates/ai/src/skills/skill_provider.rs#L31-L154) defines supported skill providers and their search paths; CoCo is not currently one of them.
- [`script/presubmit:1-70 @ 69254d73d`](https://github.com/warpdotdev/warp/blob/69254d73db0c568db55333cad1d3090041cd334a/script/presubmit#L1-L70) is the repository-owned format, test-module, three-profile Clippy, native-format, nextest, and doc-test entry point.

The implementation has one unresolved external dependency. Current Snowflake documentation uses **CoCo CLI**, while commit-pinned official [Snowflake-Labs plugin metadata](https://github.com/Snowflake-Labs/snowflake-ai-kit/blob/df14567511849307f124f09397b8426f3ae78293/plugins/cortex-code/.codex-plugin/plugin.json#L1-L24) still uses **Snowflake Cortex Code** / **Cortex Code CLI** and publishes `#29B5E8` as that plugin's brand color. The implementation must therefore keep the current documentation name for Warp's visible label while treating the older metadata as evidence of an ongoing naming transition, not as authority to revert the UI label.

Snowflake-Labs publishes only a commit-pinned [PNG logo in `snowflake-ai-kit`](https://github.com/Snowflake-Labs/snowflake-ai-kit/blob/df14567511849307f124f09397b8426f3ae78293/actions/cortex-code/assets/logo.png). Its [Apache-2.0 license section 6](https://github.com/Snowflake-Labs/snowflake-ai-kit/blob/df14567511849307f124f09397b8426f3ae78293/LICENSE#L138-L141) reserves trademark rights, so that repository alone is not enough evidence for a contributor to invent or automatically trace a Warp SVG treatment. No official CoCo vector asset has been found. Maintainers must therefore select an acceptable asset source and treatment; this spec does not make a legal determination about what permission is required. Although `#29B5E8` is grounded in official plugin metadata rather than inferred from documentation chrome, its use in Warp remains part of that maintainer decision.

## Proposed changes

### 1. Add an exact local CoCo CLI identity

Add `CLIAgent::Coco` to `app/src/terminal/cli_agent.rs` with these explicit properties. The identity and deterministic tests may be prepared while section 3's visual-asset decision is pending, but the user-visible implementation is not complete until that decision is resolved:

- `command_prefixes()` returns only `&["cortex"]`.
- `display_name()` returns `"CoCo CLI"`.
- `icon()` returns the new approved CoCo icon variant.
- `brand_color()` and `brand_icon_color()` use only the maintainer-selected asset treatment. The official plugin metadata's `#29B5E8` is a grounded candidate, not a requirement.
- `supported_skill_providers()` returns an empty slice.
- `supports_bash_mode()` returns `false` for Warp's agent-integration capability. CoCo's own documented shell escape remains wholly inside CoCo.
- `supports_cli_agent_footer()` returns `false`. Recognition must not opt CoCo into Warp's managed prompt composer before its input and submission semantics have been separately specified and verified against the real CLI.

Do not add a `Harness` conversion. This is a local executable identity, not a Warp-hosted cloud agent. Retain the existing `Unknown` serialized-name fallback.

Because `CLIAgent` matches are intentionally exhaustive, add `Coco` explicitly to every affected match. Where a structural return value is required by code that is unreachable when `supports_cli_agent_footer()` is false, select the conservative existing inline submission strategy and document the capability gate; do not infer or emulate CoCo keystrokes.

### 2. Reuse existing detection and command classification

The existing `CLIAgent::detect` data flow already provides the required semantics:

1. The terminal reports the top-level command when a long-running process starts.
2. `CLIAgent::detect` parses the executable, reduces an explicit path to its basename, and resolves a shell alias when available.
3. Enum iteration compares the normalized executable to each exact `command_prefixes()` entry.
4. `TerminalView` creates a local `CLIAgentSession` carrying `CLIAgent::Coco`.

Do not add substring, output-text, process-title, environment-variable, or remote detection. Exact basename matching prevents near matches from acquiring the CoCo identity and keeps recognition independent of product versions.

Add `"cortex"` to `ONE_OFF_SHELL_COMMAND_KEYWORDS` in `crates/input_classifier/src/util.rs`. This prevents a bare executable name from being routed to the natural-language path and mirrors the existing treatment of recognized one-word CLI-agent commands.

### 3. Resolve the icon and color with maintainers

Before the implementation PR is ready for review, ask maintainers to select or accept the asset source and color treatment. Suitable outcomes include an authoritative Snowflake vector asset or a Warp-maintainer-provided asset whose provenance and intended treatment are acceptable under the project's normal review process. Record that decision in the implementation PR.

After the decision is made:

- add a descriptively named icon variant such as `Icon::CocoLogo` to `crates/warp_core/src/ui/icons.rs`;
- map it to the selected bundled SVG without a contributor automatically tracing the available PNG;
- preserve the selected asset's proportions and determine `brand_color()` and `brand_icon_color()` from the accepted treatment, considering the officially published `#29B5E8` value; and
- record the asset source and maintainer decision in the implementation PR.

If the decision is still open, the identity and test work may remain in draft, but the PR must not be presented as completing the requested branded identity. Do not substitute a generic terminal glyph and describe it as the CoCo icon.

### 4. Let shared UI consumers render the new identity

No separate vertical-tab or pane-header detection path is needed. Once the local session carries `CLIAgent::Coco`, the shared agent-icon resolver supplies the icon, brand color, and CLI-agent status variant to its current consumers. The lifecycle remains owned by `TerminalView`: the identity appears only after the current long-running-command signal and disappears when that command completes.

The implementation must inspect every current consumer of `terminal_agent_icon` during review and confirm that the new icon has no clipped geometry, misleading status decoration, or generic-terminal fallback. Surface-specific code should change only if a current consumer cannot correctly render the approved icon through the shared path.

### 5. Keep CoCo-specific protocol, plugin, skills, and managed-input integrations absent

In `cli_agent_sessions::listener`, keep `Coco` out of `is_agent_supported` and add it to the `None` arm of `create_handler`. In `cli_agent_sessions::plugin_manager`, add `Coco` to the `None` arm of `plugin_manager_for_with_shell`. Extend `listener/mod_tests.rs` and `plugin_manager/mod_tests.rs` to assert both policies. Do not add an OSC identifier or interpret terminal output as structured CoCo events: no official Warp-compatible protocol has been verified.

Keep `supported_skill_providers()` empty and make no changes to `SkillProvider`. Although official CoCo [extensibility documentation](https://docs.snowflake.com/en/user-guide/cortex-code/extensibility) describes `.cortex/skills` and `.claude/skills` locations and `$`-prefixed skill invocation, adding a provider would create separate discovery, compatibility, and watcher behavior outside issue #14319. Do not add a `skill_command_prefix()` exception in this patch; a future CoCo skills integration would need its own specification and would need to preserve that documented `$` behavior.

The `supports_cli_agent_footer()` gate above prevents recognized CoCo sessions from automatically enabling Warp-managed rich input. Snowflake's [keyboard documentation](https://docs.snowflake.com/en/user-guide/cortex-code/keyboard-shortcuts) describes Enter submission, Ctrl+J newlines, and paste shortcuts, but it does not establish which of Warp's PTY write-framing strategies is safe. Starting and using `cortex` therefore continues to occur in the terminal application itself; a separate verified change may opt it into the footer later.

Do not add a CoCo-specific text-framing path. The existing generic CLI-agent context actions intentionally remain available: once CoCo is the active CLI agent, explicit user actions from Code or Code Review may route their generated plain-text context to the PTY through `try_send_text_to_cli_agent_or_rich_input`. Because `supports_cli_agent_footer()` is false, these actions take the existing PTY branch rather than Warp-managed rich input. They append text but do not synthesize CoCo's submit keystroke. Preserve this behavior and cover it with the existing routing test seam; excluding CoCo would require a separate product decision and a new per-agent context-delivery capability.

### 6. Extend telemetry and preserve serialization compatibility

Add `CLIAgentType::Coco` in `app/src/server/telemetry/events.rs` and map `CLIAgent::Coco` to it in the exhaustive conversion. This is an additive classification for the existing CLI-agent event stream; it does not introduce a new event, payload field, account identifier, or terminal-content capture.

Before release, confirm with the telemetry schema owner that downstream ingestion and dashboards accept the additive `Coco` value. If that contract requires a schema registry update outside this repository, land or coordinate it before emitting the value in production.

The opaque serialized local-agent name becomes `"Coco"` through the existing enum serialization. No database migration is required. Existing clients already map unrecognized serialized names to `CLIAgent::Unknown`; retain and test that behavior so an older client can degrade to the generic identity rather than fail when it encounters a newer value. Do not add a cloud `Harness::Coco` or change network serialization for hosted agents.

## Testing and validation

### Unit tests

1. Extend the table-driven cases in `app/src/terminal/cli_agent_tests.rs` to cover behavior 1–4:
   - `cortex` and representative official argument forms resolve to `CLIAgent::Coco`;
   - absolute and relative executable paths whose basename is `cortex` resolve to CoCo;
   - an alias that expands to `cortex` resolves to CoCo;
   - `coco`, `cortex-code`, `cortex2`, `my-cortex`, and commands that merely contain the word do not resolve to CoCo.

2. Add focused metadata and lifecycle-capability assertions covering behavior 6–8 and 11–13:
   - display name is exactly `CoCo CLI`;
   - the approved icon and approved color treatment are returned;
   - rich input, structured listeners, plugins, hosted harnesses, shell-mode integration, and skill providers remain disabled;
   - all pre-existing agent metadata cases remain unchanged.

3. Extend the all-variant serialization test to round-trip `CLIAgent::Coco`, and retain a focused unknown-string assertion. Add an assertion that `CLIAgent::Coco` converts to `CLIAgentType::Coco`. These cover behavior 13–14 and the additive telemetry contract.

4. Add `"cortex"` to the shared one-off command-keyword table in `crates/input_classifier/src/util_tests.rs`. Both current heuristic profiles must classify bare `cortex` as a shell command, covering behavior 5.

5. Extend `app/src/ui_components/agent_icon_tests.rs` with a `LocalCocoCommandDetected` canonical state. Build its `TerminalIconInputs` with `CLISessionInputs { agent: CLIAgent::Coco, has_listener: false, supports_rich_status: false, .. }`, and assert that `agent_icon_variant_from_terminal_inputs` returns the local CLI-agent variant. The existing `PlainTerminal` state remains the pure-state assertion for the post-session generic fallback. Verify actual session removal in the existing `TerminalView` lifecycle harness if a focused event test is practical; otherwise cover that transition in the manual proof rather than inventing a new test-only lifecycle.

6. Extend the existing `try_send_text_to_cli_agent_or_rich_input`/code-review routing tests with an active `CLIAgent::Coco` session and closed rich input. Assert that an explicit context action selects `CliAgentRouting::Pty`, writes the generated plain text without an automatic submit keystroke, and does not open rich input. This covers product behavior 11 and prevents the new identity from silently changing generic context delivery.

Run the focused suites after implementation, using the exact package and feature names confirmed from that implementation base:

```bash
cargo nextest run --no-tests=fail -p warp -E 'test(/cli_agent|agent_icon/)'
cargo nextest run --no-tests=fail -p input_classifier --features nld_heuristic_v1 -E 'test(test_is_likely_shell_command_one_off_keyword_short_circuits_true_for_nld_heuristic_v1)'
cargo nextest run --no-tests=fail -p input_classifier --features nld_heuristic_v2 -E 'test(test_is_likely_shell_command_one_off_keyword_short_circuits_true_for_nld_heuristic_v2)'
```

The two input-classifier commands exercise the separately compiled heuristic profiles rather than assuming the default profile covers both.

### Manual GUI proof for the implementation PR

Use a supported GUI build and an official, unmodified CoCo CLI installation whose executable resolves to `cortex`. A valid interactive CoCo environment may require a Snowflake account; `cortex --help` is not sufficient because it may exit before Warp's long-running-command lifecycle activates.

Record one concise before/after proof with the same interactive invocation:

1. On the inspected base commit, start `cortex` and capture the generic terminal identity.
2. On the implementation commit, start the same `cortex` executable and capture **CoCo CLI** with the approved icon in every current local-agent identity surface available in that layout, including the visible tab/list treatment.
3. Keep the session active long enough to show that CoCo remains directly interactive and that no Warp-managed rich-input composer, plugin control, or unsupported structured status appears. Exercise one user-initiated generic context action and show that it appends plain text without automatically submitting it.
4. Exit CoCo and show the ordinary terminal identity returning without starting another agent.
5. Repeat the icon check in both a light and a dark Warp theme, at the smallest currently supported tab-icon size.

Include the resolved executable path and `cortex` version in the evidence notes, but redact account names, prompts, terminal content, and credentials. If the contributor cannot access an official interactive CoCo environment, do not present `--help`, a mock binary, or a renamed process as product proof; coordinate a maintainer-run verification before moving an implementation PR to ready for review.

This proof covers behavior 1, 6–11, and 15. Behavior 2–5 and 12–14 remain deterministic unit-test concerns.

### Repository checks

Before pushing an implementation update, run:

```bash
./script/format
./script/presubmit
```

At the inspected commit, `script/presubmit` owns the required format check, no-inline-test-module check, workspace/default-GUI/`warp_completer` Clippy profiles, native-format checks, nextest run, and doc tests. The implementation PR must report the complete result and any platform-specific skips honestly.

## Parallelization

The spec must be approved before implementation. Afterward, identity work and maintainer asset selection can proceed in parallel, but no agent should invent or adapt a logo while the asset decision is unresolved. Use this bounded split:

1. **Identity owner** — in a dedicated worktree and branch, owns `cli_agent.rs`, telemetry conversion, input-classifier keyword handling, explicit no-integration branches, and their unit tests. This owner does not edit bundled assets.
2. **Brand owner** — in a separate worktree and branch, records the maintainer-selected asset source and treatment, then adds only the accepted SVG and `Icon` registry mapping. If the decision is absent or ambiguous, this owner reports the open question and makes no asset commit.
3. **Integrator** — on the issue branch, reviews and combines the two non-overlapping commits, adds or adjusts the shared icon-resolver test, runs the focused suites and full presubmit, and records manual GUI evidence from the combined build.

The identity branch and maintainer asset-selection discussion may proceed concurrently after spec approval. GUI proof and final repository checks are sequential because they must exercise the exact combined head. Use one implementation PR for #14319 so reviewers can evaluate executable recognition and its accepted identity together.

## Risks and mitigations

### Product naming can drift

Older material calls the product Cortex Code, while current official documentation uses Snowflake CoCo and CoCo CLI; the executable remains `cortex`.

Mitigation: keep executable matching separate from the visible label, link the current official documentation in the PR, and recheck the name immediately before implementation review.

### An available image is not permission to ship a branded derivative

The Snowflake-Labs PNG is technically accessible, its repository license reserves trademark rights, and no official vector has been found. Those facts do not by themselves define the correct Warp asset treatment.

Mitigation: ask maintainers to select an acceptable source and treatment, document that decision, and do not let a contributor automatically trace or improvise the logo. Treat the officially published plugin color as evidence for the review, not an automatic implementation choice.

### A new enum variant can accidentally enable adjacent integrations

Several capability matches are exhaustive, while the current footer default enables most recognized agents. Adding only a command prefix could therefore opt CoCo into managed-input behavior that issue #14319 did not request. Independently, current generic CLI-agent context actions already append user-selected text to any active CLI agent's PTY.

Mitigation: add explicit `Coco` branches, keep footer, listener, plugin, hosted-harness, shell-mode, and skills integrations disabled, and test those negative capabilities. Explicitly preserve and test the generic, user-initiated PTY context route without adding automatic submission or CoCo-specific framing.

### Exact executable matching cannot authenticate the binary's publisher

Any executable named `cortex` can satisfy local basename detection. Warp does not currently verify the publisher of other built-in CLI-agent commands either.

Mitigation: preserve exact matching and avoid claims that Warp has authenticated the installed binary. The identity communicates which command Warp detected, not supply-chain verification.

### Additive telemetry can surprise downstream consumers

An exhaustive Rust conversion guarantees a local value but does not prove an external analytics schema accepts it.

Mitigation: confirm the additive value with the telemetry schema owner before release and retain the unknown serialized-name fallback for client compatibility.
