# TECH.md — Support Kiro CLI Agent Integration in Warp

Issue: https://github.com/warpdotdev/warp/issues/9066
Product spec: `specs/GH9066/product.md`

## Context

Warp's CLI agent integration is built around the `CLIAgent` enum
(`app/src/terminal/cli_agent.rs`). Every supported agent is a variant of that
enum. The enum drives:

- Command detection (matching the first token of a terminal command against
  `command_prefix()`).
- Display (logo icon, brand color, display name).
- Plugin management (install/update instructions, version checks, auto-install).
- Skill support (which `SkillProvider` variants are shown in the rich input
  slash menu).
- Telemetry (`CLIAgentType` in `app/src/server/telemetry/events.rs`).
- Shared session serialization (`to_serialized_name` / `from_serialized_name`).

Adding a new agent requires touching a fixed set of files, all following the
same pattern established by Claude, Codex, Gemini, Amp, and the others. No
changes to the session model, event system, rich input, or footer rendering are
needed — those are generic over `CLIAgent`.

Relevant files:

- `app/src/terminal/cli_agent.rs` — `CLIAgent` enum and all its `impl` blocks.
  Lines 116–350 (approximately). Every `match self` arm must be extended.
- `app/src/terminal/cli_agent_sessions/plugin_manager/mod.rs` — factory that
  maps `CLIAgent` variants to `CliAgentPluginManager` trait objects. Lines 1–30
  (module declarations) and the factory `match` (lines ~60–80).
- `app/src/terminal/cli_agent_sessions/plugin_manager/codex.rs` — reference
  implementation for a simple plugin manager (no auto-install, manual steps
  only). New `kiro.rs` follows this pattern.
- `app/src/server/telemetry/events.rs` — `CLIAgentType` enum at line 491.
- `crates/warp_features/src/lib.rs` — `FeatureFlag` enum and rollout lists
  (`DOGFOOD_FLAGS`, `PREVIEW_FLAGS`, `RELEASE_FLAGS`). New feature flag gates
  the Kiro variant until the plugin is ready for general availability.
- `app/src/features.rs` — re-export used by app code (`pub use
  warp_core::features::*;`) when importing `FeatureFlag`.
- `app/src/ui_components/icons.rs` (or the equivalent icon registry) — `Icon`
  enum. A `KiroLogo` variant is needed.
- `app/src/settings_view/ai_page.rs` — the file that renders the "Third party
  CLI agents" subpage (navigated to via Settings → Agents → Third party CLI
  agents in the UI; the sidebar umbrella label is "Agents", defined in
  `app/src/settings_view/mod.rs`). The agent-assignment dropdown is populated by
  iterating `all::<CLIAgent>()` (via `enum_iterator::Sequence`) at line ~2121,
  filtering out `CLIAgent::Unknown`. The current settings UI does not render a
  per-agent documentation link — entries are driven entirely by
  `agent.display_name()` and `agent.icon()`. The product spec behavior #8
  ("a link to the Kiro CLI documentation") therefore cannot be satisfied by the
  current settings UI without a UI change. **Resolution**: the documentation
  link requirement is deferred to a follow-up; the initial implementation adds
  Kiro to the dropdown only (consistent with all other agents). This file needs
  two explicit behavior guards: exclude `CLIAgent::Kiro` from dropdown items
  when the feature flag is disabled, and coerce persisted `CLIAgent::Kiro`
  assignments to "Other" in the selected label when the flag is disabled (see
  feature flag section below).

## Proposed changes

### 1. Add `CLIAgent::Kiro` to the enum and implement all match arms

In `app/src/terminal/cli_agent.rs`:

```rust
pub enum CLIAgent {
    Claude,
    Gemini,
    Codex,
    // ... existing variants ...
    Kiro,   // ← new
    Unknown,
}
```

