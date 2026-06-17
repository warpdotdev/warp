# APP-3773: Image attachments in the feedback skill — technical plan
Linear: https://linear.app/warpdotdev/issue/APP-3773/add-support-for-image-uploads-in-the-feedback-skill
See `PRODUCT.md` for user-facing behavior.
## Context
This feature touches three surfaces: the Warp client's slash-command → skill invocation pipeline, the bundled feedback skill's instructions, and the feedback skill's filing helper script.
The skill-invocation entry point is in the slash-command controller. `app/src/ai/blocklist/controller/slash_command.rs:74` builds the context that accompanies every slash-command request:
```rust path=null start=null
let context = input_context_for_request(
    /* is_user_query = */ false,
    controller.context_model.as_ref(ctx),
    ...
);
```
The `is_user_query: false` argument is the root cause of the "attachments don't reach the skill" bug. `input_context_for_request` in the same module delegates to `BlocklistAIContextModel::pending_context` in `app/src/ai/blocklist/context_model.rs`, which gates user-attached items — including `AIAgentContext::Image` entries built from `pending_attachments` — behind that flag. Non-slash-command user queries pass `true` here; all slash commands, including `InvokeSkill`, pass `false`. As a result, pending images, pending selected text, pending context blocks, and auto-attached agent-view blocks are stripped from the skill's context before it reaches the agent.
The relevant image carrier is `ImageContext` in `app/src/ai/agent/mod.rs:1893`:
```rust path=null start=null
pub struct ImageContext {
    pub data: String,        // base64-encoded image data
    pub mime_type: String,
    pub file_name: String,
    pub is_figma: bool,
}
```
It holds an in-memory base64 blob and a filename string, but no on-disk path. This is why the filing script cannot receive a path to the attached image — there isn't one. The scoped design in `PRODUCT.md` avoids this by never passing image bytes or paths to the script at all.
The feedback skill lives at `resources/channel-gated-skills/dogfood/feedback/` and ships two artifacts that matter here: `SKILL.md` (agent instructions) and `scripts/file_feedback_issue.py` (the filing helper). The script's top-level control flow is in `main()`:
- If `gh_path_if_authenticated()` returns a usable path, call `create_issue_with_gh` and print a `created` (or `failed`) result.
- Otherwise, print an `unavailable` result and exit.
The filing script currently has no concept of "attachments are present," no browser path at all, and no way for the agent to pick a filing method explicitly. It also exposes a `--dry-run` flag that no caller actually uses.
## Proposed changes
Three landable changes, ordered by how independently each can ship.
### 1. Platform: let skill invocations see user-attached images
In `app/src/ai/blocklist/controller/slash_command.rs`, change the `is_user_query` argument passed to `input_context_for_request` for `SlashCommandRequest::InvokeSkill` specifically. Other slash-command variants continue to pass `false`.
There are two reasonable shapes; pick the narrower one:
- **Preferred:** Branch on `SlashCommandRequest::InvokeSkill` when building `context` and pass `true` only for that variant. Smallest behavior change; no new parameters on `input_context_for_request`.
- **Alternative:** Add an `include_user_attachments: bool` parameter to `input_context_for_request` (or to `pending_context`) that is orthogonal to `is_user_query`, and pass `true` for `InvokeSkill`. Use this only if we discover we want images but not blocks or selected text on skill invocations. The product spec doesn't currently require that separation, so the preferred path is simpler.
Reads / writes on `pending_attachments` in `BlocklistAIContextModel` do not need to change. `pending_context` already emits `AIAgentContext::Image(image.clone())` for each `PendingAttachment::Image` entry; the fix just stops hiding that output behind the slash-command gate.
No server-side change is required. `AIAgentContext::Image` is already serialized over the existing multi-agent API and rendered as multimodal input to the model.
### 2. Script: required `--use {gh|browser}` flag in `file_feedback_issue.py`
Replace the current implicit fallback logic in `main()` with an explicit, caller-selected method:
- New CLI argument: `--use` (required, `choices=["gh", "browser"]`, `dest="use_method"`). The caller (the skill) must pass one of the two values; `argparse` enforces the constraint and rejects anything else.
- Remove the previously-existing `--dry-run` flag. The skill does not use it, and leaving it in creates an unused branch the caller has to reason about.
- Split `main()` into two helper functions: `file_with_gh(title, body)` for the `--use gh` path and `fallback_to_browser(title, body)` for the `--use browser` path. `main()` becomes a thin dispatcher: `if args.use_method == "browser": fallback_to_browser(...)` else `file_with_gh(...)`.
- `file_with_gh` preserves today's behavior: returns `status: "created"` on success, `status: "unavailable"` when `gh` is missing or unauthenticated, and `status: "failed"` with a `gh_error` on error. It does not automatically fall back to the browser; if the caller wants browser, it must pass `--use browser`.
- `fallback_to_browser` is simplified. It no longer takes a `has_attachments` parameter: the browser path is only used by the skill when image attachments are present, so its user-facing `message` text and failure errors always reference pasting/dropping screenshots. The URL-prefill vs. body-in-payload branching is preserved: when the full URL would exceed `MAX_PREFILL_URL_LENGTH`, the body is returned under a `body` field in the JSON result and only the title is prefilled in the URL.
- Do not add a `forced_browser` (or equivalent) field to the JSON result payload. Per PRODUCT.md invariant 16, no telemetry is captured from this feature; keeping the payload shape unchanged avoids creating a latent telemetry hook that would need to be wired up or removed later.
No changes are needed in the script's `gh`, browser, or URL helpers themselves. The existing `browser_is_available()` gate continues to cover headless sessions and will produce a failure payload the agent can surface verbatim; the only adjustment is that the failure message always mentions image attachments, since `--use browser` implies they are present.
### 3. Skill: instruct the agent on the image-attached branch
Edit `SKILL.md` in the feedback skill to add a short, explicit conditional that runs after duplicate detection and before filing, and update the `Output` section to describe the required `--use` flag:
- If the user's query includes one or more image attachments (visible in multimodal context), the agent must:
  - Draft the issue body as normal, including a short, plain-language description of each attached image's content in the relevant Behavior/Problem/Actual-behavior/Artifacts section.
  - In the `Artifacts` section, emit one placeholder per attached image, one per line, in the order the images were attached. The placeholder text should be recognizable to a human reader as a placeholder (e.g. `_Paste screenshot here_`). The skill does not need to use any particular sentinel; the goal is that the user can see where to drop.
  - Invoke `file_feedback_issue.py` with `--use browser` (instead of `--use gh`) alongside the existing `--title` and `--body-file` arguments.
  - In the final user-visible response, combine the standard browser-opened (or body-in-payload) messaging with an explicit instruction to paste or drag each attached image into the placeholder line(s) before submitting the issue. Reference the count of attached images so the user knows how many to paste.
