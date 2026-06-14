# GH11681: Tech Spec — Custom inference in-app data flow disclosure

## Problem
Warp's Custom inference UI currently has accurate local-storage intent but incomplete data-flow copy. The Settings description says API keys are stored only on the user's device and used to make provider requests, while the current request path serializes user-provided keys and custom endpoint settings into agent request parameters that are sent to Warp's backend for the server-side agent harness to call the provider.

The implementation should update the app UI copy and link target so the Settings page and custom endpoint modal match the current architecture. It should not change the request transport or custom endpoint storage model.

## Relevant code
- `app/src/settings_view/ai_page.rs:168` — `CUSTOM_INFERENCE_LEARN_MORE_URL`, currently pointing at BYOK-related docs rather than the Custom inference endpoint page.
- `app/src/settings_view/ai_page.rs:7245` — `render_custom_inference_description`, current Settings copy that says keys are stored only on device and used for provider requests.
- `app/src/settings_view/ai_page.rs:7283` — `render_custom_inference_info_icon`, existing hover-only Terms of Service and plan-size tooltip.
- `app/src/settings_view/ai_page.rs (7460-7531)` — `ApiKeysWidget::render`, Custom inference section layout, description placement, provider key editors, endpoint list, and fallback toggle.
- `app/src/settings_view/custom_inference_modal.rs:521` — modal body description shown before endpoint fields.
- `app/src/settings_view/custom_inference_modal.rs (621-693)` — endpoint URL and API key fields; a visible disclosure can be placed before these fields or in the initial modal description.
- `app/src/settings_view/custom_inference_modal.rs:864` — URL validation requires HTTPS and rejects local/private hosts.
- `crates/ai/src/api_keys.rs:252` — `ApiKeyManager::custom_model_providers_for_request`, serializes custom endpoint `base_url`, `api_key`, and model `config_key` into request settings when Custom inference is enabled.
- `crates/ai/src/api_keys.rs (361-383)` — local secure-storage write path for API keys and custom endpoint settings.
- `app/src/ai/agent/api.rs:94` — `RequestParams` includes `api_keys` and `custom_model_providers`.
- `app/src/ai/agent/api.rs (245-248)` — `RequestParams::new` builds BYOK API keys and custom model provider settings from `ApiKeyManager`.
- `app/src/ai/agent/api.rs (329-334)` — request parameters carry `api_keys` and `custom_model_providers` into the agent API request.
- `app/src/ai/llms.rs:1319` — `build_custom_llm_infos`, turns locally stored custom endpoints into selectable model picker entries without changing routing.
- `app/src/settings_view/custom_inference_modal_tests.rs (1-100)` — existing unit coverage for custom endpoint URL and form validation.
- `crates/ai/src/api_keys_tests.rs (158-259)` — existing unit coverage for custom provider request serialization.

## Current state

### Settings UI
`ApiKeysWidget::render` shows a Custom inference section when `FeatureFlag::CustomInferenceEndpoints` is enabled and the current workspace has Custom inference entitlement. The section renders:

1. a "Custom inference" subheader plus a small info icon;
2. `render_custom_inference_description`;
3. provider key editors;
4. a custom endpoint list when endpoints exist;
5. the Warp credit fallback toggle when BYOK or Custom inference is available.

The current description combines BYOK and custom endpoint messaging and says API keys are stored only on device, never on Warp servers, and are used to make provider requests. It links to the old BYOK docs URL constant.

### Modal UI
`CustomEndpointModal::render` starts with a short description:

"Provide your endpoint details below. You can add as many models from the endpoint as you'd like and can also provide aliases for the model picker in your input."

Then it renders endpoint name, endpoint URL, API key, and model rows. There is no visible explanation that Warp's backend, not the local client, will call the endpoint. URL validation rejects local and private hosts, but the UI copy does not explain that the endpoint must be reachable from Warp's backend.

### Data and request flow
`ApiKeyManager` stores provider keys and custom endpoints in secure storage under the `AiApiKeys` key. This supports the "stored only on your device" at-rest claim.

When building an agent request, `RequestParams::new` reads `ApiKeyManager`, checks workspace entitlements, and populates:

- `api_keys` for provider BYOK credentials; and
- `custom_model_providers` for custom endpoint `base_url`, `api_key`, and model slugs/config keys.

Those fields are then sent as request settings to Warp's backend. The backend agent harness uses them in-flight to call the selected provider or custom endpoint. The app code therefore needs disclosure copy that distinguishes secure local storage at rest from server-side routing during requests.

## Proposed changes

### 1. Update the Custom inference docs link
Change `CUSTOM_INFERENCE_LEARN_MORE_URL` to the Custom inference endpoint documentation page:

- `https://docs.warp.dev/agent-platform/inference/custom-inference-endpoint/`

If Product prefers a future combined BYOK/Custom inference page, update the constant to that page instead, but the linked docs must include the in-flight backend routing explanation.

### 2. Replace the Settings description copy
Update `ApiKeysWidget::render_custom_inference_description` to use explicit at-rest and in-flight language.

Recommended copy direction:

- "Use your own API keys from model providers for Warp Agent, or add custom OpenAI-compatible endpoints for third-party models."
- "Your API keys and endpoint configuration are stored only on this device."
- "When you run Warp Agent with a BYOK or custom endpoint model, your key and prompt are sent to Warp's backend in-flight so Warp's server-side agent harness can call your selected provider or endpoint."
- "Warp does not store your API key on its servers."
- "Auto models or models from providers without your API keys still consume Warp credits."