Implement every exhaustive `match self` arm (exhaustive matching is required by
the codebase's coding standards — no `_` wildcards):

| Method | Value |
|--------|-------|
| `command_prefix` | `"kiro"` |
| `display_name` | `"Kiro"` |
| `icon` | `Some(Icon::KiroLogo)` |
| `brand_color` | `Some(KIRO_COLOR)` — see color note below |
| `brand_icon_color` | `ColorU::white()` (default) |
| `supported_skill_providers` | `&[SkillProvider::Agents]` |
| `skill_command_prefix` | `"/"` (default, no arm needed if using `_`) — but add explicit arm per exhaustive-match rule |
| `supports_bash_mode` | `false` (not confirmed supported; add to the `matches!` list if confirmed later) |

**Brand color**: Kiro's brand uses a blue-purple (`#5B4FE8` approximately, from
kiro.dev). Define a constant:

```rust
const KIRO_COLOR: ColorU = ColorU {
    r: 91,
    g: 79,
    b: 232,
    a: 255,
};
```

Confirm the exact hex against the Kiro brand guide before merging; the value
above is a reasonable starting point from the kiro.dev website.

### 2. Add `Icon::KiroLogo` to the icon registry

In `app/src/ui_components/icons.rs` (or wherever `Icon` is defined), add:

```rust
KiroLogo,
```

The icon asset (SVG or bundled image) must be added to `app/assets/` following
the same pattern as `ClaudeLogo`, `GeminiLogo`, etc. The asset path and
embedding macro call follow the existing pattern in the icon registry.

### 3. Add `CLIAgentType::Kiro` to the telemetry enum

In `app/src/server/telemetry/events.rs`, extend `CLIAgentType`:

```rust
pub enum CLIAgentType {
    // ... existing variants ...
    Kiro,
}
```

Update the conversion from `CLIAgent` to `CLIAgentType` (wherever that mapping
lives — typically a `From<CLIAgent>` impl or a match in the telemetry module) to
map `CLIAgent::Kiro => CLIAgentType::Kiro`.

### 4. Add a feature flag

In `crates/warp_features/src/lib.rs`, add:

```rust
pub enum FeatureFlag {
    // ... existing variants ...
    KiroCLIAgent,
}
```

Gate the `CLIAgent::Kiro` variant's detection in the command-matching path
behind `FeatureFlag::KiroCLIAgent.is_enabled()`. This allows the feature to be
enabled in dogfood/preview before stable release, consistent with how other
agents were rolled out. Add `FeatureFlag::KiroCLIAgent` to `DOGFOOD_FLAGS`
initially.

**Important — all enum-iteration surfaces must be gated, not just command
detection.** Adding `CLIAgent::Kiro` to the enum automatically exposes it
through every consumer of `enum_iterator::all::<CLIAgent>()`. There are three
additional enum-iteration surfaces that must be gated, plus the plugin-manager
factory, plus one deserialization path that bypasses enum iteration entirely:

1. **`CLIAgent::detect()` in `app/src/terminal/cli_agent.rs`** — the
   `enum_iterator::all::<CLIAgent>()` loop that matches commands. Add a
   `.filter()` step to exclude `CLIAgent::Kiro` when the flag is off:

   ```rust
   enum_iterator::all::<CLIAgent>()
       .filter(|agent| !matches!(agent, CLIAgent::Unknown))
       .filter(|agent| {
           !matches!(agent, CLIAgent::Kiro)
               || FeatureFlag::KiroCLIAgent.is_enabled()
       })
       .find(|agent| { ... })
   ```

2. **Settings dropdown in `app/src/settings_view/ai_page.rs`** — the
   `for agent in all::<CLIAgent>()` loop at line ~2121 that populates the
   agent-assignment dropdown. Add the same guard and disabled-state selection
   fallback:

   ```rust
   for agent in all::<CLIAgent>() {
       if matches!(agent, CLIAgent::Unknown) {
           continue;
       }
       if matches!(agent, CLIAgent::Kiro) && !FeatureFlag::KiroCLIAgent.is_enabled() {
           continue;
       }
       // ... existing item construction
   }
   let selected_name = if matches!(current_agent, CLIAgent::Unknown)
       || (matches!(current_agent, CLIAgent::Kiro)
           && !FeatureFlag::KiroCLIAgent.is_enabled())
   {
       "Other"
   } else {
       current_agent.display_name()
   };
   ```

