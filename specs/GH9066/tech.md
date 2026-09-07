# Tech Spec: Support Kiro CLI agent integration

**Issue:** [warpdotdev/warp#9066](https://github.com/warpdotdev/warp/issues/9066)

**Product spec:** [`specs/GH9066/product.md`](product.md)

**Prior spec:** [warpdotdev/warp#12387](https://github.com/warpdotdev/warp/pull/12387)

## Context

Warp models each recognized terminal agent with `CLIAgent`. This enum supplies the
agent name, executable names, icon, colors, skill providers, serialization, and
telemetry conversion.

The existing footer, Rich Input, settings, and shared-session systems consume this
identity. Kiro support therefore needs a new enum variant and exhaustive updates
to these consumers. It does not need a separate UI implementation.

Relevant current code:

- `app/src/terminal/cli_agent.rs:146-450` defines agent identities, command
  detection, display properties, skill providers, and input capabilities.
- `app/src/terminal/cli_agent.rs:626-648` converts an agent identity to its
  telemetry value.
- `app/src/server/telemetry/events.rs:453-473` defines `CLIAgentType`.
- `crates/warp_core/src/ui/icons.rs:260-300,610-650` defines icon identities and
  bundled asset paths.
- `crates/ai/src/skills/skill_provider.rs:31-43,77-103,105-154` defines Kiro as a
  skill provider and maps `.kiro/skills` paths.
- `app/src/terminal/view/use_agent_footer/mod.rs:101-144` selects the PTY
  submission strategy for each agent.
- `app/src/terminal/cli_agent_sessions/listener/mod.rs:60-90` selects session event
  handlers.
- `app/src/terminal/cli_agent_sessions/plugin_manager/mod.rs:250-305` selects
  plugin managers.
- `app/src/settings_view/ai_page.rs:3584-3657` builds the known-agent selector for
  custom command mappings.
- `app/src/terminal/local_tty/terminal_view_adaptor.rs` and
  `app/src/terminal/shared_session/` serialize CLI-agent identities for shared
  sessions.
- `app/src/integration_testing/input/step.rs` provides Rich Input test steps.
- `crates/integration/src/test/kiro_cli.rs` covers the Kiro Rich Input and
  vertical-tabs flow.

The closed spec PR #12387 proposed a Kiro plugin manager, status events, install
instructions, and a rollout flag. Kiro does not currently expose the compatible
plugin integration that proposal requires. This specification limits the initial
change to supported behavior.

## Proposed changes

### 1. Add the Kiro agent identity

Add `CLIAgent::Kiro` to `app/src/terminal/cli_agent.rs`. Define these properties:

| Property | Value |
| --- | --- |
| Canonical executable | `kiro-cli` |
| Executable names | `kiro-cli`, `kiro`, `kiro-cli-chat`, `kiro-cli-term` |
| Display name | `Kiro` |
| Icon | `Icon::KiroLogo` |
| Brand color | `ColorU::new(192, 156, 255, 255)` |
| Brand icon color | black |
| Skill providers | `SkillProvider::Kiro` |
| Skill command prefix | `/` |
| Rich Input bash mode | disabled |
| CLI-agent footer | enabled |

The existing `CLIAgent::detect` iteration then recognizes Kiro through
`matches_command`. Existing parsing continues to handle executable basenames,
paths, aliases, arguments, and shell environment assignments.

The enum derives Serde traits and `Sequence`. As a result, the existing
serialization functions use `Kiro`, and known-agent selectors include the new
variant without separate settings code.

### 2. Add Kiro branding

Add `Icon::KiroLogo` to `crates/warp_core/src/ui/icons.rs`. Map it to
`app/assets/bundled/svg/kiro.svg`.

Use the Kiro logo for both `CLIAgent::Kiro` and `SkillProvider::Kiro`. Use a black
foreground on the light Kiro brand color to preserve icon contrast.

The asset must remain a monochrome SVG because callers control its fill color.
The source and license of the asset must be acceptable for repository use.

### 3. Connect Kiro skills and Rich Input

Return only `SkillProvider::Kiro` from
`CLIAgent::Kiro.supported_skill_providers()`. The existing skill index reads Kiro
skills from these paths:

- `~/.kiro/skills`
- `<project>/.kiro/skills`

Use `RichInputSubmitStrategy::DelayedEnter` for Kiro. This strategy writes the
prompt text to the PTY and writes carriage return after the standard short delay.
It avoids an input submission race without changing Kiro CLI.

Do not add Kiro to `supports_bash_mode`. Kiro bash-mode compatibility is not
established.

### 4. Preserve telemetry and shared-session identity

Add `CLIAgentType::Kiro` to the telemetry enum. Map `CLIAgent::Kiro` to this value
in the exhaustive conversion.

The derived Serde representation supplies `Kiro` to existing shared-session
writers. Existing readers reconstruct `CLIAgent::Kiro` through
`from_serialized_name`.

### 5. Keep plugin behavior explicit

Add Kiro to the no-manager arm in `plugin_manager_for_with_shell`. Add Kiro to the
no-listener arm in `create_handler`.

These exhaustive arms make the current limitation explicit. They prevent the code
from implying that Kiro has plugin installation, update, or event-status support.
A later contribution can add those features after Kiro exposes a compatible
integration.

The generic version-1 event parser can recognize a Kiro executable name because it
iterates all agent identities. Without a Kiro session handler, these events do not
enable a Kiro-specific status integration.

### 6. Add focused automated coverage

Extend `app/src/terminal/cli_agent_tests.rs` with:

- Detection cases for all supported executable names.
- Assertions for the Kiro name, icon, colors, skill provider, footer, input mode,
  and telemetry conversion.
- Existing all-variant serialization coverage for the Kiro identity.

Extend `app/src/terminal/view_tests.rs` to assert the Kiro Rich Input placeholder.

Generalize the integration helper in
`app/src/integration_testing/input/step.rs` so a test can open Rich Input for a
specified agent. Add assertions for the active agent, open input state, and
configured placeholder.

Add `crates/integration/src/test/kiro_cli.rs`. The real-display test opens Kiro
Rich Input, captures its branding, opens vertical tabs, and captures the pane
identity there.

## End-to-end flow

1. The user starts a supported Kiro executable in a terminal pane.
2. `CLIAgent::detect` returns `CLIAgent::Kiro`.
3. The CLI-agent session model stores the Kiro identity for that pane.
4. The footer and vertical-tabs UI read the Kiro name, icon, and colors.
5. Rich Input filters slash commands to Kiro skills.
6. Rich Input writes the prompt and delayed Enter to the Kiro PTY.
7. Telemetry and shared-session code serialize the Kiro identity.
8. The session ends through the existing command and session lifecycle.

## Testing and validation

| Product invariants | Verification |
| --- | --- |
| 1-3 | Unit tests call `CLIAgent::detect` for each supported Kiro executable name. |
| 4 | Unit property tests and a real-display screenshot cover the name, icon, and colors. |
| 5 | The real-display integration test opens vertical tabs and asserts that the panel exists for the active Kiro session. |
| 6 | A view test checks `Enter prompt for Kiro...`. The integration test asserts that Rich Input is open. |
| 7 | A unit property test checks `SkillProvider::Kiro`. Existing provider tests cover `.kiro/skills` discovery. |
| 8-9 | Manual testing covers prompt submission. A unit property test makes sure that bash mode remains disabled. |
| 10 | Unit tests check telemetry conversion. The existing all-variant round-trip test covers serialization. |
| 11 | The enum-driven settings selector includes every known agent except `Unknown`. Focused manual testing covers Kiro. |
| 12 | Exhaustive plugin-manager and listener matches return no Kiro integration. |
| 13 | Repository formatting, Clippy, unit tests, integration tests, and presubmit protect existing behavior. |

Run these required checks:

```bash
./script/presubmit
```

Manually run Warp with `./script/run`. Start Kiro CLI, open Rich Input, submit a
prompt, and inspect the vertical-tabs panel.

Capture before and after screenshots for the footer, Rich Input, and vertical-tabs
panel.

## Risks and mitigations

- Kiro can change its executable names. Keep detection names covered by tests and
  remove unsupported aliases after maintainer review.
- An incorrect or unlicensed logo can block the visual change. Record the asset
  source and replace it if the maintainers request another official asset.
- Kiro input behavior can differ across releases. Manual prompt submission and the
  real-display test reduce this risk.
- Adding an enum variant exposes Kiro in all known-agent iterations. Exhaustive
  matches and serialization tests make these surfaces visible during review.
- Kiro status can appear static without event support. The initial integration
  does not claim plugin status support.

## Open questions

- Does the initial release require a feature flag for staged rollout?
- Which Kiro source supplies the approved logo and exact brand color?
- Are `kiro`, `kiro-cli-chat`, and `kiro-cli-term` supported public launchers?
