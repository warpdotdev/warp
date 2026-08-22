# Disable file path links in terminal output — Tech Spec
Product spec: `specs/GH12890/product.md`
GitHub issue: https://github.com/warpdotdev/warp/issues/12890

## Context
The current implementation has separate paths for URL detection and implicit file-path detection. URL detection happens synchronously from terminal grid text, while file-path detection validates path-shaped text against the local filesystem on `local_fs` builds and can run in a background thread. The new setting should gate only the file-path branches so URL links, markdown hyperlinks, and explicit URL opening behavior continue unchanged.

Relevant code inspected at `b2804a09125a0249f5d949c267d43de59e1df791`:

- [`app/src/terminal/general_settings.rs:55-65 @ b2804a09`](https://github.com/warpdotdev/warp/blob/b2804a09125a0249f5d949c267d43de59e1df791/app/src/terminal/general_settings.rs#L55-L65) — `general.link_tooltip`, the existing adjacent general setting that controls tooltip behavior but not link detection.
- [`app/src/settings_view/features_page.rs:708-746 @ b2804a09`](https://github.com/warpdotdev/warp/blob/b2804a09125a0249f5d949c267d43de59e1df791/app/src/settings_view/features_page.rs#L708-L746), [`app/src/settings_view/features_page.rs:1856-1860 @ b2804a09`](https://github.com/warpdotdev/warp/blob/b2804a09125a0249f5d949c267d43de59e1df791/app/src/settings_view/features_page.rs#L1856-L1860), and [`app/src/settings_view/features_page.rs:4690-4728 @ b2804a09`](https://github.com/warpdotdev/warp/blob/b2804a09125a0249f5d949c267d43de59e1df791/app/src/settings_view/features_page.rs#L4690-L4728) — Settings action/widget pattern for the current link tooltip toggle.
- [`app/src/settings_view/features_page.rs:2665-2670 @ b2804a09`](https://github.com/warpdotdev/warp/blob/b2804a09125a0249f5d949c267d43de59e1df791/app/src/settings_view/features_page.rs#L2665-L2670) — the General settings widget list where the new control should be placed near link-tooltip behavior.
- [`app/src/terminal/view/link_detection.rs:242-329 @ b2804a09`](https://github.com/warpdotdev/warp/blob/b2804a09125a0249f5d949c267d43de59e1df791/app/src/terminal/view/link_detection.rs#L242-L329) — `TerminalView::maybe_link_hover`, which first detects URLs and then enqueues background file-path scanning when no URL was found.
- [`app/src/terminal/view/link_detection.rs:338-629 @ b2804a09`](https://github.com/warpdotdev/warp/blob/b2804a09125a0249f5d949c267d43de59e1df791/app/src/terminal/view/link_detection.rs#L338-L629) — `handle_find_link`, `scan_for_file_path`, `compute_valid_paths`, and `handle_file_link_completed`.
- [`app/src/terminal/model/grid/grid_handler.rs:648-747 @ b2804a09`](https://github.com/warpdotdev/warp/blob/b2804a09125a0249f5d949c267d43de59e1df791/app/src/terminal/model/grid/grid_handler.rs#L648-L747) — grid URL scanning, which should remain ungated.
- [`app/src/terminal/model/grid/grid_handler.rs:1108-1271 @ b2804a09`](https://github.com/warpdotdev/warp/blob/b2804a09125a0249f5d949c267d43de59e1df791/app/src/terminal/model/grid/grid_handler.rs#L1108-L1271) — grid file-path candidate generation used by hover file-link scanning.
- [`app/src/util/link_detection.rs:349-424 @ b2804a09`](https://github.com/warpdotdev/warp/blob/b2804a09125a0249f5d949c267d43de59e1df791/app/src/util/link_detection.rs#L349-L424) — `detect_file_paths`, which validates rich-content path candidates against the local filesystem.
- [`app/src/util/link_detection.rs:587-690 @ b2804a09`](https://github.com/warpdotdev/warp/blob/b2804a09125a0249f5d949c267d43de59e1df791/app/src/util/link_detection.rs#L587-L690) — `detect_all_links` and `detect_links`, which combine URLs, markdown hyperlinks, and file paths for AI/rich terminal content.
- [`app/src/ai/blocklist/block.rs:1648-1766 @ b2804a09`](https://github.com/warpdotdev/warp/blob/b2804a09125a0249f5d949c267d43de59e1df791/app/src/ai/blocklist/block.rs#L1648-L1766) — `AIBlock::spawn_link_detection`, where rich-content link detection runs in a background task.
- [`app/src/ai/blocklist/block.rs:4551-4563 @ b2804a09`](https://github.com/warpdotdev/warp/blob/b2804a09125a0249f5d949c267d43de59e1df791/app/src/ai/blocklist/block.rs#L4551-L4563) — action-result file list link detection that calls `detect_links` directly.
- [`app/src/terminal/view.rs:18199-18235 @ b2804a09`](https://github.com/warpdotdev/warp/blob/b2804a09125a0249f5d949c267d43de59e1df791/app/src/terminal/view.rs#L18199-L18235) — grid click handling uses `GeneralSettings::link_tooltip` only for tooltip opening, so the new setting should prevent file links from reaching `highlighted_link` rather than overloading tooltip behavior.

## Proposed changes

### 1. Add a synced general setting
Add a new boolean setting to `GeneralSettings` next to `link_tooltip`.

Suggested Rust and config shape:

- setting field: `file_path_links`
- setting type: `FilePathLinks`
- default: `true`
- supported platforms: `SupportedPlatforms::ALL`
- sync: `SyncToCloud::Globally(RespectUserSyncSetting::Yes)`
- public user-config path: `general.file_path_links`
- description: `Whether to detect file and directory paths as clickable links in terminal output.`

This keeps backward compatibility because the default preserves today’s behavior. It also keeps the new setting independent from `general.link_tooltip`, which should continue to mean only “show a tooltip instead of directly opening on a normal click.”

### 2. Add Settings UI and action plumbing
Update `app/src/settings_view/features_page.rs` following the existing `LinkTooltipWidget` pattern:

- Import `FilePathLinks` from `crate::terminal::general_settings`.
- Add `FeaturesPageAction::ToggleFilePathLinks`.
- Add telemetry for the action using the generic `TelemetryEvent::FeaturesPageAction` pattern, with action name `ToggleFilePathLinks` and boolean value from `GeneralSettings::file_path_links`.
- Add a handler arm that calls `GeneralSettings::handle(ctx).update(... settings.file_path_links.toggle_and_save_value(ctx) ...)`.
- Add a `FilePathLinksWidget` near `LinkTooltipWidget` in the General section.
- Suggested label: `Link file paths in terminal output`.
- Suggested search terms: `file path link clickable path terminal output open file folder directory`.
- Add a `ToggleSettingActionPair` in `init_actions_from_parent_view` so Command Palette/settings-search toggle discovery matches the surrounding General toggles.

No new feature flag is needed. The control should appear wherever the existing link tooltip control appears unless the final implementation has a platform-specific reason to hide it.

### 3. Gate terminal-grid file-path scanning, not URL scanning
Update `TerminalView::maybe_link_hover` and the file-path scanning helpers in `app/src/terminal/view/link_detection.rs`.

The intended flow:

1. Keep `model.url_at_point(position)` running regardless of the setting.
2. If a URL is found, set `GridHighlightedLink::Url` exactly as today.
3. If no URL is found and `GeneralSettings::as_ref(ctx).file_path_links` is false:
   - clear any existing `GridHighlightedLink::File`;
   - do not enqueue `FindLinkArg`;
   - update `last_hover_fragment_boundary` enough to avoid repeated unnecessary work while hovering the same fragment;
   - keep the cursor as arrow/default.
4. If no URL is found and the setting is true, run the existing file-path scan path.

Add defensive guards in `handle_find_link` or `scan_for_file_path` as well, because a `FindLinkArg` may have been queued before the user toggled the setting off. `handle_file_link_completed` should also check the setting before applying a completed `GridHighlightedLink::File`. If the setting is now disabled, drop the result, clear stale highlighting if necessary, reset the cursor, and notify.

This three-layer guard prevents stale background filesystem work from reintroducing file links after the setting turns off.

### 4. Gate rich-content file-path detection with an explicit parameter
Update `app/src/util/link_detection.rs` so callers decide whether file-path detection is enabled.

Suggested API changes:

- `detect_all_links(..., file_path_links_enabled: bool, current_working_directory: Option<&String>, shell_launch_data: Option<&ShellLaunchData>)`
- `detect_links(..., file_path_links_enabled: bool, current_working_directory: Option<&String>, shell_launch_data: Option<&ShellLaunchData>)`

Inside both functions:

- Always call `detect_urls`.
- Always add pre-extracted markdown hyperlinks as `DetectedLinkType::Url`.
- On `local_fs`, call `detect_file_paths` only when `file_path_links_enabled` is true and a current working directory exists.
- Preserve the existing URL-over-file overlap rule when file path detection is enabled.

Pass `*GeneralSettings::as_ref(ctx).file_path_links` from `AIBlock::spawn_link_detection` before spawning the blocking task, so the background task captures a stable setting value. In the task completion handler, re-check the current setting before replacing link state if the computed result can include file paths. The simplest safe implementation is to recompute with the captured setting and, if the setting has since turned off, either discard file-path entries before `replace_all_links` or rerun the cheap URL-only path on the main thread.

For direct `detect_links` calls in action-result handling, pass the current setting value so SearchCodebase fallback file lists do not create implicit file links when disabled.

### 5. Re-run or clear rich-content link detection on setting changes
`AIBlock` currently re-runs link detection when shell launch data or rendered output changes, but the new setting can change independently. Subscribe `AIBlock` to `GeneralSettings::handle(ctx)` or extend an existing settings subscription so a `file_path_links` change:

1. aborts any in-flight rich-content link-detection task;
2. clears stale `DetectedLinkType::FilePath` hover and tooltip state;
3. re-runs `spawn_link_detection(ctx)` so URL links and markdown hyperlinks remain available while file-path entries disappear.

Avoid clearing all links permanently when the setting turns off, because URLs must stay interactive. Replacing the state with a URL-only detection result is preferable to clearing `DetectedLinksState` and waiting for unrelated output changes.

### 6. Keep opening and context-menu code mostly unchanged
The open/click paths in `app/src/terminal/view.rs`, `app/src/terminal/view/tooltips.rs`, and `app/src/ai/blocklist/block.rs` can stay largely unchanged if file links are prevented from entering highlighted/detected-link state.

Expected outcomes:

- `click_on_grid` still opens URL tooltips based on `general.link_tooltip`.
- `maybe_open_link` still opens whichever highlighted link exists.
- File-specific context-menu items disappear naturally because `highlighted_link` is no longer `GridHighlightedLink::File`.
- `AIBlock::open_link` and `show_link_tooltip` still handle `DetectedLinkType::FilePath`, but that variant is not produced while the setting is disabled.

Keep any additional checks limited and defensive. The core ownership boundary should remain “link detection decides whether file links exist,” not “every consumer rechecks the setting.”

### 7. User config and generated schema considerations
Because `define_settings_group!` drives setting storage/schema behavior, adding `toml_path: "general.file_path_links"` should expose the setting in user config the same way `general.link_tooltip` is exposed. Verify whether this repository requires regenerating any settings schema artifacts after adding a public setting; if so, run the existing generation command documented for settings changes and include the generated diff in the implementation PR.

### 8. Telemetry and privacy
Use the existing Settings-page toggle telemetry pattern only. Do not add telemetry for individual path hovers, path text, path-detection results, or filesystem validation failures. File paths can contain sensitive local information, so the implementation should not log newly detected path text as part of this feature.

## Testing and validation

Unit and view tests should map directly to the product invariants in `specs/GH12890/product.md`.

- `app/src/terminal/view/link_detection.rs` or existing terminal view tests:
  - setting enabled: hovering an existing file path still sets a `GridHighlightedLink::File`.
  - setting disabled: hovering an existing file path does not enqueue or apply a file highlight.
  - setting disabled: hovering a URL still sets `GridHighlightedLink::Url`.
  - setting toggled off after a file scan is queued: the completed scan does not set `GridHighlightedLink::File`.

- `app/src/util/link_detection_tests.rs`:
  - `detect_all_links` with file-path links enabled returns both URL and valid file-path links for mixed text.
  - `detect_all_links` with file-path links disabled returns URL links but omits file-path links.
  - URL-over-file overlap behavior remains unchanged when enabled.
  - `detect_links` direct-call behavior matches `detect_all_links`.

- `app/src/ai/blocklist/block.rs` tests or nearby AI block rendering tests:
  - toggling the setting off replaces rich-content detected links with URL-only results.
  - action-result file list detection respects the setting.

- Settings tests:
  - `GeneralSettings::file_path_links` defaults to true.
  - `general.file_path_links` persists and sync metadata matches the spec.
  - `FeaturesPageAction::ToggleFilePathLinks` toggles and saves the value.
  - Settings search terms find the widget.

Manual validation:

1. Start Warp with the default setting. In a local directory, run output that contains an existing file path and a URL. Confirm both are clickable.
2. Disable “Link file paths in terminal output.” Hover the same output. Confirm the file path is plain text and the URL remains clickable.
3. While a file path is highlighted, disable the setting. Confirm the highlight and cursor clear.
4. Re-enable the setting and hover the file path again. Confirm file-path link behavior returns.
5. Repeat in alt-screen content such as a full-screen terminal program where Warp currently detects file paths.
6. Repeat in an AI/rich terminal response containing a valid file path and a URL. Confirm only the file path changes behavior.

Recommended validation commands for an implementation PR:

- Targeted Rust unit tests for `app/src/util/link_detection_tests.rs`.
- Targeted terminal view/link detection tests added by the implementation.
- The repository’s normal formatting/check command for Rust changes.

## Parallelization
Do not use parallel implementation agents for the first implementation pass. The setting, terminal hover path, rich-content link-detection path, and tests are tightly coupled enough that a single implementer is less likely to introduce inconsistent gating or stale-link races. A follow-up validation agent could run targeted tests after the implementation compiles, but code ownership should stay in one branch and one PR.

## Risks and mitigations

### Risk: accidentally disabling URL links
The terminal hover path computes URLs before file paths, and rich-content detection combines URLs with file paths in the same map.

Mitigation: add an explicit `file_path_links_enabled` boolean only around file-path scanning. Tests should assert URLs are still returned when the setting is disabled.

### Risk: stale background detection restores file links after the user toggles off
Terminal grid file scanning and AI/rich-content detection can run off the main path.

Mitigation: guard before enqueue, before scan, and before applying completed results. Abort in-flight AIBlock link-detection work on setting changes where practical.

### Risk: inconsistent behavior between terminal grids and rich content
There are two link-detection systems: grid hover detection and rich-content text detection.

Mitigation: gate both systems from the same `GeneralSettings::file_path_links` value and include manual validation for command output, alt screen, and AI/rich content.

### Risk: overloading `general.link_tooltip`
The existing setting is close in the UI and issue context, but changing its semantics would break users who only want direct-open behavior instead of tooltip behavior.

Mitigation: keep `general.link_tooltip` unchanged and introduce a separate setting with a distinct label, config path, action, telemetry name, and tests.

## Follow-ups
- If users later request finer control, a future change could split file links and directory links or separate grid output from AI/rich content. This spec intentionally starts with one global terminal file-path linkification setting.