3. **`resolve_agent()` in `app/src/terminal/cli_agent_sessions/event/v1.rs`**
   — plugin event parsing also iterates `all::<CLIAgent>()`. Apply the same
   feature-flag guard so disabled variants do not resolve from incoming event
   payloads:

   ```rust
   enum_iterator::all::<CLIAgent>()
       .filter(|agent| !matches!(agent, CLIAgent::Unknown))
       .filter(|agent| {
           !matches!(agent, CLIAgent::Kiro)
               || FeatureFlag::KiroCLIAgent.is_enabled()
       })
       .find(|agent| agent.command_prefix() == incoming_agent_name)
   ```

4. **`plugin_manager_for_with_shell()` in
   `app/src/terminal/cli_agent_sessions/plugin_manager/mod.rs`** — the factory
   match already uses per-agent feature flag guards (e.g.
   `CLIAgent::Codex if FeatureFlag::CodexNotifications.is_enabled() && ...`).
   Follow the same pattern for Kiro:

   ```rust
   CLIAgent::Kiro
       if FeatureFlag::KiroCLIAgent.is_enabled()
           && FeatureFlag::HOANotifications.is_enabled() =>
   {
       Some(Box::new(kiro::KiroPluginManager))
   }
   ```

   Both guards are required: `HOANotifications` is the global kill switch that
   disables all non-Claude notification plugin managers (see `mod.rs` — every
   agent except Claude checks `FeatureFlag::HOANotifications.is_enabled()`).
   The catch-all `CLIAgent::Kiro` arm (either flag disabled) falls through to
   `None`, consistent with how other agents behave when their flag is off.

5. **`CompiledCommandsForCodingAgentToolbar::matched_agent` (custom-command
   deserialization path)** — persisted custom-command mappings are serialized
   as `"Kiro"` via `CLIAgent::to_serialized_name()` and deserialized back
   through `CLIAgent::from_serialized_name()`. This path does not go through
   `enum_iterator::all::<CLIAgent>()`, so the filters above do not protect it.
   A user who saved a Kiro mapping while the flag was on and then had the flag
   turned off would otherwise still activate `CLIAgent::Kiro` via their custom
   regex — bypassing the flag entirely. Add a flag guard at the
   `matched_agent` call site (or at the point where the deserialized agent is
   consumed to start a session) so that a resolved `CLIAgent::Kiro` is treated
   as `CLIAgent::Unknown` when `FeatureFlag::KiroCLIAgent` is disabled:

   ```rust
   let agent = CompiledCommandsForCodingAgentToolbar::matched_agent(...);
   let agent = if matches!(agent, CLIAgent::Kiro)
       && !FeatureFlag::KiroCLIAgent.is_enabled()
   {
       CLIAgent::Unknown
   } else {
       agent
   };
   ```

   This satisfies product behavior #8: persisted Kiro mappings display as
   "Other" and do not activate Kiro while the flag is off.

### 5. Add the Kiro plugin manager

Create `app/src/terminal/cli_agent_sessions/plugin_manager/kiro.rs`:

```rust
use std::sync::LazyLock;

use async_trait::async_trait;

use super::{CliAgentPluginManager, PluginInstructionStep, PluginInstructions};

pub(super) struct KiroPluginManager;

#[async_trait]
impl CliAgentPluginManager for KiroPluginManager {
    fn minimum_plugin_version(&self) -> &'static str {
        "0.0.0"  // No versioned plugin yet; version gating is inactive
    }

    fn can_auto_install(&self) -> bool {
        false  // Manual install until the plugin is stable
    }

    fn supports_update(&self) -> bool {
        false
    }

    fn install_instructions(&self) -> &'static PluginInstructions {
        &INSTALL_INSTRUCTIONS
    }

    fn update_instructions(&self) -> &'static PluginInstructions {
        &EMPTY_INSTRUCTIONS
    }
}

static INSTALL_INSTRUCTIONS: LazyLock<PluginInstructions> = LazyLock::new(|| {
    PluginInstructions {
        title: "Enable Warp Integration for Kiro",
        subtitle: "Install the Kiro CLI and enable the Warp plugin to get real-time status tracking while you work.",
        steps: &[
            PluginInstructionStep {
                description: "Install the Kiro CLI. Follow the instructions on the Kiro download page.",
                command: "",
                executable: false,
                link: Some("https://kiro.dev/downloads/"),
            },
            PluginInstructionStep {
                description: "Enable the Warp plugin in your Kiro configuration so that Kiro emits structured status events that Warp can display. See the Kiro documentation for the exact config key.",
                command: "",
                executable: false,
                link: Some("https://kiro.dev/docs/"),
            },
        ],
        post_install_notes: &[
            "Restart your terminal session after installation.",
        ],
    }
});

static EMPTY_INSTRUCTIONS: LazyLock<PluginInstructions> =
    LazyLock::new(|| PluginInstructions {
        title: "",
        subtitle: "",
        steps: &[],
        post_install_notes: &[],
    });
```

