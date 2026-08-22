# APP-4978 — Surface the active model under the Warping indicator

*Spec: Show the resolved model and router configuration link while Agent Mode is responding*

## Product

### Summary

When a GUI Agent Mode response is in progress and the selected base model is a model router, the existing `Warping...` indicator will identify the concrete model selected for that turn. A custom router also exposes a configuration link: local routers open their YAML source in Warp's editor, while cloud/team routers navigate to the Warp Agent settings surface. Built-in auto routers show only the resolved model name. Ordinary non-router turns retain the current implicit `Warping...` text.

The complete behavior is gated by a new feature flag. Turning that flag off must leave the existing fallback-model messaging (including its independent `FallbackModelLoadOutputMessaging` gate) unchanged.

### Key design choices

1. Classify the selected model from the client-side base-model ID, and use the already-streamed per-exchange `OutputModelInfo` for the resolved model; no warp-server or protocol change is required.
2. Restrict the new display to turns whose selected model is a custom router or built-in auto router. Do not change the normal `Warping...` experience for direct model selections.
3. Link only custom routers. Local links reuse the existing `source_path`/Warp editor flow; cloud/team links reuse settings navigation. Built-in auto routers have no link.
4. Preserve the existing footer layout, clipping, shimmer, fallback explanation, and secondary-tip behavior. The resolved-model link is an inline, action-capable formatted-text element rather than a new footer row.

### Behavior invariants

1. While a router-selected GUI Agent Mode exchange is in progress and `OutputModelInfo` is available, the primary indicator names the resolved concrete model using its display name; if the display name is empty, the model ID is the deterministic fallback.
2. A custom local router renders one configuration affordance. Activating it opens that router's `CustomModelRouter.source_path` in Warp's code editor using the same behavior as the settings view's existing “Open file” action. A missing `source_path` produces no link and does not make the indicator fail.
3. A custom cloud/team router renders one configuration affordance that dispatches existing settings navigation to the Warp Agent section, with the router name/ID supplied as the settings search query where supported. It must not open an external URL or require a server API change.
4. A built-in auto router renders the resolved model name but no configuration link.
5. A direct, non-router selected model continues to render the current implicit `Warping...` text, even when the exchange's `ModelUsed` message contains a model name.
6. Before the current exchange receives `ModelUsed`, the indicator remains safe and deterministic: it shows the normal `Warping...` text unless the existing follow-up lookback rules provide an eligible previous model. A fresh user query must never display stale model information from an earlier exchange.
7. If the new feature flag is disabled, all new router display/link behavior is absent. Existing fallback messaging remains governed solely by `FallbackModelLoadOutputMessaging`, including the current fallback text, explanation, previous-exchange lookback, and no-fallback behavior.
8. The resolved-model affordance remains within the existing footer's clipping and width constraints. It must be selectable/clickable in the same way as other action hyperlinks, and it must not displace stop, auto-execute, queue, control, or response-hiding controls.
9. The scope is GUI status-bar rendering only. TUI parity, router configuration editing UX, and server-side router metadata are follow-up work.

## Technical design

### Current context (pinned to `f7693d9930c21af1677a9b44b3fba20f88213ba9`)

- `app/src/ai/agent/conversation.rs:2953-2973` handles `api::message::Message::ModelUsed` and stores the server-provided model ID, display name, fallback bit, and cache expiry on the current exchange.
- `app/src/ai/agent/mod.rs:459-471` defines `OutputModelInfo`, the per-exchange client record consumed by the GUI.
- `app/src/ai/blocklist/block/status_bar.rs:772-927` determines whether the latest exchange is actively warping, reads `output_to_render().model_info`, and supplies the primary and secondary footer elements.
- `app/src/ai/blocklist/block/status_bar.rs:997-1018` implements the immediate-previous-exchange lookback used for follow-up exchanges.
- `app/src/ai/blocklist/block/status_bar.rs:1127-1158` contains the fallback-only `resolve_fallback_warping_message` helper. It currently exits when `FallbackModelLoadOutputMessaging` is disabled and otherwise returns `Warping with {name}.` only for fallback output.
- `app/src/ai/blocklist/block/view_impl/common.rs:157-203` defines `WarpingProps`, including the primary text and optional secondary element; `:203-364` passes those values to the common renderer.
- `app/src/ai/blocklist/block/view_impl/common.rs:482-551` renders the footer, including the fixed-height/clipped secondary line and existing action controls. `FormattedTextElement` action-link support is already used by the status-bar agent-tip renderer.
- `app/src/ai/custom_model_routers.rs:105-151` stores local router `source_path`; `:285-311` provides `is_auto_target`, `is_custom_router_id`, `is_local_custom_router_id`, and `is_cloud_custom_router_id`.
- `app/src/ai/llms.rs:927-1000` gates cloud/team router choices and includes them in Agent Mode choices; `:1148-1169` resolves a local router from its `LLMId`.
- `app/src/settings_view/custom_router_view.rs:30-75,235-257` implements the existing local-router “Open file” action and emits `OpenFile(PathBuf)`.
- `app/src/workspace/view.rs:17776-17808` implements `open_custom_router_file`, opening the YAML in Warp's own editor.
- `app/src/settings_view/mod.rs:246-386` defines `SettingsSection::WarpAgent` and the existing settings navigation model.
- `crates/warp_features/src/lib.rs:346-350` defines `FallbackModelLoadOutputMessaging`; `:890-950` contains the feature-state arrays and channel lists. `app/Cargo.toml:581-922` and `app/src/features.rs:224-225` show the existing Cargo-feature-to-`FeatureFlag` plumbing.
- `app/src/ai/blocklist/block/view_impl/common_tests.rs:172-195` demonstrates the pure rendering-helper unit-test seam.

