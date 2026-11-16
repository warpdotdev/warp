# APP-4154 Phase 1 — NSString checklist

Every Rust call site that produces or passes an NSString into Cocoa. Each batch agent fills in the `disposition`, `thread-origin`, `hot/cold`, `strategy`, and `action` columns for their rows, applies the fix, and ticks the row. See `TECH.md` for the decision rule.

## Reproducible greps

```
rg -n 'make_nsstring\b' -g '*.rs'
rg -n 'NSString::alloc' -g '*.rs'
```

Ignore these (not call sites):
- `crates/warpui/src/platform/mac/mod.rs:34` — definition of `make_nsstring` itself. Excluded because the body is a one-liner that always returns an autoreleased NSString; the leak potential is at its callers, not the definition.
- `use ... make_nsstring` import lines.
Audited even though it's a definition, not a call:
- `crates/warpui_extras/src/user_preferences/user_defaults.rs:88-89` — local `util::make_nsstring` helper definition. Unlike the warpui helper, this one returns a retained `StrongPtr`-wrapped NSString; the definition itself is the correctness point, so it's listed in batch 1.D.

## Row format

```
- [ ] path:line — function — disposition (retained|autoreleased|?) — thread-origin (appkit-event|gcd-block|rust-thread|unknown|?) — hot/cold — strategy (ambient|local-pool|autorelease-helper|explicit-release|?) — action
```

## Batch 1.A — `sentry-nsstring`

Files: `app/src/crash_reporting/mac.rs`. Reference pattern (`forward_breadcrumb`, already pooled) kept for consistency.