Create the corresponding test file
`app/src/terminal/cli_agent_sessions/plugin_manager/kiro_tests.rs` following
the pattern in `codex_tests.rs`.

In `app/src/terminal/cli_agent_sessions/plugin_manager/mod.rs`, add:

```rust
pub(crate) mod kiro;
```

And extend the factory `match` in `plugin_manager_for_with_shell()` to include
the Kiro arms, following the same pattern as Codex and Gemini (both the
agent-specific flag and the HOA kill switch must be enabled; the disabled arm
falls through to the existing catch-all `None` list):

```rust
CLIAgent::Kiro
    if FeatureFlag::KiroCLIAgent.is_enabled()
        && FeatureFlag::HOANotifications.is_enabled() =>
{
    Some(Box::new(kiro::KiroPluginManager))
}
```

Add `CLIAgent::Kiro` to the existing catch-all `None` arm alongside the other
agents that return `None` when their flag is off:

```rust
CLIAgent::OpenCode
| CLIAgent::Codex
| CLIAgent::Gemini
| CLIAgent::Amp
| CLIAgent::Droid
| CLIAgent::Copilot
| CLIAgent::Pi
| CLIAgent::Auggie
| CLIAgent::CursorCli
| CLIAgent::Goose
| CLIAgent::Kiro      // ← add here (flag-disabled fallthrough)
| CLIAgent::Unknown => None,
```

**Note on the Warp plugin protocol**: The Kiro CLI must emit structured JSON
events to stdout (the same `SessionStart`, `PromptSubmit`, `Stop`,
`PermissionRequest`, `QuestionAsked`, `PermissionReplied`, `ToolComplete`,
`IdlePrompt` event types defined in
`app/src/terminal/cli_agent_sessions/event/`) for status tracking to work. If
Kiro CLI does not yet support this protocol, the plugin manager should set
`can_auto_install = false` and `minimum_plugin_version = "0.0.0"` (as above),
and the install instructions should be updated once the protocol is supported.
The footer and rich input work without the plugin; only status tracking requires
it.

## End-to-end flow

1. User runs `kiro` in a Warp terminal pane.
2. `FeatureFlag::KiroCLIAgent.is_enabled()` → true (in dogfood/preview).
3. Command detection matches `"kiro"` → `CLIAgent::Kiro`.
4. `CLIAgentSessionsModel::set_session` creates a new session with
   `agent: CLIAgent::Kiro`, `status: InProgress`.
5. The terminal view renders the Kiro-branded footer (logo from `Icon::KiroLogo`,
   brand color `KIRO_COLOR`).
6. If the Warp plugin is installed and emitting events, `CLIAgentSessionListener`
   parses them and calls `CLIAgentSessionsModel::update_from_event`, updating
   status in real time.
7. If the plugin is not installed, the footer shows the install instructions
   pane from `KiroPluginManager::install_instructions()`.
8. Ctrl-G opens the rich input composer; submitting sends the prompt to the PTY.
9. When `kiro` exits, `remove_session` is called and the footer disappears.

## Testing and validation

- `cargo fmt` and `cargo clippy --workspace --all-targets --all-features --tests
  -- -D warnings` must pass.
- `cargo nextest run --no-fail-fast --workspace --exclude command-signatures-v2`
  must pass. In particular:
  - `CLIAgent::Kiro` must be covered by any existing exhaustive-match tests in
    `app/src/terminal/cli_agent_tests.rs` (if present) or
    `app/src/terminal/cli_agent_sessions/mod_tests.rs`.
  - `kiro_tests.rs` must include at minimum smoke tests that
    `KiroPluginManager::install_instructions()` returns a non-empty title and at
    least one step, that `supports_update()` returns `false`, and that
    `update_instructions()` returns empty content — mirroring `codex_tests.rs`.
  - The `CLIAgentType` conversion test (if one exists) must cover `Kiro`.