### Proposed changes

1. Add a new descriptive `FeatureFlag` variant for the resolved-router warping indicator, a matching `app` Cargo feature, and the app feature mapping. Keep the new flag separate from `FallbackModelLoadOutputMessaging`; do not change the fallback flag's meaning or call sites.
2. Generalize the status-bar model-resolution seam without conflating fallback and router behavior. A resolver should:
   - first determine the selected base model ID from `model.base_model(app)`;
   - classify it with `custom_model_routers::is_custom_router_id` and the built-in portion of `is_auto_target`;
   - read the current `OutputModelInfo`, applying the existing one-exchange follow-up lookback only where the current helper permits it;
   - return a display label and an optional link target only for a router-selected turn and only when the new flag is enabled.
3. Keep fallback resolution semantically independent. The status-bar caller may combine the results into one `default_warping_text`, but disabling the new flag must still execute the current fallback helper exactly as before, and fallback explanations must still win over tips when they do today.
4. Represent link targets as typed action data that can be consumed by the existing `FormattedTextFragment::hyperlink_action` / `register_default_click_handlers_with_action_support` path:
   - local custom router: add or reuse a workspace action that calls `open_custom_router_file(source_path)` rather than duplicating file-opening logic;
   - cloud/team custom router: dispatch `WorkspaceAction::ShowSettingsPageWithSearch { section: Some(SettingsSection::WarpAgent), search_query: router label or ID }`.
   The implementation must make the link text accessible and concise (for example, “Configure router”), and must avoid exposing the local filesystem path in the footer.
5. Extend `WarpingProps`/the common renderer only as needed to carry the formatted primary content or inline link. Preserve the current footer height calculation and `ClipConfig::ellipsis()` behavior; do not add a second status row.
6. Add focused pure tests around the resolver/link-target builder and retain/update the existing `warping_footer_height` tests. The tests should use feature-flag overrides and synthetic model IDs/output metadata where possible, avoiding a live server.
7. Do not modify warp-server, `ModelUsed`, protobuf definitions, TUI status rendering, or router configuration storage in this PR. A future server enhancement may stamp a router `config_key` onto `ModelUsed`, but it is not needed for this client-scoped change.

### Design alternatives and tradeoffs

- **Show the resolved model on every turn vs. router-only:** Showing it everywhere would alter ordinary Agent Mode messaging and add noise. Router-only is selected because the feature answers “which branch did the router choose?” while preserving direct-model behavior.
- **Stamp router identity in `ModelUsed` server-side vs. infer it from the selected base model:** Server stamping would be more authoritative but expands this single-repository task into protocol/server work. Client inference is sufficient to decide whether a turn was router-selected; the existing `model_id`/display name already identifies the resolved model. Server stamping remains future work if inference proves insufficient.
- **Reuse the fallback helper vs. create a router-aware resolver:** Reusing fallback logic directly risks making the new flag alter fallback behavior. A small router-aware resolver composed beside the fallback helper preserves independent gates and makes router-only tests explicit.
- **Link built-in auto routers to docs/settings vs. no link:** Built-in auto is product-owned behavior rather than an editable user configuration. No link avoids a misleading destination.
- **Open local YAML externally vs. use Warp's editor:** External opening would diverge from the existing custom-router settings flow and platform behavior. Reusing `open_custom_router_file` keeps the action in Warp and honors the existing source-path pattern.
- **Add GUI and TUI parity now vs. GUI only:** The ticket explicitly limits this change to the GUI warping indicator. TUI parity would introduce another renderer and test surface, so it is out of scope.

### Open questions resolved / assumptions

