# Automatic Model Fallback Opt-Out — Product Spec
GitHub issue: https://github.com/warpdotdev/warp/issues/10804
Figma: none provided

## Summary
Add a user-facing setting that lets users opt out of Warp automatically falling back to a different model when their selected Warp Agent model is unavailable. The setting defaults to on so existing users keep today's behavior. When it is off, Warp must stop before sending the request to a fallback model and show a clear recoverable error instead.

## Problem
Warp currently documents model fallback as automatic: if the selected model is temporarily unavailable, Warp may continue with another model, potentially from a different provider. That behavior is useful for availability, but it is wrong for users who deliberately selected a specific model for quality, reasoning style, tool-use behavior, compatibility, provider preference, or credit/cost expectations.

The disruptive case is not that a request fails. The disruptive case is that Warp continues successfully on a model the user did not choose, and the user only discovers the substitution after the agent's behavior or credit consumption has changed.

## Goals
- Expose a setting for automatic model fallback in the Agent profile model settings, defaulting to enabled.
- Preserve current behavior for existing and new users unless they explicitly disable the setting.
- When disabled, never silently substitute the active selected base model with a fallback model for a user-initiated Warp Agent request.
- When disabled and the selected model is unavailable, show a clear inline error that names the selected model and gives the user next actions.
- Make the setting understandable from the UI without requiring users to read docs.
- Keep the behavior consistent across normal local agent conversations, profile-specific model selections, and per-conversation model overrides.
- Define validation that covers UI state, persistence, request behavior, and error rendering.

## Non-goals
- Removing model fallback globally. Fallback remains the default behavior.
- Changing which fallback model the server chooses when fallback is enabled.
- Adding provider-specific fallback allowlists or cost ceilings.
- Changing the model picker availability rules for admin-disabled, plan-gated, unsupported, or permanently unavailable models.
- Guaranteeing that every internal auxiliary model call uses the exact same model. The opt-out applies to fallback from the active selected Warp Agent base model for primary agent requests; separately documented auxiliary model use for summarization, code generation, or specialized tools remains out of scope unless it is implemented as a fallback for that selected base model.
- Adding a one-off prompt every time fallback would occur. The requested behavior is a persistent setting, not an interrupting confirmation dialog.
- Updating public docs in this spec PR. Docs should be updated when the implementation ships.

## User experience

### Setting location and default
1. Each Agent profile has an automatic fallback setting in the profile editor's Models section, near the Base model picker and context window control.
2. The setting label is: "Automatic model fallback".
3. The setting description is: "When your selected model is unavailable, Warp may use another model to keep the agent running."
4. The switch is on by default for all profiles, including existing synced profiles, unsynced default profiles, newly created profiles, and CLI/default profiles where the setting is applicable.
5. Turning the switch off updates only that profile. Other profiles keep their own setting values.
6. If the user selects a different active profile in a terminal, the active profile's fallback setting takes effect for that terminal.
7. If the user uses a per-conversation base-model override from the model picker, the active profile's fallback setting still controls whether that explicitly selected model may fall back.

### Behavior when automatic fallback is enabled
1. Behavior matches today.
2. If the selected model is unavailable and Warp/server fallback succeeds, the request may continue on a fallback model.
3. Existing fallback messaging that indicates Warp is using another model should continue to appear when available.
4. No additional confirmation is required.

### Behavior when automatic fallback is disabled
1. If the selected base model is available, the request proceeds normally on that model.
2. If the selected base model is temporarily unavailable and the server would otherwise fall back, the request fails before any fallback model generates agent output.
3. The inline error is shown in the conversation where the request was attempted.
4. The error message must name the selected model when the display name is known. Recommended copy:
   - "{Model display name} is currently unavailable. Retry later or choose another model."
5. If Warp only knows the model ID, the error may use the ID:
   - "{model_id} is currently unavailable. Retry later or choose another model."
6. The failed request must not create agent text, tool calls, file edits, terminal commands, child agents, or other side effects from a fallback model.
7. The user can recover by retrying after the provider/model recovers, turning automatic fallback back on, or manually choosing another model.
8. Retrying without changing the setting or model is allowed and should send the same no-fallback preference again.
9. If the server returns a broader availability error before it can identify a fallback decision, Warp still shows a normal request error. The new setting does not need to convert every model-related failure into the recommended copy.

