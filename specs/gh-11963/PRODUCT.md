# gh-11963: Custom Endpoint Model Configuration

## Summary

Warp should let users configure shareable, non-secret metadata for custom
endpoint models, including context limits and model variants, while keeping API
keys in secure storage. A custom model entry should behave like a first-class
model choice that can be assigned to Agent profiles and whose capabilities are
used when building Agent Mode requests.

## Problem

Custom endpoints currently store endpoint and model rows inside the AI API key
blob. That is convenient for secrets, but it makes non-secret model configuration
hard to review, share, and version-control. It also leaves custom models with
stubbed model metadata, so Warp cannot make accurate context-window or
reasoning decisions for models such as DeepSeek, GLM, and other
OpenAI-compatible endpoints.

## Goals

1. Let users define one or more custom model variants per endpoint.
2. Let each variant declare non-secret model metadata, starting with context
   window and reasoning behavior.
3. Make the non-secret configuration representable in `settings.toml`.
4. Keep endpoint API keys and other secrets out of `settings.toml`.
5. Let Agent profiles select a specific custom model variant and have Agent Mode
   request settings use that variant's declared capabilities.

## Non-goals

1. Do not store API keys in `settings.toml`.
2. Do not change the client-to-server routing path for custom endpoint requests.
3. Do not silently add no-op controls for generation parameters before the
   request wire can send them.
4. Do not require existing custom endpoint users to recreate their endpoints.

## Figma

No Figma mock was provided in issue #11963 or PR #12834. The behavior below
intentionally avoids prescribing final layout, spacing, or component placement.

## Behavior

1. A user can create a custom endpoint with the existing endpoint name, base URL,
   API key, and model slug fields.

2. The API key remains secret. Warp never writes it to `settings.toml`, never
   shows it in plaintext after save, and continues to store it through the
   existing secure-storage path.

3. A user can create multiple model variants for one endpoint. Variants may use
   the same provider model slug but have different display names and metadata.
   Example: one `deepseek-r1` variant can have reasoning off, and another can
   enable a reasoning setting for an Agent profile that needs it.

4. Each custom model variant has a stable identity. Renaming a variant or
   changing its metadata does not break Agent profiles that already selected it.

5. Each variant can optionally declare a context window as a positive integer
   token count. When set, Warp treats that value as the model's usable maximum
   context for request construction and context UI. When unset, Warp treats the
   window as unknown and preserves today's behavior.

6. Each variant can optionally declare reasoning behavior. At minimum, a variant
   can declare reasoning disabled or enabled. If Warp supports named reasoning
   levels for custom models, each named level is represented as a separate
   selectable variant so profiles can choose the exact behavior they need.

7. Agent profiles can select any valid custom model variant for the surfaces
   that already support custom model selection. The selected variant controls the
   request's model id and capability metadata.

8. Non-secret endpoint and variant metadata can be represented in
   `settings.toml`. This includes endpoint identity, endpoint display name, base
   URL when product/security accepts it as non-secret, model slug, display name,
   stable variant id, context window, and reasoning metadata.

9. Importing or editing `settings.toml` must not create a usable custom endpoint
   unless the matching secret exists in secure storage or the user enters it in
   the UI. Missing secrets produce an explicit disabled or needs-key state.

10. Invalid settings values are not silently converted to a different working
    configuration. Invalid context-window values, duplicate variant ids, missing
    required slugs, or malformed metadata are surfaced through the existing
    settings validation/error path and the affected endpoint or variant is not
    used for requests until corrected.

11. Existing custom endpoints continue to appear after the upgrade. If they only
    exist in secure storage, Warp keeps them working with unknown context window
    and reasoning disabled until the user edits or migrates their metadata.

12. When a configured custom model variant is used for Agent Mode, the request
    carries the selected model id and the effective context/capability metadata
    derived from that variant. Hosted model behavior is unchanged.

13. Temperature, max output tokens, and arbitrary provider parameters remain
    visible as desired future capabilities for issue #11963, but they must not be
    presented as active request controls until the request wire can carry them.
    If such fields are accepted in a future settings schema before runtime
    support ships, Warp must label them unsupported instead of silently ignoring
    them.

14. Settings search should find the custom endpoint configuration using terms
    such as `custom endpoint`, `custom model`, `context window`, `reasoning`,
    `thinking`, `temperature`, and `max tokens`.

15. Removing a custom endpoint or variant makes any Agent profile selection that
    depends on it fall back through the same missing-model behavior used for
    other unavailable models. Warp should not keep sending a stale custom model
    id after the variant is removed.
