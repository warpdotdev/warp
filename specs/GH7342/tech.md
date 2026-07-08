# GH7342: Tech Spec — Customizable Spinner Verbs
## 1. Context
The implementation customizes only the generic `Warping...` loading label for Warp Agent and Oz. App-launch/startup splash screens and application boot loading surfaces are explicitly out of scope, even if they use similar copy.
Relevant code:
- `app/src/settings/ai.rs` defines the public `spinner_verbs` setting at `agents.warp_agent.spinner_verbs` and companion custom list at `agents.warp_agent.custom_spinner_verbs`.
- `app/src/ai/loading/warping_verb.rs` owns `DEFAULT_WARPING_VERB`, normalization constants, normalization helpers, render-time formatting, and `WarpingVerbSelector`.
- `app/src/ai/loading/warping_verb_pack.rs` owns the read-only built-in packs.
- `app/src/ai/blocklist/block/status_bar.rs` owns a `WarpingVerbSelector` and passes the resolved generic fallback string into `WarpingProps`.
- `app/src/ai/blocklist/block/view_impl/common.rs` keeps action-specific loading labels ahead of `props.default_warping_text`.
- `app/src/settings_view/ai_page.rs` renders the Settings UI mode buttons and custom editor.
## 2. Settings Contract
The persisted user-facing source setting is:
- Name: `spinner_verbs`
- TOML path: `agents.warp_agent.spinner_verbs`
- Type: `SpinnerVerbsMode`
- Default: `default`
- Supported platforms: all
- Sync: global, respecting the user's sync preference
- Feature flag: `FeatureFlag::CustomWarpingVerbs`
Valid values are `default`, `medieval`, `conspiracy`, `cooking`, `warpy`, and `custom`.
The companion custom-list setting is:
- Name: `custom_spinner_verbs`
- TOML path: `agents.warp_agent.custom_spinner_verbs`
- Type: `SpinnerVerbList` in memory, serialized as a TOML string array
- Default: empty list
- Supported platforms: all
- Sync: global, respecting the user's sync preference
- Feature flag: `FeatureFlag::CustomWarpingVerbs`
`default` resolves to `DEFAULT_WARPING_VERB` (`Warping...`). Pack modes resolve from source at render time. `custom` uses the normalized `custom_spinner_verbs` list, falling back to `Warping...` if the normalized list is empty.
## 3. Normalization
`custom_spinner_verbs` uses `SpinnerVerbList`, which stores `NormalizedSpinnerVerb` entries in memory while preserving the public TOML shape as an array of strings. Settings-file reads, serde deserialization, and programmatic writes normalize through this wrapper.
`AISettings::set_custom_spinner_verbs` is the canonical write path for Settings UI and agent-driven custom-list updates. It constructs a `SpinnerVerbList` and switches `spinner_verbs` to `custom`.
`WarpingVerbSelector::resolve_from_verbs` also normalizes before picking a display value as defense-in-depth for preset packs and test/raw sources that enter the renderer as plain strings.
The selector caches the normalized source list and recomputes it only when the raw candidate list changes. A cache hit avoids repeated normalization across renders or sessions that use the same source list while still preserving renderer-boundary safety.
Normalization rules are implemented in `app/src/ai/loading/warping_verb.rs`:
- Trim each entry.
- Strip trailing `.` and `…` characters.
- Drop entries that are empty after trimming and stripping.
- Truncate each entry to `MAX_WARPING_VERB_CHARS`.
- Keep at most `MAX_CUSTOM_WARPING_VERBS` entries.
`format_for_display` appends `...` unless the normalized selected phrase already ends with `.`, `!`, `?`, or `…`.
## 4. Built-In Packs
`WarpingVerbPack` defines four read-only packs:
- `medieval`
- `conspiracy`
- `cooking`
- `warpy`
Packs expose:
- `all()` for Settings UI display order.
- `display_name()` for button labels.
- `identifier()` for natural-language setting descriptions and skill routing.
- `verbs()` for static source values.
- `verbs_as_vec()` for resolving pack mode into display candidates.
Agents handling natural-language "spinner verbs", "warping verbs", or "flavor verbs" requests should modify `agents.warp_agent.spinner_verbs` and, for custom lists, `agents.warp_agent.custom_spinner_verbs` through the settings path. They should not edit source pack definitions unless the developer explicitly asks to change built-in packs.
## 5. Rendering Flow
`BlocklistAIStatusBar` stores a `WarpingVerbSelector`.
When rendering the latest exchange:
1. `resolve_fallback_warping_message` is evaluated first.
2. If fallback-model text exists, that exact model-specific text becomes `default_warping_text`, and the fallback explanation is rendered as secondary content.
3. Otherwise, `warping_verb_session_key_for_exchange` chooses a stable session key for the current LLM response from the active response stream or exchange.
4. `WarpingVerbSelector::resolve` resolves the configured source mode, returns one selected phrase for that session, or `DEFAULT_WARPING_VERB` if the feature is disabled or the resolved list is empty.
5. The resolved string is passed to `render_warping_indicator` as `WarpingProps::default_warping_text`.
`render_warping_indicator` keeps all specific states above the generic fallback, including summarization, document generation/update, passive code diff, file edit request, ask-user-question preparation, web search, review-comment fetching, interrupt adjustment, codebase search, grep, MCP call/resource read, file glob, writing to long-running commands, command execution, command waiting, and waiting-for-user-input states.
Only the final generic fallback branch uses `props.default_warping_text`.
## 6. Selector Semantics
`WarpingVerbSelector` caches:
- `session_key`
- raw selected phrase
- formatted display phrase
A warping session is one LLM response or active response stream, not a terminal session or full conversation. If the same session key renders again, it returns the cached display phrase. When the session key changes, it picks from the normalized list. With more than one candidate, it filters out the previously selected raw phrase when possible to avoid an immediate repeat. A single conversation can include multiple warping sessions and may show different spinner verbs across separate responses.
The selector clears its cache and returns `DEFAULT_WARPING_VERB` when `FeatureFlag::CustomWarpingVerbs` is disabled or the normalized list is empty.
## 7. Settings UI
`AISettingsPageView` tracks `spinner_verb_mode` with:
- `Default`
- `Pack(WarpingVerbPack)`
- `Custom`
The mode is derived directly from `AISettings::spinner_verbs`, so Warp remembers whether the user selected Default, a named pack, or a custom list even if a pack's source phrases later change.
The `Spinner verbs` widget renders mode buttons and either:
- a default preview,
- a pack preview,
- or a custom comma-separated editor.
Custom editor behavior:
- User edits switch the local mode to `Custom` and set `custom_spinner_verb_editor_has_user_edits`.
- Blur and Enter save through `save_custom_spinner_verbs_from_editor`.
- Selecting Default or a pack clears the dirty flag and hides the inactive editor, but does not clear the saved custom list.
- Selecting Custom restores the saved custom list into the editor and switches `spinner_verbs` to `custom`.
- External `AISettingsChangedEvent::SpinnerVerbs` or `AISettingsChangedEvent::CustomSpinnerVerbs` updates mode and editor text only when there are no unsaved user edits.
This prevents stale custom editor text after external updates while also avoiding persistence on every keystroke.
## 8. Settings File Behavior
Example:
```toml
[agents.warp_agent]
spinner_verbs = "custom"
custom_spinner_verbs = ["Sautéing", "Braising", "Fermenting"]
```
Pack example:
```toml
[agents.warp_agent]
spinner_verbs = "cooking"
```
The custom list is inactive while `spinner_verbs` is `default` or a pack identifier, so switching modes preserves any saved custom list unless the user explicitly clears it.
Settings hot reload uses the existing public-settings path. Raw values are normalized when read into `SpinnerVerbList` and are normalized defensively again at render time.
Invalid or unsupported values should follow the repository's existing settings-file error behavior. Valid arrays that normalize to an empty list fall back to `Warping...`.
## 9. Tests and Validation
Unit coverage should include:
- `normalize_warping_verb` trimming, trailing-dot stripping, dots-only dropping, casing preservation, unicode safety, and truncation.
- `normalize_warping_verbs` empty filtering and max-list capping.
- `SpinnerVerbList` settings-file and serde deserialization normalization.
- selector default fallback when the feature is disabled or list is empty.
- selector per-response stability.
- selector no-immediate-repeat behavior when alternatives exist.
- selector renderer-boundary normalization for raw settings/synced values.
- selector normalized-source cache invalidation when raw settings/synced values change.
- pack identifiers, display names, and pack contents.
Manual validation should cover:
- Settings UI mode switching.
- Custom editor save on blur/Enter.
- External settings-file edits while the custom editor has and does not have unsaved changes.
- Generic warping text in local Warp Agent and Oz/cloud-agent flows.
- Specific status messages and fallback-model messages overriding custom verbs.
Suggested targeted commands:
- `cargo test --manifest-path /Users/erica/warp/Cargo.toml -p warp warping_verb`
- `cargo test --manifest-path /Users/erica/warp/Cargo.toml -p warp warping_verb_pack`
- `cargo test --manifest-path /Users/erica/warp/Cargo.toml -p warp spinner_verbs`
Before pushing PR updates, run formatting and clippy using the repository-standard commands.
## 10. Risks and Mitigations
- Risk: custom verbs replace useful status labels. Mitigation: keep custom text only at the final generic fallback branch.
- Risk: raw synced/settings-file values bypass UI normalization. Mitigation: normalize through `SpinnerVerbList` when settings are read or deserialized, then normalize again in `WarpingVerbSelector::resolve_from_verbs` as defense-in-depth and cache by raw source list to avoid repeated work.
- Risk: the Settings UI shows stale custom text after external updates. Mitigation: track unsaved user edits and sync editor text only when safe.
- Risk: phrase changes reset shimmer every render. Mitigation: cache one display phrase per session key.
- Risk: natural-language agents modify third-party spinner config instead of Warp's setting. Mitigation: document `agents.warp_agent.spinner_verbs` and `agents.warp_agent.custom_spinner_verbs` as the canonical handler in the setting descriptions and bundled settings skills.
## 11. Follow-ups
- Product/Design can revise built-in pack contents before broader rollout.
- A future feature can add a separate hidden/suppress mode if users request it.
- A future feature can address app-launch/startup splash copy if Product wants that surface to be configurable.