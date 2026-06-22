# Ask-User-Question Autonomy Speedbump — Technical Notes

Linear: [QUALITY-512](https://linear.app/warpdotdev/issue/QUALITY-512/add-ask-user-question-permission-speedbump)
Product spec: `specs/QUALITY-512-ask-user-question-speedbump/PRODUCT.md`

## 1. Overview
This change adds a one-shot Ask-User-Question autonomy speedbump to the existing `AIBlock` speedbump infrastructure. The speedbump installs a compact footer into `AskUserQuestionView`, backed by a `Dropdown<AIBlockAction>` that updates the active execution profile's `ask_user_question` permission.
## 2. Key files
- `app/src/settings/ai.rs` — defines `should_show_agent_mode_ask_user_question_speedbump` as a private local-only setting with `SyncToCloud::Never`.
- `app/src/ai/blocklist/block.rs` — seeds the new speedbump variant, owns the dropdown view, syncs the footer into `AskUserQuestionView`, handles dropdown actions, and consumes the one-shot flag after successful footer installation.
- `app/src/ai/blocklist/block/view_impl.rs` — renders the shared dropdown speedbump footer row.
- `app/src/ai/blocklist/inline_action/ask_user_question_view.rs` — owns the Ask-User-Question card chrome and renders the attached footer in completed/collapsed states.
- `app/src/server/telemetry/events.rs` — adds `ChangedAgentModeAskUserQuestionPermission`.
- `app/src/terminal/block_list_element.rs` — forwards covered mouse events to visible rich-content overlays so dropdown menus remain interactive.
- `crates/warpui_core/src/elements/dismiss.rs` — consumes underlay clicks for dismissable overlays that prevent interaction with other elements.
- `crates/warpui_core/src/elements/selectable_area.rs` — avoids starting terminal selection for mouse down events covered by higher-z-index overlays.
- `app/src/ai/blocklist/block_tests.rs` — covers permission index mapping, first Ask-User-Question action detection, and setting defaults/round-trip.
## 3. Setting
The new setting is intentionally local-only:
- Name: `should_show_agent_mode_ask_user_question_speedbump`
- Default: `true`
- Private: `true`
- Sync: `SyncToCloud::Never`

This keeps the speedbump display tied to the local client state. It also avoids cross-device races where one device could consume the onboarding display for another device.
## 4. Speedbump state
`AutonomySettingSpeedbump` now has:
- `ShouldShowForAskUserQuestion { action_id, shown }`

The `action_id` pins the footer to the Ask-User-Question action that triggered it. The `shown` field is set when the footer is attached, matching the broader speedbump pattern and making the attachment state explicit.
## 5. Trigger and one-shot consumption
`AIBlock::handle_complete_output` uses `first_ask_user_question_action_id(output)` to find the first Ask-User-Question action in the completed agent output. It seeds the speedbump when the feature flag is enabled, autonomy is allowed, and the local one-shot setting is still `true`.

Unlike the original design, auto-approve is not excluded. Auto-approve skipped Ask Question actions can seed the same first-use footer.

The local one-shot flag is not consumed at seed time. `sync_ask_user_question_speedbump_footer` returns `true` only after it finds a matching `AskUserQuestionView` and installs the footer. Callers then invoke `mark_ask_user_question_speedbump_as_shown` only on that successful path. If output completion happens before the view exists, the flag remains `true`; `handle_ask_user_question_stream_update` calls the same sync helper after creating/replacing the view and consumes the flag when installation succeeds.
## 6. Dropdown ownership and action flow
`AIBlock` owns `ask_user_question_speedbump_dropdown: Option<ViewHandle<Dropdown<AIBlockAction>>>`. The dropdown is created lazily and reused for the block lifetime.

Dropdown items dispatch `AIBlockAction::SetAskUserQuestionSpeedbumpPermission(permission)`. The handler:
- Resolves the active execution profile for the terminal view.
- Calls `AIExecutionProfilesModel::set_ask_user_question`.
- Emits `ChangedAgentModeAskUserQuestionPermission` with source `Speedbump`.
- Marks the local one-shot setting false idempotently.
- Clears the speedbump state and footer from the current Ask-User-Question view.
- Notifies the block so the footer hides immediately.

Profile model events refresh the dropdown selected index so external settings changes are reflected while the footer exists.
## 7. Footer rendering
The footer is threaded into `AskUserQuestionView` rather than wrapping the child view externally. This keeps the Ask-User-Question card in charge of its own border, radius, and layout.

`AskUserQuestionView` renders the footer as an attached bottom strip:
- Reduced vertical padding for a compressed speedbump height.
- Compact dropdown sizing.
- Bottom radius applied to the footer strip.
- Main card/header radius flattened at the bottom when a footer is present.
- Footer support for collapsed and expanded completed states.

This avoids a double-rounded seam between the card body and the speedbump footer.
## 8. Overlay event routing
The dropdown menu is rendered through WarpUI overlay infrastructure. A covered mouse event previously could be rejected by `BlockListElement` before the rich-content overlay received it, allowing terminal selection or surrounding content to intercept dropdown clicks.

The fix has three parts:
- `BlockListElement` forwards covered mouse events to visible rich-content views before returning.
- `SelectableArea` does not start a selection when a mouse down event is covered at its z-index.
- `Dismiss::prevent_interaction_with_other_elements` consumes mouse events that hit its underlay.

Together these keep dropdown options clickable and make outside-click dismissal reliable.
## 9. Telemetry
The new telemetry event is `ChangedAgentModeAskUserQuestionPermission` with fields:
- `src: AutonomySettingToggleSource`
- `new: AskUserQuestionPermission`

The speedbump path emits the event with `src = Speedbump` whenever the user selects a dropdown option.
## 10. Validation
Local validation:
- `cargo check -p warp`
- `cargo check --all-targets -p warp`
- `cargo nextest run --no-fail-fast -p warp ask_user_question`
- `cargo fmt --all`
- `cargo clippy --workspace --all-targets --all-features --tests -- -D warnings`
- `git --no-pager diff --check`
## 11. Follow-ups
- Add deeper regression coverage for overlay routing if a convenient harness exists for rich-content overlays.
- Consider a shared compact dropdown speedbump component if future autonomy speedbumps need dropdowns.
- Consider an internal reset affordance for local-only onboarding speedbumps to simplify QA.