- The selected model is the model returned by `AIBlockModel::base_model(app)` for the active exchange; it is not inferred from the resolved model ID alone. This is what distinguishes a router-selected turn from an ordinary turn.
- The display label is `OutputModelInfo.display_name`; if it is empty, use `OutputModelInfo.model_id` as a stable user-visible fallback. If both are empty/unavailable, keep `Warping...`.
- The existing immediate-previous-exchange lookback may be reused for agent-initiated follow-ups, but never for a fresh user query. This prevents stale router labels while retaining current anti-flicker behavior.
- “Cloud/team settings surface” means the existing Warp Agent settings section and search navigation. If the current settings page cannot focus a router by ID/name, the implementation may add the minimal search-target plumbing in the client; it must not invent a new server endpoint.
- Local-router links are conditional on `source_path` and the local filesystem build configuration. Non-local builds and pathless test fixtures render no local link but still render the resolved model where available.
- The flag is expected to start disabled outside the normal feature-flag rollout mechanism. Its exact rollout channel is an implementation/release decision, but all code paths must be runtime-gated.
- The requester has explicitly settled built-in-auto behavior (name, no link), router-only scope, GUI-only scope, custom-router link destinations, and feature-flag gating; no further product decision is required before implementation.

## Validation and verification criteria

All criteria below must pass before the implementation PR is marked ready.

1. **Resolver unit coverage — router classification.** Add deterministic tests proving that local custom IDs, cloud custom IDs, and built-in auto IDs are eligible; direct model IDs are ineligible. Check with the targeted `warp` library test command and ensure the tests fail if the selected-model classification is removed.
2. **Resolver unit coverage — resolved display.** Test current `OutputModelInfo` with a non-empty display name, empty display name with a model ID fallback, and missing model info. Verify the expected label (`Warping with <name>.` or the agreed equivalent) and safe fallback to `Warping...`.
3. **Resolver unit coverage — stale-data rules.** Test that an agent-initiated follow-up may use the immediately previous exchange under the existing lookback rule, while a fresh user query never uses that previous model. Test that only one previous exchange is consulted.
4. **Link-target unit coverage.** Test local custom routers with and without `source_path`, cloud/team custom routers, and built-in auto routers. Assert that local targets carry the exact path to the existing Warp-editor action, cloud targets carry `SettingsSection::WarpAgent` plus a deterministic search query, and built-in auto/direct models have no link.
5. **Feature-gate regression coverage.** With the new flag disabled, assert no router display/link is returned. Independently toggle `FallbackModelLoadOutputMessaging` and assert the existing fallback message, explanation selection, and fallback lookback remain unchanged in all four combinations of the two flags.
6. **Rendering regression coverage.** Preserve `warping_footer_height` coverage and add a render/helper assertion that a link is inline with the primary status text, remains selectable/action-capable, and does not add a third footer line. Confirm existing stop/auto-execute/queue/control/hide-response controls still render in the same props path.
7. **Feature plumbing checks.** Verify the new flag is declared once in `crates/warp_features`, mapped once in `app/Cargo.toml` and `app/src/features.rs`, and can be overridden in tests. Run `cargo check -p warp` (with the repository's normal local feature set) and `cargo test -p warp --lib` for the affected modules.
8. **Repository checks.** Run `./script/presubmit` from the repository root, or the repository-maintained equivalent if the script requires platform-specific setup; formatting, linting, compilation, and tests must pass without unrelated changes.
9. **GUI happy path — built-in auto.** Build the client, enable the new flag in the test/dev channel, select a built-in auto router, submit an Agent Mode request, and use computer interaction to observe the live GUI while it is responding. Capture a screenshot and/or video showing the existing in-progress indicator with the concrete resolved model name and no configuration link.
10. **GUI happy path — local custom router.** Configure/select a local YAML router with a known `source_path`, start a response, and use computer interaction to capture the indicator showing the resolved model and custom-router link. Activate the link and capture evidence that the YAML opens in Warp's code editor.
11. **GUI happy path — cloud/team custom router.** In an account/team context with a cloud router available, start a response and capture the indicator showing the resolved model and configuration link. Activate it and capture evidence that Warp navigates to the Warp Agent settings surface/search rather than an external URL.
12. **GUI negative paths.** Capture evidence for a direct-model turn showing unchanged `Warping...`, a router turn before `ModelUsed` arrives showing a safe loading state, and a router turn with missing/pathless local configuration showing no broken link. Capture one flag-off run proving the pre-feature display and fallback messaging are unchanged.
13. **Visual evidence attachment.** Attach the computer-use screenshots/video from criteria 9–12 to both Linear issue `APP-4978` and the implementation PR. The PR description must state which evidence maps to each criterion; visual proof is required in addition to automated tests.
14. **Scope audit.** Before merge, review the diff to confirm it changes only the warp client GUI/feature plumbing/tests, contains no warp-server/proto/TUI changes, and does not expose local filesystem paths in rendered footer text.
