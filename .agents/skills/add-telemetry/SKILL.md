---
name: add-telemetry
description: Add telemetry events to track user behavior or system events in the Warp codebase. Use when instrumenting new features, debugging issues, or measuring product metrics.
---

# add-telemetry

## Overview

Warp uses a trait-based telemetry system where feature-specific enums implement the `TelemetryEvent` trait. This approach keeps telemetry events organized by domain rather than in one giant enum.

**Important**: Before implementing telemetry, collaborate with the user to:
- Define what events should be tracked and when
- Determine what data should be included in each event
- Clarify the purpose and expected usage of the telemetry

Adding telemetry code is straightforward, but designing meaningful instrumentation requires careful thought.

## Steps

### 1. Identify or create a telemetry module

Find an existing feature-specific telemetry file (e.g., `app/src/antivirus/telemetry.rs`) or create a new one for your feature area.

### 2. Define the telemetry event enum

Add a new variant to an enum that implements `TelemetryEvent`, or create a new enum:

```rust
use serde_json::{json, Value};
use strum_macros::{EnumDiscriminants, EnumIter};
use warp_core::telemetry::{EnablementState, TelemetryEvent, TelemetryEventDesc};

#[derive(Debug, EnumDiscriminants)]
#[strum_discriminants(derive(EnumIter))]
pub enum YourFeatureTelemetryEvent {
    ActionStarted {
        duration_ms: u64,
    },
    ActionCompleted {
        success: bool,
        error: Option<String>,
    },
}
```

### 3. Implement the TelemetryEvent trait

`EnablementState` allows you to control when events are sent:

- `EnablementState::Always` - Always send the event
- `EnablementState::Flag(FeatureFlag::YourFeature)` - Only send when the feature flag is enabled
- `EnablementState::Channel(Channel::Dev)` - Only send in specific build channels

```rust
impl TelemetryEvent for YourFeatureTelemetryEvent {
    fn name(&self) -> &'static str {
        YourFeatureTelemetryEventDiscriminants::from(self).name()
    }

    fn payload(&self) -> Option<Value> {
        match self {
            Self::ActionStarted { duration_ms } => Some(json!({
                "duration_ms": duration_ms,
            })),
            Self::ActionCompleted { success, error } => Some(json!({
                "success": success,
                "error": error,
            })),
        }
    }

    fn description(&self) -> &'static str {
        YourFeatureTelemetryEventDiscriminants::from(self).description()
    }

    fn enablement_state(&self) -> EnablementState {
        YourFeatureTelemetryEventDiscriminants::from(self).enablement_state()
    }

    fn contains_ugc(&self) -> bool {
        match self {
            Self::ActionStarted { .. } => false,
            Self::ActionCompleted { .. } => false,
        }
    }

    fn event_descs() -> impl Iterator<Item = Box<dyn TelemetryEventDesc>> {
        warp_core::telemetry::enum_events::<Self>()
    }
}
```

### 4. Implement TelemetryEventDesc for the discriminants

```rust
impl TelemetryEventDesc for YourFeatureTelemetryEventDiscriminants {
    fn name(&self) -> &'static str {
        match self {
            Self::ActionStarted => "YourFeature.Action.Started",
            Self::ActionCompleted => "YourFeature.Action.Completed",
        }
    }

    fn description(&self) -> &'static str {
        match self {
            Self::ActionStarted => "User started the action",
            Self::ActionCompleted => "User completed the action",
        }
    }

    fn enablement_state(&self) -> EnablementState {
        match self {
            Self::ActionStarted | Self::ActionCompleted => EnablementState::Always,
            // Or gate behind a feature flag:
            // EnablementState::Flag(FeatureFlag::YourFeature)
        }
    }
}
```

### 5. Register the telemetry event

At the end of your telemetry module, register the event:

```rust
warp_core::register_telemetry_event!(YourFeatureTelemetryEvent);
```

### 6. Send telemetry events from your code

Use `send_telemetry_from_ctx!` in views or models with a `ViewContext` or `ModelContext`:

```rust
use warp_core::send_telemetry_from_ctx;

// In a view update or model method
send_telemetry_from_ctx!(
    YourFeatureTelemetryEvent::ActionStarted {
        duration_ms: 150,
    },
    ctx
);
```

For code with only `AppContext`, use `send_telemetry_from_app_ctx!` instead.

### 7. Test locally

Run Warp with the `log_named_telemetry_events` feature flag to see telemetry events logged to the console:

```bash
cargo run --features log_named_telemetry_events
```

## Best Practices

- Keep telemetry enums feature-specific rather than adding to a global enum
- Set `contains_ugc()` to `true` if the payload includes user-generated content
- Use descriptive event names following the pattern `Feature.Action.Result`
- Include only necessary data in payloads to minimize bandwidth and storage
- Consider privacy implications when deciding what data to include
- Avoid exhaustive matching with wildcards; handle all variants explicitly

## Example Reference

See `app/src/antivirus/telemetry.rs` for a complete example of a feature-specific telemetry implementation.
