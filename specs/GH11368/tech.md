# Technical Spec: Support 'agy' (Antigravity) CLI Agent in Warp

See `specs/GH11368/product.md` for the product spec.

**Issue:** [warpdotdev/warp#11368](https://github.com/warpdotdev/warp/issues/11368)

## 1. Problem

Warp lacks native configuration, detection, and notification support for the `agy` (Antigravity) CLI agent. Currently, any structured notifications sent via the `warp://cli-agent` OSC 777 protocol from an `agy` session are ignored, and running the agent does not trigger Agent Mode.

To resolve this, we must wire `CLIAgent::Antigravity` into the terminal's agent detection, branding/layout config, OSC 777 notification listener, and plugin manager system.

## 2. Relevant Code

- `app/src/terminal/cli_agent.rs` — `CLIAgent` enum with all identity methods: `command_prefix()`, `to_serialized_name()`, `from_serialized_name()`, `from_harness()`, `display_name()`, `icon()`, `supported_skill_providers()`, `skill_command_prefix()`, `supports_bash_mode()`, `brand_color()`, `brand_icon_color()`, `detect()`, and the `From<CLIAgent> for CLIAgentType` telemetry conversion.
- `crates/input_classifier/src/util.rs` — `ONE_OFF_SHELL_COMMAND_KEYWORDS` that determine shell-vs-natural-language classification.
- `app/src/terminal/cli_agent_sessions/listener/mod.rs` — `is_agent_supported()` and `create_handler()` to manage session notifications.
- `crates/warp_core/src/ui/icons.rs` — `Icon` enum register and SVG asset mappings.
- `crates/warp_features/src/lib.rs` — feature flag declarations.
- `app/src/terminal/cli_agent_sessions/plugin_manager/mod.rs` — plugin manager registration and factory (feature-flag gated).
- `crates/ai/src/skills/skill_provider.rs` — `SkillProvider` enum and provider folder definitions.
- `crates/ai/src/skills/conversion.rs` — proto-to-provider mappings.
- `app/src/server/telemetry.rs` — `CLIAgentType` telemetry enum.

## 3. Proposed Changes

### 3a. Add Identity and Branding (`app/src/terminal/cli_agent.rs`)

1. Add `Antigravity` to the `CLIAgent` enum (before `Unknown`):
   ```rust
   pub enum CLIAgent {
       ...
       Goose,
       Hermes,
       Vibe,
       Antigravity,
       Unknown,
   }
   ```
2. The new `Antigravity` variant must appear in every match arm across the `CLIAgent` impl. The following is an exhaustive list of every method and the required behavior:
   - `command_prefix()`: returns `"agy"`.
   - `to_serialized_name()`: **No explicit match arm needed.** This method uses `serde_json::to_value(self)` which serializes the variant name as `"Antigravity"` via the derive macro.
   - `from_serialized_name()`: **No explicit match arm needed.** This method uses `serde_json::from_value()` which deserializes `"Antigravity"` back to `CLIAgent::Antigravity` via the derive macro. Unrecognized names fall back to `CLIAgent::Unknown`.
   - `from_harness()`: **No changes needed.** The external `Harness` proto enum (in `warp-proto-apis`) does not yet include an `Antigravity` variant. The existing exhaustive match covers all current `Harness` variants. When the proto is updated in a future milestone (Oz harness support), a new arm mapping `Harness::Antigravity => Some(CLIAgent::Antigravity)` will be added. This is consistent with the product spec non-goal of excluding Oz harness support.
   - `display_name()`: returns `"Antigravity"`.
   - `icon()`: returns `Some(Icon::AntigravityLogo)`.
   - `supported_skill_providers()`: returns `&[SkillProvider::Agents, SkillProvider::Antigravity]`.
   - `skill_command_prefix()`: **No explicit match arm needed.** The existing wildcard `_ => "/"` covers `Antigravity`.
   - `supports_bash_mode()`: **No explicit match arm needed.** The existing `matches!()` only includes `Claude | Codex | OpenCode`; `Antigravity` correctly falls through to `false`.
   - `brand_color()`: returns `Some(ColorU::from_rgb(0x63, 0x66, 0xF1))` (Indigo brand color). Add an `ANTIGRAVITY_INDIGO` constant at module level.
   - `brand_icon_color()`: **No explicit match arm needed.** The existing wildcard `_ => ColorU::white()` covers `Antigravity`. White provides good contrast on the Indigo background.
   - `detect()`: **No explicit change needed.** This method iterates over all `CLIAgent` variants (via `enum_iterator::Sequence`) and checks `command_prefix()`. Adding `Antigravity` to the enum with prefix `"agy"` is sufficient.
3. Add `CLIAgent::Antigravity => CLIAgentType::Antigravity` to the `From<CLIAgent> for CLIAgentType` impl. This requires adding an `Antigravity` variant to the `CLIAgentType` telemetry enum in `app/src/server/telemetry.rs`.

### 3b. Register Command Classifier (`crates/input_classifier/src/util.rs`)

Add `"agy"` to the static `ONE_OFF_SHELL_COMMAND_KEYWORDS` hashset:
```rust
static ref ONE_OFF_SHELL_COMMAND_KEYWORDS: HashSet<&'static str> = HashSet::from([
    "#", "echo", "man", "sudo", "claude", "codex", "gemini", "agy"
]);
```
This change is **unconditional** (not feature-flag gated), matching the established pattern for all CLI agents. The classifier must always recognize `agy` as a shell command regardless of whether notifications are enabled.

### 3c. Wire Notification Listener (`app/src/terminal/cli_agent_sessions/listener/mod.rs`)

1. Update `is_agent_supported()` to include `CLIAgent::Antigravity`:
   ```rust
   pub fn is_agent_supported(agent: &CLIAgent) -> bool {
       matches!(
           agent,
           CLIAgent::Claude
               | CLIAgent::OpenCode
               | CLIAgent::Codex
               | CLIAgent::Gemini
               | CLIAgent::Auggie
               | CLIAgent::Pi
               | CLIAgent::Antigravity
       )
   }
   ```
2. Update `create_handler()` to map `CLIAgent::Antigravity` to `DefaultSessionListener`:
   ```rust
   CLIAgent::Claude
   | CLIAgent::OpenCode
   | CLIAgent::Gemini
   | CLIAgent::Auggie
   | CLIAgent::Pi
   | CLIAgent::Antigravity => Some(Box::new(DefaultSessionListener)),
   ```

These changes are **unconditional** (not feature-flag gated), matching the established pattern. The listener and handler are always wired; the feature flag only controls plugin manager instantiation (see 3f).

### 3d. Add SVG Asset and Icon Registry (`crates/warp_core/src/ui/icons.rs`)

1. Add `AntigravityLogo` to the `Icon` enum.
2. Map it to the SVG path in the match expression inside `Icon::svg_path()`:
   ```rust
   Icon::AntigravityLogo => "bundled/svg/antigravity_cli.svg",
   ```
3. Add the SVG asset file `app/assets/bundled/svg/antigravity_cli.svg`.

### 3e. Add Feature Flag (`crates/warp_features/src/lib.rs`)

1. Add `AntigravityNotifications` to the `FeatureFlag` enum.
2. Wire it into the default dogfood or beta flag sets.
3. This flag gates **only** plugin manager instantiation (section 3f), consistent with the existing pattern used by `GeminiNotifications`, `CodexNotifications`, and `OpenCodeNotifications`. Command detection (3b) and listener wiring (3c) are unconditional.

### 3f. Add Plugin Manager (`app/src/terminal/cli_agent_sessions/plugin_manager/`)

1. Add `pub(crate) mod antigravity;` to `plugin_manager/mod.rs`.
2. Create `plugin_manager/antigravity.rs` with `AntigravityPluginManager` implementing `CliAgentPluginManager`, following the same structure as `GeminiPluginManager`:
   - `EXTENSION_REPO`: `"https://github.com/warpdotdev/agy-warp"`.
   - `EXTENSION_NAME`: `"agy-warp"`.
   - `MINIMUM_PLUGIN_VERSION`: `"1.0.0"`.
   - `is_installed()`: checks if `~/.antigravitycli/extensions/agy-warp/agy-extension.json` exists and is valid JSON.
   - `needs_update()`: compares the on-disk manifest `version` with `MINIMUM_PLUGIN_VERSION` using `compare_versions()`.
   - `install()`: runs `agy extensions install https://github.com/warpdotdev/agy-warp`.
   - `update()`: runs `agy extensions update agy-warp`, then performs a post-update version verification (reading the on-disk manifest to confirm the version changed). If the version is still outdated, returns `PluginInstallError`.
   - `install_instructions()` / `update_instructions()`: static `PluginInstructions` structs with title, subtitle, steps, and post-install notes.
3. Register `CLIAgent::Antigravity` in `plugin_manager_for_with_shell()` in `plugin_manager/mod.rs`:
   ```rust
   CLIAgent::Antigravity
       if FeatureFlag::AntigravityNotifications.is_enabled()
           && FeatureFlag::HOANotifications.is_enabled() =>
   {
       Some(Box::new(AntigravityPluginManager::new(
           shell_path,
           shell_type,
           path_env_var,
       )))
   }
   ```
4. Add `CLIAgent::Antigravity` to the existing fallthrough arm that returns `None` (for when the feature flag is disabled or other unsupported agents). This arm currently lists `CLIAgent::OpenCode | CLIAgent::Codex | CLIAgent::Gemini | ... | CLIAgent::Unknown => None`.

### 3g. Skill Provider Setup (`crates/ai/src/skills/`)

1. Register `Antigravity` in `SkillProvider` (`crates/ai/src/skills/skill_provider.rs`).
2. Add it to `SKILL_PROVIDER_DEFINITIONS`:
   ```rust
   SkillProviderDefinition {
       provider: SkillProvider::Antigravity,
       skills_path: PathBuf::from(".antigravitycli").join("skills"),
   }
   ```
3. Map it in `crates/ai/src/skills/conversion.rs` (`From<SkillProvider>` and `convert_provider` conversions). Since `warp_multi_agent_api` is imported from `warp-proto-apis`, if the proto does not yet contain `Antigravity` as a provider type, the implementation-safe behavior is to fall back by mapping `SkillProvider::Antigravity` to the generic `Agents` proto enum variant. This ensures the `From<SkillProvider>` conversion remains infallible (a `From` impl in Rust cannot return `None`).

## 4. End-to-End Flow

1. User types `agy` in their terminal and hits Enter.
2. The classifier marks it as a command execution, bypassing natural language matching.
3. The shell starts the `agy` process.
4. If the plugin isn't present and `AntigravityNotifications` is enabled, Warp detects this and displays a green install chip in the footer.
5. Clicking the chip executes the extension install command in the background.
6. Once the extension is active, it emits `warp://cli-agent` OSC 777 sequences.
7. Warp captures the sequences and initiates the Agent Mode layouts and toolbar, with custom branding and icons.

## 5. Testing and Validation

- **Unit tests**:
  - Add tests in `plugin_manager/antigravity_tests.rs` verifying `needs_update`, `is_installed`, post-update version verification, and installation instruction steps.
  - Verify CLI agent detection works for prefix `agy` in `cli_agent_tests.rs`.
  - Validate that `DefaultSessionListener` correctly processes OSC 777 notifications and constructs the session-model data flow for `agent: "agy"`.
  - Verify that when `AntigravityNotifications` is disabled, `plugin_manager_for_with_shell()` returns `None` for `CLIAgent::Antigravity`, while `is_agent_supported()` still returns `true`.
- **Manual Verification**: Run Warp locally, trigger `agy`, and verify Agent Mode transition and notification streaming.