- If the user's query has no image attachments, pass `--use gh` and file via the gh CLI path as today. No placeholders, no drag-and-drop instructions.
The rest of the skill's workflow (classification, clarifying questions, grounded references, duplicate detection, issue structure, output handling for `created` / `browser_opened` / `unavailable` / `failed`) is unchanged. The image-attached branch is additive.
## Risks and mitigations
- **Broader effect of the platform flag change.** Passing `is_user_query: true` for `InvokeSkill` also exposes pending blocks and selected text to skills, not just images. This is almost certainly desirable (skills today silently lose that context too), but it is a behavior change for every existing skill invocation. Mitigation: land the platform fix behind a feature flag if the blast radius is a concern, or gate the change to image attachments specifically via the alternative `include_user_attachments` parameter described in Proposed changes #1.
- **User submits with an empty placeholder.** If the user forgets to drop the image, the issue will contain a literal `_Paste screenshot here_` line. Mitigation: keep the placeholder text short and obviously a placeholder; rely on the agent's in-body prose description as the authoritative content. Acceptable failure mode per PRODUCT.md #12.
- **GitHub web UI changes its drag-drop behavior.** The `user-attachments` upload flow is internal to GitHub and not a public API. A change to that surface could break the "drop into the body" step. Mitigation: none required at this layer — if GitHub's web UI stops accepting drops, the whole web-UI workflow breaks for everyone and is not specific to this feature.
- **Forced-browser path is slower than `gh issue create`.** Users with `gh` authenticated lose the one-shot filing speed when they attach images. Mitigation: document in the skill's response so the user understands why the browser opened. No telemetry is captured for this feature (PRODUCT.md invariant 16), so usage-based revisiting relies on qualitative signals (user reports, direct feedback on the feedback skill itself) rather than measurement. If that turns out to be insufficient, adding measurement is a separate, explicitly-scoped change.
## Follow-ups
- If the forced-browser path becomes the dominant filing path due to screenshots being common in feedback, reassess the decision to keep `gh issue create` at all. Unifying on a single path (browser or CLI) reduces user-facing branching in the skill's final response and collapses the test matrix.
- If another skill later wants the same "caller picks filing method" signal (for example, a future `/bug` or `/support` skill), consider lifting `--use {gh|browser}` into a shared filing helper rather than duplicating the flag semantics per skill.
- Consider exposing the image-attached placeholder convention as a named sentinel (for example, `<!-- warp-feedback:image-N -->`) if we ever want to post-process the submitted issue to validate that users replaced the placeholders before submission.