Keep the existing `FormattedTextElement` pattern and retain the Learn more hyperlink as the final fragment.

Implementation notes:

- Avoid adding a new component unless the description becomes too long for inline paragraph copy.
- The description should stay readable at `CONTENT_FONT_SIZE`.
- If the copy is too dense as a single paragraph, use two `FormattedTextLine::Line` entries or render an additional paragraph with the same styling. Prefer this over burying the routing statement in the hover tooltip.

### 3. Add a visible modal disclosure
Update `CustomEndpointModal::render` so Add and Edit flows display a disclosure before the endpoint URL/API key are entered.

Recommended placement:

- Expand the existing top description text to include the routing disclosure; or
- Add a second paragraph immediately after the existing description and before "Endpoint name".

Recommended copy direction:

"Requests to custom endpoints are routed through Warp's backend. Warp sends your endpoint URL, selected model, API key, and prompt to its backend in-flight so the server-side agent harness can call your endpoint. Warp does not store the API key on its servers. Use a publicly reachable HTTPS endpoint."

Implementation notes:

- Use existing `Text::new(...).soft_wrap(true)` and theme nonactive text color to match the current modal.
- Do not rely on an info icon or hover-only tooltip for the core disclosure.
- Keep the modal height stable if possible. The current modal is `560x600`; if text wraps poorly, adjust spacing before changing dimensions.
- The disclosure applies equally to add and edit because both use the same `CustomEndpointModal` render path.

### 4. Preserve request and storage behavior
Do not change:

- `ApiKeyManager` storage schema or secure-storage key;
- custom endpoint serialization in `custom_model_providers_for_request`;
- `RequestParams` fields or agent request wire format;
- model picker behavior for custom models;
- URL validation rules;
- Warp credit fallback behavior.

This issue is a disclosure correctness fix, not a routing architecture change.

### 5. Optional test coverage
There may not be snapshot-style tests for Settings copy. If lightweight coverage exists nearby, add tests that make the disclosure hard to regress. If no suitable UI text snapshot harness exists, keep validation manual and avoid introducing broad test infrastructure for copy-only UI changes.

Reasonable unit-level checks if practical:

- a small helper function that returns the Settings disclosure text and can be asserted to contain "Warp's backend" and "stored only on this device";
- a small helper function for modal disclosure text with similar assertions.

Avoid tests that are brittle around full marketing copy unless the repository already uses text snapshots for these widgets.

## End-to-end flow
1. User opens Settings and navigates to AI settings.
2. `ApiKeysWidget::render` determines that Custom inference should be shown.
3. `render_custom_inference_description` renders the updated copy explaining both local storage and backend in-flight routing.
4. User clicks Learn more and opens the Custom inference endpoint docs.
5. User clicks "+ Add custom model".
6. `CustomEndpointModal::render` displays the visible data-flow disclosure before the endpoint configuration fields.
7. User enters endpoint URL, API key, and model names.
8. Existing validation rejects malformed, non-HTTPS, local, or private endpoints.
9. On save, `ApiKeyManager::add_custom_endpoint` stores the endpoint locally as before.
10. When the user later selects the custom model, `RequestParams::new` continues to send `custom_model_providers` to Warp's backend as before, now matching the in-app disclosure.

## Risks and mitigations

### Risk: Copy is still interpreted as "Warp never receives the key"
Mitigation: Include both phrases in the same disclosure: "stored only on this device" and "sent to Warp's backend in-flight." Do not separate them into different screens or hide routing behind the Learn more link.

### Risk: Copy sounds like Warp permanently stores the key
Mitigation: Explicitly say that Warp does not store the API key on its servers and uses it in-flight to call the endpoint.

### Risk: The modal becomes too text-heavy
Mitigation: Keep the modal copy short and action-oriented. Put detailed privacy and architecture explanation behind the Learn more docs link, but keep the essential routing statement visible.

### Risk: Link target regresses to a docs page that omits routing
Mitigation: Point at the Custom inference endpoint docs page and manually verify the page contains a "How it works" or equivalent routing section.

### Risk: Tests become brittle around copy changes
Mitigation: Prefer manual validation for exact prose, or assert only critical phrases if adding helper-based unit tests.

## Testing and validation
- Run `cargo test -p app custom_inference_modal_tests` or the repository's current equivalent command if the package/test target naming has changed.
- Run `cargo test -p ai api_keys_tests` or the repository's current equivalent for request serialization coverage if nearby changes are made.
- Run `cargo fmt` if code changes touch Rust files.
- Manually open AI Settings with Custom inference enabled and verify:
  - the Settings disclosure mentions local storage and Warp backend routing;
  - the Learn more link opens the Custom inference endpoint docs;
  - provider key editors, endpoint list, and Warp credit fallback still render.
- Manually open Add and Edit custom endpoint modals and verify:
  - the disclosure is visible without hover;
  - existing keyboard navigation still starts at the endpoint name field;
  - valid endpoint save still shows the existing success toast;
  - invalid URL/private host behavior is unchanged.

## Follow-ups
- Decide whether the BYOK-only Settings state should receive the same visible in-app disclosure when Custom inference endpoints are not enabled.
- Consider a future dedicated privacy/data-flow explainer component shared by BYOK, Custom inference endpoints, and BYOLLM.
- Consider future architecture work for local-only or direct client-to-provider inference only if Product decides that is a supported mode; it is outside this disclosure fix.
