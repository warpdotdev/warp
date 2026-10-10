# GH11681: Custom inference in-app data flow disclosure — Product Spec

## Summary
Update Warp's in-app Custom inference experience so it accurately explains how user-provided model credentials and prompts are handled. The desired outcome is that users configuring BYOK or a custom inference endpoint understand both facts at the point of configuration: keys are stored locally at rest, and when an endpoint-routed model is used, the key, prompt, and response transit Warp's backend in-flight because Warp's agent harness runs server-side.

The implementation should prioritize clear disclosure in the existing Settings UI and custom endpoint modal. This spec does not require changing the current server-routed agent architecture.

## Problem
The current Settings copy says: "API keys are stored only on your device, never on Warp's servers. They're used to make requests to your chosen model provider." Users can reasonably read that as meaning Warp never receives their key or prompt when Custom inference is used. The public docs now explain the backend routing path, but the app does not make the same distinction between local storage at rest and in-flight transmission through Warp's backend.

This creates a privacy and trust issue for users who configure a custom endpoint expecting direct client-to-provider network traffic. The issue discussion clarified that the Oz agent harness is not open source and requires traffic to route through Warp's server; the agreed product need is to make that behavior explicit in the app for users who do not read the docs first.

## Goals
- Make the Custom inference Settings copy truthful and unambiguous about local storage and in-flight routing through Warp's backend.
- Show the disclosure before a user saves a custom endpoint API key.
- Preserve the current positive value proposition: users can bring their own model provider or OpenAI-compatible endpoint, and Warp does not store their endpoint API key on Warp servers.
- Distinguish "stored on Warp's servers" from "sent to Warp's backend during requests" in plain language.
- Keep the Learn more link pointed at documentation that describes the Custom inference endpoint data flow.
- Avoid alarming or legalistic language while still being precise about what data transits Warp.

## Non-goals
- Changing Warp Agent or Oz harness routing from server-side to direct client-to-provider calls.
- Adding a local-only custom inference harness.
- Changing secure storage behavior for API keys at rest.
- Changing backend retention, logging, telemetry, or Zero Data Retention policy.
- Changing billing or credit fallback behavior.
- Updating public docs as part of this repository change, except for ensuring the in-app link opens the most relevant existing docs page.
- Adding a consent checkbox or blocking migration flow for users who already configured endpoints.

## Figma / design references
Figma: none provided.

The change should follow the existing Settings page and modal visual language. If design review later provides final copy or layout guidance, update this product spec to match the approved language.

## User experience

### Settings disclosure
When Custom inference is available and the user sees the Custom inference section in Settings, the section description must clearly communicate:

- users can provide their own API keys and custom OpenAI-compatible endpoints for Warp Agent;
- API keys are stored only on the user's device at rest, using the OS secure storage mechanism;
- when the user sends an agent request with a BYOK or custom endpoint model, Warp sends the API key and prompt to Warp's backend in-flight so the server-side agent harness can call the selected provider or endpoint;
- Warp does not store the API key on Warp's servers after the request;
- Auto models or provider models without user-provided keys still use Warp credits;
- the Learn more link opens the Custom inference endpoint or BYOK documentation that explains the data flow in more detail.

The copy must not imply that the local Warp client calls the configured endpoint directly. It also must not imply that "stored only on your device" means the key is never transmitted to Warp during a request.

### Custom endpoint modal disclosure
When adding or editing a custom endpoint, the modal must include a visible disclosure near the endpoint setup instructions or before the API key field. The disclosure must communicate:

- custom endpoint requests route through Warp's backend, not directly from the local client to the endpoint;
- the endpoint URL, selected model identifier, API key, prompt content, and response content may transit Warp's backend in-flight;
- Warp uses the endpoint API key only to call the configured endpoint and does not store it on Warp servers;
- endpoint URLs must be publicly reachable HTTPS endpoints because Warp's backend needs to call them.

The disclosure should be visible without requiring hover. The existing info icon can remain for Terms of Service or plan eligibility details, but it is not sufficient as the only data-flow disclosure.

### Existing endpoint list and editing
Users with existing custom endpoints should see the updated Settings disclosure whenever the section renders. Editing an existing endpoint should show the same modal disclosure as adding a new endpoint. No special migration dialog is required.

### Disabled or unavailable states
If Custom inference is disabled by entitlement, feature flag, or global AI settings, the new disclosure should not introduce enabled-looking controls. Text colors and disabled states should remain consistent with the rest of the AI Settings page.

If the app falls back to the legacy API Keys header because Custom inference is unavailable, this issue does not require adding a Custom inference-specific disclosure there. The implementation may still improve BYOK copy if it uses the same server-routed request path and can do so without broadening scope.

### Error states and validation
The existing custom endpoint validation remains unchanged:

- endpoint URL is required before save;
- endpoint URL must use HTTPS;
- local or private hosts are rejected;
- at least one model name and a non-empty API key are required.

The new disclosure should help explain the private-host restriction: Warp's backend calls the endpoint, so local/private addresses are not valid custom endpoint targets.

### Tone and terminology
Use user-facing terms already present in Warp:

- "Warp Agent"
- "Custom inference"
- "custom endpoint"
- "Warp's backend"
- "API key"
- "Warp credits"

Avoid vague phrases such as "never leaves your device" or "directly connects to your provider" unless they are explicitly scoped to storage at rest.

## Success criteria
1. The Custom inference Settings description no longer states or implies that API keys are only ever used locally.
2. The Settings description explicitly states that BYOK or custom endpoint requests route through Warp's backend in-flight.
3. The Settings description explicitly states that API keys are stored locally at rest and not stored on Warp servers.
4. The Add custom endpoint modal includes visible, non-hover-only copy explaining server-side routing before the user can save a new endpoint.
5. The Edit custom endpoint modal shows the same data-flow disclosure as the Add flow.
6. Users can still add, edit, remove, and select custom endpoint models exactly as before.
7. Existing URL, API key, and model validation behavior is unchanged.
8. The Learn more link opens documentation that covers Custom inference endpoint routing through Warp's backend.
9. The disclosure does not introduce a new consent gate, migration prompt, or blocking flow for existing endpoint users.
10. The copy does not use closing or misleading privacy language such as "never sent to Warp" for API keys or prompts.

## Validation
- Open Settings with Custom inference enabled and verify the Custom inference section states both local storage at rest and in-flight routing through Warp's backend.
- Confirm the section still renders provider API key editors, custom endpoint entries, and the Warp credit fallback toggle in the same order.
- Click Learn more and verify it opens a documentation page that describes Custom inference endpoint or BYOK backend routing.
- Open the Add custom endpoint modal and verify the disclosure is visible before saving.
- Save a valid endpoint and verify the success toast and endpoint list behavior are unchanged.
- Open the Edit custom endpoint modal for an existing endpoint and verify the same disclosure is visible.
- Verify invalid URL, HTTP URL, local/private host, empty API key, and empty model behavior remains unchanged.
- With Custom inference disabled or unavailable, verify no enabled-looking custom endpoint controls are introduced.

## Open questions
- Final user-facing copy should be reviewed by Product/Legal/Privacy before implementation lands.
- Should the same in-app disclosure also be shown in the legacy BYOK-only API Keys state when Custom inference is not enabled?
- Should the docs link target the Custom inference endpoint page directly, or a broader BYOK and Custom inference comparison page if one becomes available?
