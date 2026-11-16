# APP-4104: Tech Spec

## Problem

APP-4104 covers the macOS memory growth on the breadcrumb-forwarding path between Rust Sentry and Cocoa Sentry. The hot path is installed in `before_breadcrumb` and forwards every Rust `sentry::Breadcrumb` into the native bridge. Today that bridge creates `NSString` values with `alloc/init_str` and passes them across FFI without balancing ownership. In breadcrumb-heavy sessions, temporary bridge allocations can live far longer than intended and drive sustained process memory growth.

## Relevant code

- `app/src/crash_reporting/mod.rs (476-489)` — installs the `before_breadcrumb` callback and forwards macOS breadcrumbs through `mac::forward_breadcrumb`
- `app/src/crash_reporting/mac.rs (20-84)` — Rust-to-ObjC Cocoa Sentry bridge, including `forward_breadcrumb`, `set_user_id`, `set_tag`, and `to_nsstring`
- `app/src/platform/mac/objc/crash_reporting.m (67-81)` — native `recordBreadcrumb` implementation that constructs `SentryBreadcrumb`
- `crates/warpui/src/platform/mac/mod.rs (29-34)` — `make_nsstring`, the existing helper that returns an autoreleased `NSString`
- `app/src/app_services/mac.rs (20-29)` — another app-side example of returning an autoreleased `NSString`

## Current state

### Rust bridge

`init_sentry` registers `before_breadcrumb`, and the macOS branch calls `mac::forward_breadcrumb(&crumb)` before returning the original Rust breadcrumb unchanged. `forward_breadcrumb` extracts `message`, `category`, `level`, and `timestamp`, then calls `recordBreadcrumb`.

The string bridge in `app/src/crash_reporting/mac.rs` is currently:

- `unsafe fn to_nsstring(val: &str) -> id`
- implemented with `NSString::alloc(nil).init_str(val)`
- used by `init_cocoa_sentry`, `set_user_id`, `forward_breadcrumb`, and `set_tag`

That helper returns a retained Objective-C object. The crash-reporting bridge does not currently release or autorelease those values before returning to the caller.

### Objective-C bridge

`recordBreadcrumb` creates a `SentryBreadcrumb`, maps the level string via `levelFromString`, sets `category`, `message`, and `timestamp`, and then calls `[SentrySDK addBreadcrumb:crumb]`.

That file is compiled without ARC, so the native breadcrumb object itself must still be balanced explicitly after the `SentrySDK` call returns.

### Existing repo pattern

Warp already has established patterns for temporary Cocoa strings:

- `warpui::platform::mac::make_nsstring` returns an autoreleased `NSString`
- app-owned macOS bridges already use direct `.autorelease()` in places like `app_services/mac.rs`

The crash-reporting bridge is the outlier.

## Proposed changes

### 1. Replace retained bridge strings with autoreleased strings

Remove the retained `to_nsstring` pattern from `app/src/crash_reporting/mac.rs` and switch the file to the same autoreleased `NSString` strategy already used elsewhere in the repo.

Concretely:

- import `warpui::platform::mac::make_nsstring` at the top of the file
- use that helper for every bridge string created in this module
- apply the change consistently to `init_cocoa_sentry`, `set_user_id`, `forward_breadcrumb`, and `set_tag`

Using one helper for the whole file keeps the ownership model uniform at the Rust→ObjC boundary and fixes the hot breadcrumb path without leaving the same bug in the lower-volume tag and user paths.

### 2. Add a local autorelease boundary in `forward_breadcrumb`

Create an `NSAutoreleasePool` inside `app/src/crash_reporting/mac.rs::forward_breadcrumb` so the pool is active before any bridge `NSString` calls `autorelease()`, and drain it immediately after `recordBreadcrumb` returns.

This bounds the lifetime of the Rust-created bridge strings because they are now autoreleased while the per-breadcrumb pool is current, rather than relying on an unknown outer pool on whatever thread emitted the log record.

### 3. Keep native breadcrumb ownership explicit

Keep `recordBreadcrumb`'s existing `![SentrySDK isEnabled]` guard and field mapping logic, but explicitly balance the non-ARC `SentryBreadcrumb` allocation with `[crumb release]` after `[SentrySDK addBreadcrumb:crumb]`.

### 4. Preserve breadcrumb semantics

This fix is ownership-only. It does not change:

- which Rust log records become breadcrumbs
- how `before_breadcrumb` is registered
- the `message`, `category`, `level`, or `timestamp` values forwarded to Cocoa Sentry
- Sentry’s native breadcrumb storage limits or filtering behavior

## End-to-end flow

1. Rust logging emits a record and Sentry Rust turns it into a `sentry::Breadcrumb`.
2. `before_breadcrumb` forwards that breadcrumb to `mac::forward_breadcrumb`.
3. `forward_breadcrumb` creates a short-lived `NSAutoreleasePool`, then creates autoreleased `NSString` values for `message`, `category`, and `level`, and calls `recordBreadcrumb`.
4. `recordBreadcrumb` creates a native `SentryBreadcrumb`, sets its fields, passes it to `SentrySDK`, and explicitly releases its local ownership before returning.
5. `forward_breadcrumb` drains its local pool once the native call completes; only the objects retained by Cocoa Sentry remain live.

## Risks and mitigations

1. **Autoreleased arguments must be retained by the callee**
   `SentryBreadcrumb` and `SentrySDK` must retain or copy any values they keep after `recordBreadcrumb` returns. This is the standard Objective-C ownership contract for stored properties, and the objects remain valid for the full duration of the call.

2. **Per-breadcrumb autorelease-pool overhead**
   Adding a small pool around each breadcrumb has a fixed cost. That cost is negligible compared with the existing cross-language allocation work, and bounding memory is the higher-priority outcome for this path.

3. **Other macOS FFI bridges may still have separate ownership issues**
   APP-4104 is scoped to crash reporting because that is the hot path implicated by the memory profile. A wider audit can be handled separately once this fix lands.

## Testing and validation

1. Build the macOS client path with Cocoa Sentry enabled to catch compile and linkage issues in the Rust and Objective-C bridge.
2. Run a breadcrumb-heavy macOS session and verify that RSS no longer grows linearly while breadcrumb forwarding remains active.
3. Capture a Sentry event after the change and confirm breadcrumb `message`, `category`, `level`, and `timestamp` still appear as before.
4. Confirm `set_user_id` and `set_tag` still reach Cocoa Sentry after they switch to the same autoreleased string helper.

## Follow-ups

- If APP-4104 resolves the memory growth, do a focused audit of other app-owned macOS FFI helpers that still call `NSString::alloc(nil).init_str(...)` directly so we can standardize on one safe bridge helper.
