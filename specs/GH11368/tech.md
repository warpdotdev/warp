# Technical Spec: Support 'agy' (Antigravity) CLI Agent in Warp

See `specs/GH11368/product.md` for the product spec.

**Issue:** [warpdotdev/warp#11368](https://github.com/warpdotdev/warp/issues/11368)

## 1. Problem

Warp lacks native configuration, detection, and notification support for the `agy` (Antigravity) CLI agent. Currently, any structured notifications sent via the `warp://cli-agent` OSC 777 protocol from an `agy` session are ignored, and running the agent does not trigger Agent Mode.

To resolve this, we must wire `CLIAgent::Antigravity` into the terminal's agent detection, branding/layout config, OSC 777 notification listener, and plugin manager system.

## 2. Relevant Code

- `app/src/terminal/cli_agent.rs` — `CLIAgent` enum, command prefix mapping, display names, brand color, and logo mappings.
- `crates/input_classifier/src/util.rs` — `ONE_OFF_SHELL_COMMAND_KEYWORDS` that determine shell-vs-natural-language classification.
- `app/src/terminal/cli_agent_sessions/listener/mod.rs` — `is_agent_supported()` and `create_handler()` to manage session notifications.
- `crates/warp_core/src/ui/icons.rs` — `Icon` enum register and SVG asset mappings.
- `crates/warp_features/src/lib.rs` — feature flag declarations.
- `app/src/terminal/cli_agent_sessions/plugin_manager/mod.rs` — plugin manager registration and factory.
- `crates/ai/src/skills/skill_provider.rs` — `SkillProvider` enum and provider folder definitions.
- `crates/ai/src/skills/conversion.rs` — proto-to-provider mappings.

## 3. Proposed Changes

### 3a. Add Identity and Branding (`app/src/terminal/cli_agent.rs`)

1. Add `Antigravity` to the `CLIAgent` enum:
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
2. Map details in `CLIAgent` implementation methods:
   - `command_prefix()`: returns `"agy"`.
   - `display_name()`: returns `"Antigravity"`.
   - `icon()`: returns `Some(Icon::AntigravityLogo)`.
   - `brand_color()`: returns `Some(ColorU::from_rgb(0x63, 0x66, 0xF1))` (Indigo brand color).
   - `supported_skill_providers()`: returns `&[SkillProvider::Agents, SkillProvider::Antigravity]`.

### 3b. Register Command Classifier (`crates/input_classifier/src/util.rs`)

Add `"agy"` to the static `ONE_OFF_SHELL_COMMAND_KEYWORDS` hashset:
```rust
static ref ONE_OFF_SHELL_COMMAND_KEYWORDS: HashSet<&'static str> = HashSet::from([
    "#", "echo", "man", "sudo", "claude", "codex", "gemini", "agy"
]);
```
This guarantees that launching the `agy` command is handled as a standard shell invocation and is not classified as natural language input.

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
3. Ensure this flag strictly gates **all** `agy` integrations codebase-wide, including command classification (`input_classifier`), notification listener wiring, and plugin manager instantiations, to guarantee the feature can be completely disabled.

### 3f. Add Plugin Manager (`app/src/terminal/cli_agent_sessions/plugin_manager/`)

1. Add `pub(crate) mod antigravity;` to `plugin_manager/mod.rs`.
2. Create `plugin_manager/antigravity.rs` with `AntigravityPluginManager` implementing `CliAgentPluginManager`:
   - `EXTENSION_REPO`: `"https://github.com/warpdotdev/agy-warp"`.
   - `EXTENSION_NAME`: `"agy-warp"`.
   - `MINIMUM_PLUGIN_VERSION`: `"1.0.0"`.
   - `is_installed()`: checks if `~/.antigravitycli/extensions/agy-warp/agy-extension.json` exists.
   - `needs_update()`: compares the manifest version with `MINIMUM_PLUGIN_VERSION`.
   - `install()`: runs `agy extensions install https://github.com/warpdotdev/agy-warp`.
   - `update()`: runs `agy extensions update agy-warp`.
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

### 3g. Skill Provider Setup (`crates/ai/src/skills/`)

1. Register `Antigravity` in `SkillProvider` (`crates/ai/src/skills/skill_provider.rs`).
2. Add it to `SKILL_PROVIDER_DEFINITIONS`:
   ```rust
   SkillProviderDefinition {
       provider: SkillProvider::Antigravity,
       skills_path: PathBuf::from(".antigravitycli").join("skills"),
   }
   ```
3. Map it in `crates/ai/src/skills/conversion.rs` (`From<SkillProvider>` and `convert_provider` conversions). Since `warp_multi_agent_api` is imported from `warp-proto-apis`, if the proto does not yet contain `Antigravity` as a provider type, the implementation-safe behavior is to fall back by mapping it to the generic `Agents` proto enum variant to ensure the `From<SkillProvider>` conversion remains infallible.

## 4. End-to-End Flow

1. User types `agy` in their terminal and hits Enter.
2. The classifier marks it as a command execution, bypassing natural language matching.
3. The shell starts the `agy` process.
4. If the plugin isn't present, Warp detects this and displays a green install chip in the footer.
5. Clicking the chip executes the extension install command in the background.
6. Once the extension is active, it emits `warp://cli-agent` OSC 777 sequences.
7. Warp captures the sequences and initiates the Agent Mode layouts and toolbar, with custom branding and icons.

## 5. Testing and Validation

- **Unit tests**:
  - Add tests in `plugin_manager/antigravity_tests.rs` verifying `needs_update`, `is_installed`, and installation instruction steps.
  - Verify CLI agent detection works for prefix `agy` in `cli_agent_tests.rs`.
  - Validate that `DefaultSessionListener` correctly processes OSC 777 notifications and constructs the session-model data flow for `agent: "agy"`.
- **Manual Verification**: Run Warp locally, trigger `agy`, and verify Agent Mode transition and notification streaming.