Behavior-to-verification mapping (from `product.md`):

| Behavior | Verification |
|----------|-------------|
| #1 — detection | Run `kiro` in a Warp pane with the feature flag enabled; confirm `CLIAgentSessionsModel` has an active session. |
| #2 — footer | Confirm the Kiro logo and brand color appear in the footer. |
| #3, #4 — rich input | Press Ctrl-G; type a prompt; confirm it is sent to the PTY. |
| #5 — status tracking | With the plugin installed and emitting events, confirm the footer status updates. |
| #6 — install instructions | With the plugin absent (local session), confirm the install instructions pane renders with the correct steps. |
| #7 — update instructions | `supports_update()` returns `false` and `minimum_plugin_version` is `"0.0.0"`; the outdated-plugin path is unreachable in this release. Confirm the update chip never appears. Full update support (version detection via `is_installed()`/`needs_update()` overrides and a real minimum version) is deferred to a follow-up once a versioned Kiro Warp plugin exists. |
| #10 — remote session | In a remote SSH pane, confirm the footer appears and the install/update instructions pane renders (same as a local session with no plugin installed). |
| #8 — settings page | Open Settings → Agents → Third party CLI agents; with the flag enabled confirm Kiro appears in the list, and with the flag disabled confirm Kiro is hidden and persisted Kiro mappings display as "Other". |
| #9 — telemetry | Confirm `CLIAgentType::Kiro` events are emitted on session start/end. |
| #11 — shared sessions | Confirm `CLIAgent::Kiro.to_serialized_name() == "Kiro"` and `CLIAgent::from_serialized_name("Kiro") == CLIAgent::Kiro`. |
| #12 — not Unknown | Confirm `kiro` does not fall through to `CLIAgent::Unknown`. |
| #13 — no regressions | Run the full test suite; confirm no existing agent tests fail. |

## Risks and mitigations

- **Plugin protocol not yet supported by Kiro CLI**: The footer and rich input
  work without the plugin. Status tracking degrades gracefully to "no status"
  rather than breaking. The install instructions pane is shown but can be
  dismissed. This is the same degraded-mode behavior as other agents before
  their plugins were available.
- **Brand color accuracy**: The `KIRO_COLOR` constant should be confirmed
  against the official Kiro brand guide before the PR is merged. An incorrect
  color is a cosmetic issue only and does not affect functionality.
- **Exhaustive match compile errors**: Adding `CLIAgent::Kiro` will cause
  compile errors at every `match self` that lacks a `Kiro` arm. This is
  intentional — the compiler enforces completeness. All arms must be added
  before the build passes.
- **Feature flag rollout**: Starting in `DOGFOOD_FLAGS` means the feature is
  only visible to internal users until explicitly promoted to `PREVIEW_FLAGS`
  and then `RELEASE_FLAGS`. This is the standard rollout path.

## Follow-ups

- Once the Kiro CLI publishes a Warp plugin that emits the structured event
  protocol, update `minimum_plugin_version`, `can_auto_install`, and
  `install_instructions` in `kiro.rs` to enable auto-install and real-time
  status tracking.
- Once a versioned Kiro Warp plugin exists, implement full update support in
  `KiroPluginManager`: set `minimum_plugin_version` to the actual minimum,
  override `is_installed()` and `needs_update()` with real version detection,
  and set `supports_update()` to `true`. This unblocks product behavior #7
  (the outdated-plugin update instructions pane).
- Promote `FeatureFlag::KiroCLIAgent` from `DOGFOOD_FLAGS` to `PREVIEW_FLAGS`
  and then `RELEASE_FLAGS` once the integration is validated.
- If Kiro CLI adds support for bash mode (`!` prefix in rich input), add
  `CLIAgent::Kiro` to the `supports_bash_mode` match arm.
- Consider adding `SkillProvider::Kiro` if Kiro CLI develops a native skill
  format distinct from the generic `SkillProvider::Agents` format.