### Disabled and hidden model states
1. Admin-disabled models remain hidden from the base model dropdown as they are today.
2. Plan-gated or permanently unavailable models continue to follow existing preference reconciliation rules.
3. Transient provider outages should preserve the user's selected model so disabling fallback remains meaningful; Warp should not clear the profile's selected model merely because a provider outage is reported.
4. The setting does not make an unavailable model selectable if the existing picker would not show it.

### Auto models
1. If the selected base model is an "auto" model, automatic fallback opt-out does not turn "auto" into a fixed model. "Auto" remains an explicit choice to let Warp/server select the best available model.
2. The setting applies when a concrete selected model or concrete per-conversation override would otherwise be replaced by fallback.
3. The UI description should not imply that turning fallback off disables all automatic routing done by an "auto" model.

### Local, cloud, and child agent scope
1. Local Warp Agent conversations use the active profile's fallback setting.
2. Local child agents spawned by the Warp Agent inherit the fallback behavior that corresponds to their requested model/profile context. If the parent explicitly asks a child to use a model and the active profile has fallback disabled, the child must not silently fall back from that requested model.
3. Cloud agent runs and remote child agents should receive an equivalent no-fallback preference when the client launches them with a selected model. This requires server support and must not be implemented as a client-only display change.
4. Historical conversations keep rendering the model actually used for prior exchanges. Turning the setting off later does not rewrite existing exchanges that already used a fallback model.

### Error presentation
1. The error appears inline in the agent conversation using the existing agent error rendering pattern.
2. The error is recoverable and should not force the conversation to close.
3. The message should avoid provider blame when the client only knows that the selected model is unavailable.
4. The message should not say Warp used a fallback model, because no fallback model should have been used.
5. If the request was part of child-agent startup, the parent receives a startup failure that includes the selected-model-unavailable message.

## Success criteria
- A user can find and toggle "Automatic model fallback" while editing an Agent profile.
- The setting persists with the profile and syncs consistently across devices through the same profile-sync mechanism as model preferences.
- Existing profiles behave as if the setting is on until a user turns it off.
- With the setting on, a provider outage can still use today's fallback path.
- With the setting off, the same outage produces an inline error instead of fallback-generated output.
- The request path sends the no-fallback preference to the server for primary Warp Agent requests.
- The server enforces the preference; the client is not relying solely on detecting fallback after the fact.
- Retrying, changing models, and changing profiles are all obvious recovery paths.
- Telemetry or logs can distinguish fallback allowed, fallback blocked by user preference, and ordinary request failures.
- Docs can accurately describe both the default fallback behavior and the opt-out.

## Validation
- Unit-test the profile model default, serialization, deserialization of legacy profiles, and setter behavior.
- Unit-test profile editor action handling so toggling the switch updates the active profile and emits the existing profile-updated path.
- Unit-test request construction so a profile with fallback disabled produces a request/config field that disables fallback, while omitted/legacy values default to fallback enabled.
- Unit-test error conversion/rendering for the server's "selected model unavailable and fallback disabled" response.
- Integration-test a mocked or fixture-backed model-unavailable response:
  - fallback enabled: request proceeds through the fallback path and model-used metadata marks fallback.
  - fallback disabled: request ends with the selected-model-unavailable error and no agent actions are executed.
- Manually verify Settings > Agents > Profiles:
  - new profile shows the switch on.
  - toggling the switch off/on persists after closing and reopening settings.
  - switching active profiles changes the effective behavior.
  - a per-conversation model override still respects the active profile's setting.
- Manually verify a cloud/remote run once server support is available, because this issue is labeled as requiring server work.

## Open questions
- What exact server error code should represent "selected model unavailable because fallback was disabled" so the client can render tailored copy instead of a generic transport error?
- Should this setting be available for CLI-created profiles immediately, or should CLI support wait for a dedicated CLI flag/config-field follow-up?
- Should the public Oz Agent API expose the same fallback preference in its agent config schema at the same time as the Warp client?
- Should analytics record only boolean state on requests, or also the model/provider that was blocked from falling back?
