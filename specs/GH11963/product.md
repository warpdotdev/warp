# GH11963: Model configuration & capability parity for custom endpoints

## Summary
Give custom (bring-your-own) endpoint models real, user-configurable metadata —
context window size and reasoning/thinking capability — and make Warp's Agent Mode
derive request behavior from that metadata the same way it does for Warp-hosted
models. (Generation parameters like temperature are part of issue #11963 but are
deferred here pending a wire/proto change — see Goals.) Today custom models are
registered
with a stubbed, all-zero metadata record, so context management, reasoning, and
context-window features silently degrade. This is the source of the perceived
"penalty for open models." The context *payload itself* is already identical for
hosted and custom models (see `specs/GH11963/audit.md`); this spec closes the
remaining metadata and capability gap.

## Problem
Custom endpoints are configured GUI-only and expose essentially no model
settings. As a result (per issue #11963 and confirmed in the audit):

1. **Context window is unknown to Warp.** Custom models register with a zeroed
   context window, so Warp sends `base_model_context_window_limit = 0` and cannot
   make informed truncation / summarization / conversation-length decisions.
2. **Reasoning/thinking is off by default.** Custom models carry no reasoning
   level, so thinking-capable models (e.g. DeepSeek via OpenRouter) either lose
   reasoning or error out (issues #11810, #11587). In-flight PR #11812 responds by
   *disabling* reasoning for all custom endpoints — a regression for models that
   do support it.
3. **No generation parameters.** There is no way to set temperature, max output
   tokens, or provider-specific parameters (issue #11947).
4. **Capabilities are guessed, not declared.** Because metadata is stubbed,
   several Warp features fall back to ad-hoc per-model-type branches
   (`is_custom_endpoint` checks) instead of reading a capability from the model.

## Goals
1. Let users declare a context window size for each custom model, and have Warp
   use it for context management exactly as it does for hosted models.
2. Let users declare whether a custom model supports reasoning/thinking, and route
   reasoning behavior from that declaration rather than a blanket on/off.
3. Make custom-model metadata representable in `settings.toml`, not GUI-only, so
   it is version-controllable and shareable.
4. Drive Agent Mode request capabilities from the effective model's metadata, so
   custom and hosted models share one code path and there is no implicit penalty.

> **Generation parameters (temperature, max output tokens) are deferred.** The
> current request wire (`warp-proto-apis` rev `97d1b36`) has no field for them on
> `ModelConfig` or the custom-model message (it carries only `slug` +
> `config_key`). Delivering issue #11963's temperature/param request requires a
> proto change in the external `warp-proto-apis` repo first; see tech spec
> "Follow-ups". Context window and reasoning *are* already wire-supported and are
> the deliverable scope here.

## Non-goals
1. Do not change the network routing model. Requests continue to flow through
   Warp's backend; this spec does not propose direct client→endpoint connections
   (that is issue #12142).
2. Do not change the context *payload* assembly — it is already unified (audit §1).
3. Do not implement `/v1/models` auto-discovery here; this spec defines the
   metadata model that discovery (issue #11514 / PR #11731) can later populate.
4. Do not redesign the custom-endpoint modal beyond the fields needed for the new
   metadata.
5. Do not add server-side provider translation logic (out of this repo).

## Behavior
1. When adding or editing a custom endpoint model, the user can optionally set:
   - **Context window** (max tokens the model accepts). Optional; if unset, Warp
     behaves as today (treats the window as unknown / server default).
   - **Supports reasoning** (toggle). Default off. When on, Warp keeps the
     reasoning-message capability enabled for that model.

   (Temperature / max output tokens are deferred pending a wire change — see the
   Goals note above.)
2. These fields are persisted with the rest of the custom-endpoint configuration
   and are also expressible in `settings.toml` under the existing custom-endpoint
   structure, so they round-trip through settings sync and version control.
3. With a context window set, Warp uses it for the same context-management,
   long-context, and configurable-window behaviors available to hosted models —
   no `is_custom_endpoint` special-casing.
4. With **Supports reasoning** on, reasoning works for that custom model. With it
   off, reasoning is suppressed for that model only — not for all custom models,
   and not for hosted models in the same workspace.
5. Existing custom models with no configured metadata keep today's behavior
   exactly (unknown window, reasoning off): this change is additive and
   backward-compatible.
6. The custom-endpoint fields are discoverable in settings search via terms like
   `context window`, `reasoning`, `thinking`, `custom model`.
7. Invalid values (e.g. a non-numeric context window) fall
   back to "unset" using the existing settings validation/error behavior rather
   than blocking the agent.

## Success criteria
1. A user configures a custom model's context window and Warp's context management
   respects it (verifiable: the request carries the configured
   `base_model_context_window_limit` instead of `0`).
2. A user enables reasoning on a thinking-capable custom model and uses it without
   the errors reported in #11810 / #11587; a user with reasoning off sees it
   suppressed for that model only.
3. Custom-model metadata set in the GUI appears in `settings.toml` and survives a
   restart and a settings-sync round trip.
4. Hosted models are unaffected; existing custom models with no new metadata
   behave exactly as before.
5. The number of `is_custom_endpoint` / custom-vs-hosted branches in Agent Mode
   request building does not grow — capability decisions read from model metadata.

## Relationship to other work
- **Anchors** issue #11963; partially addresses #11810, #11587, #11947.
- **Supersedes the approach of** PR #11812 (blanket reasoning disable) and PR
  #12808 (per-type long-context gate) by replacing per-type branches with declared
  capability. Those PRs can become trivial once metadata drives the flags.
- **Complements** PR #11731 / issue #11514 (auto-discovery can populate this
  metadata) and PR #12783 (billing-policy gating).
- **Builds on** the data-flow disclosure in PR #12003 / issue #11681.
