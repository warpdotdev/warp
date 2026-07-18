# Technical Spec: Support 'agy' (Antigravity) CLI Agent in Warp

See `specs/GH11368/product.md` for the product spec.

**Issue:** [warpdotdev/warp#11368](https://github.com/warpdotdev/warp/issues/11368)

## 1. Problem

Warp lacks native command detection and branding support for the `agy` (Antigravity) CLI agent. Running the agent does not trigger Agent Mode or the associated toolbar.

To resolve this, we must wire `CLIAgent::Antigravity` into the terminal's agent detection and branding config. Support for plugins and OSC 777 notifications is intentionally excluded in this milestone.

## 2. Relevant Code

- `app/src/terminal/cli_agent.rs` — `CLIAgent` enum with all identity methods: `command_prefix()`, `to_serialized_name()`, `from_serialized_name()`, `from_harness()`, `display_name()`, `icon()`, `supported_skill_providers()`, `skill_command_prefix()`, `supports_bash_mode()`, `brand_color()`, `brand_icon_color()`, `detect()`, and the `From<CLIAgent> for CLIAgentType` telemetry conversion.
- `crates/input_classifier/src/util.rs` — `ONE_OFF_SHELL_COMMAND_KEYWORDS` that determine shell-vs-natural-language classification.
- `crates/warp_core/src/ui/icons.rs` — `Icon` enum register and SVG asset mappings.
- `app/src/server/telemetry/events.rs` — `CLIAgentType` telemetry enum.

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
2. The new `Antigravity` variant must appear in every match arm across the `CLIAgent` impl.
   - `command_prefix()`: returns `"agy"`.
   - `to_serialized_name()`: Uses derive macro `"Antigravity"`.
   - `from_serialized_name()`: Uses derive macro `"Antigravity"`.
   - `from_harness()`: No change needed.
   - `display_name()`: returns `"Antigravity"`.
   - `icon()`: returns `Some(Icon::AntigravityLogo)`.
   - `supported_skill_providers()`: returns `&[]` (no skills integration yet).
   - `skill_command_prefix()`: Falls through to wildcard `_ => "/"`.
   - `supports_bash_mode()`: Falls through to `false`.
   - `brand_color()`: returns `Some(ANTIGRAVITY_COLOR)`, matching Pi's white monochrome brand tile color. Add an `ANTIGRAVITY_COLOR` constant at module level.
   - `brand_icon_color()`: returns `ColorU::new(0, 0, 0, 255)`, matching Pi's black logo color on light brand tiles.
   - `detect()`: Works automatically via `enum_iterator::Sequence` and prefix `"agy"`.
3. Add `CLIAgent::Antigravity => CLIAgentType::Antigravity` to the telemetry conversion. Add an `Antigravity` variant to `CLIAgentType` in `app/src/server/telemetry/events.rs`.

### 3b. Register Command Classifier (`crates/input_classifier/src/util.rs`)

Add `"agy"` to the static `ONE_OFF_SHELL_COMMAND_KEYWORDS` hashset:
```rust
static ref ONE_OFF_SHELL_COMMAND_KEYWORDS: HashSet<&'static str> = HashSet::from([
    "#", "echo", "man", "sudo", "claude", "codex", "gemini", "agy"
]);
```

### 3c. Add SVG Asset and Icon Registry (`crates/warp_core/src/ui/icons.rs`)

1. Add `AntigravityLogo` to the `Icon` enum.
2. Map it to the SVG path in the match expression inside `Icon::svg_path()`:
   ```rust
   Icon::AntigravityLogo => "bundled/svg/antigravity_cli.svg",
   ```
3. Add the SVG asset file `app/assets/bundled/svg/antigravity_cli.svg`.

## 4. End-to-End Flow

1. User types `agy` in their terminal and hits Enter.
2. The classifier marks it as a command execution.
3. The shell starts the `agy` process.
4. Warp identifies the `agy` command via `CLIAgent::detect()` and initiates the Agent Mode layouts and toolbar, with custom branding and icons.

## 5. Testing and Validation

- **Unit tests**: Verify CLI agent detection works for prefix `agy` in `cli_agent_tests.rs`.
- **Unit tests**: Verify `"agy"` short-circuits as a one-off shell keyword in `input_classifier` tests, ensuring it bypasses natural language classification.
- **Manual Verification**: Run Warp locally, trigger `agy`, and verify Agent Mode transition and UI styling.