- [x] app/src/crash_reporting/mac.rs:28 — `init_cocoa_sentry` — autoreleased — rust-thread (early init from `init_sentry` before AppKit loop) — cold (once per session) — local-pool — wrapped body in `NSAutoreleasePool::new(nil)` / `pool.drain()`
- [x] app/src/crash_reporting/mac.rs:30 — `init_cocoa_sentry` — autoreleased — rust-thread — cold — local-pool — covered by the same pool
- [x] app/src/crash_reporting/mac.rs:31 — `init_cocoa_sentry` — autoreleased — rust-thread — cold — local-pool — covered by the same pool
- [x] app/src/crash_reporting/mac.rs:55 — `set_user_id` — autoreleased — rust-thread (invoked from `set_optional_user_information` on auth state changes and init) — cold — local-pool — wrapped body in `NSAutoreleasePool::new(nil)` / `pool.drain()`
- [x] app/src/crash_reporting/mac.rs:71 — `forward_breadcrumb` — autoreleased — rust-thread (Sentry `before_breadcrumb`) — hot — local-pool — already pooled (post-#560), confirmed no-op
- [x] app/src/crash_reporting/mac.rs:72 — `forward_breadcrumb` — autoreleased — rust-thread — hot — local-pool — already pooled, confirmed no-op
- [x] app/src/crash_reporting/mac.rs:73 — `forward_breadcrumb` — autoreleased — rust-thread — hot — local-pool — already pooled, confirmed no-op
- [x] app/src/crash_reporting/mac.rs:82 — `set_tag` (key) — autoreleased — rust-thread (called from `init_cocoa_sentry` loop and `set_tag` wrapper in `mod.rs`) — cold — local-pool — wrapped body in `NSAutoreleasePool::new(nil)` / `pool.drain()`
- [x] app/src/crash_reporting/mac.rs:82 — `set_tag` (value) — autoreleased — rust-thread — cold — local-pool — covered by the same pool

## Batch 1.B — `app-ffi-nsstring`

Files: `app/src/app_services/mac.rs`, `app/src/appearance.rs`, `app/src/util/file/external_editor/mac.rs`. `app/src/settings_view/appearance_page.rs` and `app/src/lib.rs` were dropped from this batch's scope: the rg invocations at the top of this file show no matches there, and a zero-hit re-grep is sufficient to prove completeness — no rows needed.

- [x] app/src/app_services/mac.rs:27 — `warp_services_provider_custom_url_scheme` — autoreleased — appkit-event (called from `services.m` inside an `@autoreleasepool` on the NSServices dispatch path) — cold — autorelease-helper — replaced the raw `NSString::alloc(nil).init_str(...).autorelease()` with `make_nsstring(...)`; the ambient ObjC pool owns the returned string.
- [x] app/src/appearance.rs:222 — `AppearanceManager::set_app_icon` (plugin_name) — autoreleased — mixed (startup from `lib.rs:1204`, settings/autoupdate completion callbacks) — cold — local-pool — wrapped the `unsafe { … }` body in `NSAutoreleasePool::new(nil)` held by an `AutoreleasePoolGuard` RAII wrapper whose `Drop` impl sends `drain`, so the pool is released on every exit path (early `return`, normal fall-through, or an unexpected panic from an intermediate `msg_send!`).
- [x] app/src/appearance.rs:233 — `AppearanceManager::set_app_icon` (image_name) — autoreleased — mixed (see above) — cold — local-pool — covered by the same `AutoreleasePoolGuard` as plugin_name.
- [x] app/src/appearance.rs:234 — `AppearanceManager::set_app_icon` (extension) — autoreleased — mixed (see above) — cold — local-pool — covered by the same `AutoreleasePoolGuard` as plugin_name.
- [x] app/src/util/file/external_editor/mac.rs:357 — `default_app_to_open_path` / `to_nsstring` helper — was retained (leaked, never released) — main-thread, UI action (`open_file_path_with_line_and_col`) — cold — autorelease-helper + local-pool — swapped to `make_nsstring`, wrapped the body in `NSAutoreleasePool::new(nil) … pool.drain()`, and changed the return type from `Option<&'static str>` to `Option<String>` so the UTF-8 bytes are copied out before the pool drains (the previous `'static` cast was a lie whose only safety net was the leak it caused).

Before ticking, agent 1.B must re-run the rg invocations at the top of this checklist across the whole workspace and confirm no new hits have landed since this scaffolding was written. Add any new rows that appear.

## Batch 1.C — `warpui-platform-nsstring`

Files: `crates/warpui/src/platform/mac/{app.rs, clipboard.rs, delegate.rs, menus.rs, window.rs, keycode.rs}`. If the batch diff exceeds ~200 lines, split by file.

- [x] crates/warpui/src/platform/mac/app.rs:81 — `create_native_platform_modal` — autoreleased — appkit-event (show_native_platform_modal via AppContext) — cold — ambient — no-op
- [x] crates/warpui/src/platform/mac/app.rs:82 — `create_native_platform_modal` — autoreleased — appkit-event — cold — ambient — no-op
- [x] crates/warpui/src/platform/mac/app.rs:84 — `create_native_platform_modal` — autoreleased — appkit-event — cold — ambient — no-op
- [x] crates/warpui/src/platform/mac/clipboard.rs:43 — `pasteboard_type_for_image_mime_type` — retained — appkit-event (copy action on main thread) — cold — autorelease-helper — switched to `make_nsstring`
- [x] crates/warpui/src/platform/mac/clipboard.rs:49 — `Clipboard::write` (plain text) — retained — appkit-event — cold — autorelease-helper — switched to `make_nsstring`
- [x] crates/warpui/src/platform/mac/clipboard.rs:55 — `Clipboard::write` (html) — retained — appkit-event — cold — autorelease-helper — switched to `make_nsstring`
- [x] crates/warpui/src/platform/mac/clipboard.rs:142 — `read_image_data_from_pasteboard` (public.png) — retained — appkit-event — cold — autorelease-helper — switched to `make_nsstring`
- [x] crates/warpui/src/platform/mac/clipboard.rs:143 — `read_image_data_from_pasteboard` (public.jpeg) — retained — appkit-event — cold — autorelease-helper — switched to `make_nsstring`
- [x] crates/warpui/src/platform/mac/clipboard.rs:144 — `read_image_data_from_pasteboard` (public.gif) — retained — appkit-event — cold — autorelease-helper — switched to `make_nsstring`
- [x] crates/warpui/src/platform/mac/clipboard.rs:145 — `read_image_data_from_pasteboard` (public.webp) — retained — appkit-event — cold — autorelease-helper — switched to `make_nsstring`
- [x] crates/warpui/src/platform/mac/clipboard.rs:146 — `read_image_data_from_pasteboard` (public.svg-image) — retained — appkit-event — cold — autorelease-helper — switched to `make_nsstring`
- [x] crates/warpui/src/platform/mac/clipboard.rs:147 — `read_image_data_from_pasteboard` (com.compuserve.gif) — retained — appkit-event — cold — autorelease-helper — switched to `make_nsstring`
- [x] crates/warpui/src/platform/mac/delegate.rs:257 — `application_bundle_info` — autoreleased — appkit-event — cold — ambient — no-op
- [x] crates/warpui/src/platform/mac/delegate.rs:267 — `application_bundle_info` — autoreleased — appkit-event — cold — ambient — no-op
- [x] crates/warpui/src/platform/mac/delegate.rs:343 — `send_desktop_notification` (title) — autoreleased — appkit-event — cold — ambient — no-op
- [x] crates/warpui/src/platform/mac/delegate.rs:344 — `send_desktop_notification` (body) — autoreleased — appkit-event — cold — ambient — no-op
- [x] crates/warpui/src/platform/mac/delegate.rs:345 — `send_desktop_notification` (data) — autoreleased — appkit-event — cold — ambient — no-op
- [x] crates/warpui/src/platform/mac/delegate.rs:423 — `microphone_access_state` — retained — appkit-event — cold — autorelease-helper — switched to `make_nsstring`
- [x] crates/warpui/src/platform/mac/keycode.rs:50 — `Keycode::keycodes_from_key_name` (`charToKeyCodes` wrapper) — autoreleased — appkit-event (register/unregister global shortcut via AppContext) — cold — ambient — no-op
- [x] crates/warpui/src/platform/mac/menus.rs:215 — `resolve_key_equivalent` (empty key_equivalent) — autoreleased — appkit-event (menu item update) — hot — local-pool — covered by pool wrapping `apply_changes` body (hot: AppKit menu validation per open/shortcut)
- [x] crates/warpui/src/platform/mac/menus.rs:219 — `resolve_key_equivalent` (special char key equivalent) — autoreleased — appkit-event — hot — local-pool — covered by pool wrapping `apply_changes` body
- [x] crates/warpui/src/platform/mac/menus.rs:220 — `resolve_key_equivalent` (literal key equivalent) — autoreleased — appkit-event — hot — local-pool — covered by pool wrapping `apply_changes` body
- [x] crates/warpui/src/platform/mac/menus.rs:240 — `apply_changes` (setTitle) — autoreleased — appkit-event — hot — local-pool — wrapped `apply_changes` body in NSAutoreleasePool
- [x] crates/warpui/src/platform/mac/menus.rs:265 — `make_submenu` (delegated menu title) — autoreleased — appkit-event (menu rebuild) — cold — ambient — no-op
- [x] crates/warpui/src/platform/mac/menus.rs:296 — `make_menu_item` standard-action title — autoreleased — appkit-event (menu rebuild) — cold — ambient — no-op
- [x] crates/warpui/src/platform/mac/menus.rs:298 — `make_menu_item` standard-action key equivalent — retained — appkit-event — cold — autorelease-helper — switched to `make_nsstring`
- [x] crates/warpui/src/platform/mac/menus.rs:313 — `make_top_level_menu_item` (top-level menu title) — autoreleased — appkit-event (app startup / menubar rebuild) — cold — ambient — no-op
- [x] crates/warpui/src/platform/mac/window.rs:711 — `Window::open_url` — autoreleased — appkit-event (delegate call from AppContext) — cold — ambient — no-op
- [x] crates/warpui/src/platform/mac/window.rs:718 — `Window::open_file_path` — autoreleased — appkit-event — cold — ambient — no-op
- [x] crates/warpui/src/platform/mac/window.rs:726 — `Window::open_file_path_in_explorer` — autoreleased — appkit-event — cold — ambient — no-op
- [x] crates/warpui/src/platform/mac/window.rs:735 — `Window::open_file_picker` (file type mapping) — autoreleased — appkit-event — cold — ambient — no-op
- [x] crates/warpui/src/platform/mac/window.rs:754 — `Window::open_save_file_picker` (default_directory) — autoreleased — appkit-event — cold — ambient — no-op
- [x] crates/warpui/src/platform/mac/window.rs:755 — `Window::open_save_file_picker` (default_directory fallback) — autoreleased — appkit-event — cold — ambient — no-op
- [x] crates/warpui/src/platform/mac/window.rs:758 — `Window::open_save_file_picker` (default_filename) — autoreleased — appkit-event — cold — ambient — no-op
- [x] crates/warpui/src/platform/mac/window.rs:759 — `Window::open_save_file_picker` (default_filename fallback) — autoreleased — appkit-event — cold — ambient — no-op
- [x] crates/warpui/src/platform/mac/window.rs:803 — `Window::set_accessibility_contents` (value) — autoreleased — appkit-event (fires per user action when VoiceOver is enabled) — hot — local-pool — wrapped `Window::set_accessibility_contents` body in NSAutoreleasePool
- [x] crates/warpui/src/platform/mac/window.rs:804 — `Window::set_accessibility_contents` (help) — autoreleased — appkit-event — hot — local-pool — covered by pool wrapping `Window::set_accessibility_contents` body
- [x] crates/warpui/src/platform/mac/window.rs:805 — `Window::set_accessibility_contents` (role) — autoreleased — appkit-event — hot — local-pool — covered by pool wrapping `Window::set_accessibility_contents` body
- [x] crates/warpui/src/platform/mac/window.rs:893 — `Window::set_window_title` — autoreleased — appkit-event — cold — ambient — no-op
- [x] crates/warpui/src/platform/mac/window.rs:1230 — `warp_get_accessibility_contents` (C-unwind) — autoreleased — appkit-event (AppKit accessibility callback) — hot — ambient — no-op; local-pool not applicable because the autoreleased NSString is the return value and must outlive this scope

## Batch 1.D — `warpui-extras-nsstring`

Files: `crates/warpui_extras/src/user_preferences/user_defaults.rs`.

This batch also owns the adjacent `msg_send![class!(NSUserDefaults), alloc]` site on line 39 (even though it's Phase-2 by category), because editing lines 39 and 40 from separate PRs would conflict on merge.

- [x] crates/warpui_extras/src/user_preferences/user_defaults.rs:39 — `UserDefaultsPreferencesStorage::user_defaults` — retained (chained into `initWithSuiteName:` on line 42 and wrapped in `StrongPtr::new`) — rust-thread (startup) — cold — explicit-release — no-op: `alloc` → `initWithSuiteName:` → `StrongPtr::new` takes ownership of the +1 retain; drop releases (Phase 2 row, owned here to avoid adjacency conflicts)
- [x] crates/warpui_extras/src/user_preferences/user_defaults.rs:40 — `UserDefaultsPreferencesStorage::user_defaults` — retained (local `util::make_nsstring` returns `StrongPtr`) — rust-thread (startup) — cold — explicit-release — no-op: `StrongPtr` drop at end of scope releases the retained NSString
- [x] crates/warpui_extras/src/user_preferences/user_defaults.rs:53 — `UserPreferences::write_value` (key) — retained (local `util::make_nsstring` returns `StrongPtr`) — rust-thread (settings writes) — cold — explicit-release — no-op: `StrongPtr` drop at end of scope releases
- [x] crates/warpui_extras/src/user_preferences/user_defaults.rs:54 — `UserPreferences::write_value` (value) — retained (local `util::make_nsstring` returns `StrongPtr`) — rust-thread (settings writes) — cold — explicit-release — no-op: `StrongPtr` drop at end of scope releases
- [x] crates/warpui_extras/src/user_preferences/user_defaults.rs:63 — `UserPreferences::read_value` (key) — retained (local `util::make_nsstring` returns `StrongPtr`) — rust-thread (settings reads) — cold — explicit-release — no-op: `StrongPtr` drop at end of scope releases
- [x] crates/warpui_extras/src/user_preferences/user_defaults.rs:77 — `UserPreferences::remove_value` (key) — retained (local `util::make_nsstring` returns `StrongPtr`) — rust-thread (settings removes) — cold — explicit-release — no-op: `StrongPtr` drop at end of scope releases
- [x] crates/warpui_extras/src/user_preferences/user_defaults.rs:89 — `util::make_nsstring` body (`NSString::alloc(nil).init_str(...)`) — retained (wrapped in `StrongPtr::new`) — n/a (helper) — n/a — explicit-release — no-op: `NSString::alloc(nil).init_str(...)` returns a +1 retained object; `StrongPtr::new` takes ownership without additional retain, and its `Drop` impl sends `release`, balancing the alloc/init
