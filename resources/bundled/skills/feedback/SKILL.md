---
name: feedback
description: "Turn rough feedback about the Warp app into a filed GitHub issue or duplicate-issue response for `warpdotdev/warp`. Use ONLY when the user explicitly wants to report a problem with the Warp terminal/IDE/app itself—not when they're working on their own code, managing their own GitHub repos, or doing general software development tasks. SKIP when: the user is creating/managing GitHub issues or PRs for their own projects, reviewing PRs, diagnosing CI failures, using `gh` CLI for repo management, or performing any GitHub workflow not specifically about reporting a problem with the Warp application itself."
---

# Feedback

Turn rough Warp app feedback into a crisp filed issue or duplicate-issue response for `warpdotdev/warp`.

Treat Warp client, Warp app, Warp terminal, and Warp UX feedback as `warpdotdev/warp` only when the user's latest request explicitly reports a Warp product problem. User repository, GitHub, app, or code context is not enough to infer that a public Warp issue should be filed.

## Overview
- Use the `gh` CLI to search for and fetch code from `warpdotdev/warp` when product or implementation context would improve the report.
- If those repos are not available, draft the issue from the user's report alone rather than blocking on more context.
- This skill is strictly for issue filing and duplicate detection. Never modify code, generate patches, propose implementation diffs, or open a pull request as part of this workflow.
- If you cannot file an issue, say so explicitly in the response instead of attempting another side effect.
- The helper script applies the `in-app-feedback` label to filed issues for tracking.
- GitHub issues filed by this skill are public. Treat every title, body, URL, screenshot description, source reference, log excerpt, and repository name as publicly visible.

## Code access boundaries

This skill runs in environments where Warp source code may be present in the current working directory or on disk. The following rules apply unconditionally regardless of what source code is visible locally:

- **Never write, edit, or delete any source file.** Do not use Edit, Write, or any tool that modifies files on disk, even if asked to do so as part of filing feedback or "while you're in the code."
- **Never create or modify any git artifact.** Do not stage files, create commits, create branches, produce patches, or modify any git state.
- **Local source code is read-only context at most.** You may read local Warp source files (e.g., with Read or grep) only to find concrete file paths, symbol names, or setting names that would make a source reference more precise. Never read local files to produce a code fix or diff.
- **Prefer `gh` CLI for code lookups.** Use `gh` to search and fetch code from `warpdotdev/warp` rather than reading the local checkout when both are available.
- **The presence of local source code is not an invitation to fix it.** Observing that you are inside a Warp source directory changes nothing about the permitted outputs of this skill: issue filed, duplicate found, or explicit refusal.

Load the bundled reference files only when relevant:
- platform and OS-version resolution, plus operating-system-specific behavior: `references/platforms.md`
- logs and crash artifacts: `references/logs.md`
- output calibration examples: `references/examples.md`

## Workflow

### 1. Confirm scope and classify the report

- This skill only handles feedback about the Warp product that could plausibly be addressed by a code or docs change to the Warp client, server, or SDKs. Before drafting anything, verify the request is in scope.
- **Decline and exit the skill (do not call the helper script) when the report is clearly out of scope.** Out-of-scope categories include, but are not limited to:
  - Account, billing, subscription, plan, credits, refund, or invoice questions.
  - Login, authentication, SSO, password, or session-expiry problems.
  - Requests to contact human support, sales, or legal.
  - General venting, praise, or commentary with no actionable product signal.
  - Questions about third-party tools or the user's own shell, machine, or network configuration that are not about Warp's behavior.
  - Anything the user explicitly says is not about Warp, or that they just want to talk through.
- When you decline, respond in one or two sentences that (a) say you won't file an issue, (b) name the reason in plain language, and (c) point the user at the right channel: account/billing/support concerns go to the in-app Help menu or `support@warp.dev`, community discussion goes to the Warp Slack community, and security reports go to `security@warp.dev`. Do not apologize performatively and do not offer to retry the same flow.
- Only if the request is in scope, classify it as `bug`, `regression`, `ux issue`, or `feature request` before drafting.

### 2. Protect private and user-project context

- Before asking for details, drafting, duplicate searching, or filing, do a public-filing safety check. Look at the user's latest request, attached context, current working directory, mentioned repositories, file paths, logs, command output, screenshots, and prior conversation summary if available.
- **Do not auto-file to public GitHub when the report includes private, proprietary, customer, security-sensitive, or user-project context.** This includes private repositories, non-Warp repository names such as `owner/repo`, app/product names from the user's own work, source snippets, file paths, logs, env vars, credentials, tokens, URLs for private services, or screenshots that may expose private data.
- If sensitive or user-project context is present but the underlying issue is about Warp itself, ask a focused confirmation before filing: "This would create a public issue in `warpdotdev/warp`. Should I file a redacted report that excludes private repo names, app details, code, logs, and screenshots?" If the user does not clearly opt in, do not call the helper script. Provide a redacted draft instead and say no public issue was filed.
- When public filing is confirmed, minimize and redact aggressively. Replace private repo names with "a private repository", private app/product names with "a private app", exact file paths with generic descriptions, and logs/code with short behavioral summaries. Do not include screenshots unless the user explicitly agrees they are safe for a public issue.
- When the final body has been checked and is safe to publish publicly, include the hidden marker `<!-- public-github-safe:v1 -->` immediately after `<!-- warp-feedback-skill:v1 -->`. Only include this marker after the safety check above; it is the signal to the helper script that any sensitive context has been redacted or explicitly approved.

### 3. Ask only for missing facts that materially improve the draft

- Before drafting, decide whether the report already contains the minimum actionable information: what the user was doing, what they expected, and what happened (for bugs, regressions, and UX issues) or what they want to be able to do and why (for feature requests). If any of those pieces is missing, run a single focused clarifying round.
- Use the `ask_user_question` tool for that round. Ask 3-4 high-value multiple-choice questions in a single call, focused on user experience and expectations: what the user was trying to do, what felt confusing or broken, what they expected to happen instead, where in the product they hit the issue, and how much it blocked them.
- Follow the tool guidance where possible: only ask when necessary, do not add labels like `Select One` or `Select All that Apply`, and if fixed options are too limiting, include an `Other` option. If the user skips a question, proceed with your best judgment on what they did answer.
- **Run at most one clarifying round.** If after that round the minimum actionable information is still missing, decline to file rather than drafting a weak issue. Tell the user in one or two sentences exactly which specifics would unblock a future report (for example: "A short description of what you were doing when it happened and what you expected instead would let us turn this into an actionable bug report."). Do not file a placeholder issue just to close the loop.
- For bugs and regressions, first read `references/platforms.md` and try to resolve Warp version and operating system from the bundled version metadata and available context. Ask for reproduction steps only when they are not already clear, and for regressions in particular only when the flow is not readily available from the report or supporting context.
- For crashes, startup failures, rendering bugs, sync issues, or hard-to-reproduce regressions, ask for logs or crash artifacts only when they are likely to help. Read `references/logs.md` only when needed.
- If operating system version, Warp version, or operating-system-specific behavior is relevant, read `references/platforms.md` and follow the bundled metadata guidance there yourself when possible. Ask the user only if you still cannot determine the necessary platform details.

### 4. Check whether the feature or capability is already supported

- Before concluding that something is missing from Warp (feature requests, "it doesn't do X" complaints, "I wish it could Y" asks, or any UX complaint that could be explained by an existing setting or workflow), you **must** consult the docs first.
- Call the `search_warp_documentation` tool with the user's own phrasing. If the first query is vague or returns nothing actionable, try one shorter variant that keeps the same user-visible problem.
- If the search returns a clear match, respond with a concise, direct answer that cites the docs page (title + URL) and explains how the existing functionality addresses the user's ask. Do not file an issue and do not invoke the helper script.
- If the search returns an ambiguous or partial match, briefly summarize what does exist and ask one clarifying question about whether that satisfies the user's intent before deciding whether to file.
- If the search turns up nothing relevant, proceed to step 5. Do not invent workarounds, and do not imply a feature is missing when the docs already answer the question.
- Docs-first checking applies primarily to feature-request-shaped reports. For reproducible bugs and regressions, skip ahead to step 5 unless docs would clarify whether the current behavior is intended.

### 5. Ground the report in product and code context when helpful

- Search the `warpdotdev/warp` repo via the `gh` CLI for matching product language, expected workflows, setting names, or UX intent when that context would make the draft more actionable.
- Search the `warpdotdev/warp` repo via the `gh` CLI for matching components, settings surfaces, feature flags, and likely code paths when implementation context would help triage.
- Add source references only when they point to real files, symbols, settings names, or spec text that plausibly relate to the feedback.
- Never invent a root cause just to make the report sound complete.

### 6. Draft the issue

- Keep the title concrete and user-visible.
- Rewrite rough notes into a polished issue body with the shared section structure below.
- Preserve the user's meaning while making the report easier for an engineer to act on.
- If the exact reproduction steps are still uncertain, write the best-supported scenario and call out what is still unknown.
- Make the title specific enough that it can be used as the primary duplicate-detection query.

### 7. Handle image attachments, if present

- If the user's query includes one or more image attachments visible to you as multimodal context, apply the rules in this step in addition to the normal drafting workflow.
- Incorporate what you can see in each image into the drafted issue body. At minimum, describe the relevant visual content in prose in the `Problem`, `Actual behavior`, or similar section so the report remains coherent even if the images do not end up attached to the filed issue.
- In the `Artifacts` section, emit one entry per attached image, in the order you encountered them. Each entry must include both a **caption** describing what the screenshot depicts and a drop-target placeholder, so the issue body still conveys what each screenshot was meant to show even if the user never uploads the image. Use this format per image, numbered starting at 1:

  ```md
  **Screenshot 1:** <one-sentence caption describing what this screenshot shows, written from what you saw in multimodal context>.
  _Paste screenshot 1 here_
  ```

  Captions should be concrete (for example, "Agent footer with the Send button misaligned below the text input") rather than generic ("a screenshot" or "the bug"). Do not invent details that aren't visible; if an image is ambiguous or unreadable, say so in the caption.
- When invoking the helper script (see the `Output` section below), pass `--use browser` instead of `--use gh`. This makes the script skip `gh issue create` and open the prefilled new-issue page in the browser so the user can upload images through GitHub's native drag-and-drop.
- In your final response, explicitly instruct the user to paste or drag each attached image into the body at the corresponding `_Paste screenshot N here_` line(s) and then submit the issue. Reference the count of attached images so the user knows how many to paste. Do not claim the issue has been filed until the user submits.
- If the user's query has no image attachments, do not add captions or placeholders, pass `--use gh` as usual, and do not add drag-and-drop instructions to your final response.

### 8. Check for likely duplicates before filing

- Before invoking `scripts/file_feedback_issue.py`, search issues in `warpdotdev/warp` for likely title matches using the drafted title as the primary query.
- Use a lightweight title-based check only. Prefer precision over recall, and do not run a broad semantic fishing expedition.
- Start with the exact drafted title. If the exact title returns no clear title match, try one shorter normalized variant that removes filler words while preserving the same user-visible problem.
- A suitable command is:

```bash
GH_PAGER=cat gh issue list \
  --repo warpdotdev/warp \
  --state all \
  --limit 10 \
  --search "<title> in:title" \
  --json number,title,url,state
```

- Treat a result as a duplicate candidate only when the existing issue title clearly refers to the same underlying problem or request.
- If you find a clear title match, do not file a new issue. Respond by pointing the user to the existing issue and explain briefly why it appears to match.
- If no clear title match is found, proceed to file the new issue.

## Issue Structure
Use these sections in order when they apply:

- Summary
- Problem
- Reproduction steps or desired workflow
- Artifacts
- Warp version
- Operating system
For bugs, regressions, and UX issues, also include:

- Expected behavior
- Actual behavior

If you found grounded repo evidence, append:

- Possible source references

Section rules:
- If a required field is unknown, say `Unknown`.
- If an optional section does not apply, omit it.
- If no artifacts are attached, say `None attached`.
- For feature requests, the `Problem` section can describe the current friction or missing capability, and the `Reproduction steps or desired workflow` section can describe the desired flow instead of literal repro steps.
- For feature requests, `Expected behavior` and `Actual behavior` are usually omitted unless they genuinely clarify the request.

## Source Reference Rules

- Prefer concrete file paths, symbols, settings names, and spec headings over broad guesses like "probably in the terminal code."
- Cite spec references when they clarify the intended workflow, wording, or product expectation.
- Cite implementation references when they localize the likely surface area or affected component.
- Keep each reference brief and explain why it is relevant.
- Omit references that are speculative, weakly related, or only tangentially connected.

## Writing Rules

- Turn vague feedback into specific behavior without changing the user's meaning.
- Convert fuzzy narratives into numbered reproduction steps when the sequence can be inferred responsibly.
- Call out severity, frequency, and regression context when available.
- Be explicit about uncertainty or missing details instead of smoothing them over.
- Do not mention this skill, private internal discussion, or speculative implementation theories in the issue body.
- Do not pad the issue with source references unless they genuinely improve debugging or triage.
- Never treat this workflow as permission to implement a fix. This skill may only file an issue, decline to file, or point to an existing issue.
- Never suggest code changes, patches, or implementation diffs in the issue body, in your response, or as a follow-up action — even informally or "as a starting point." If the user asks for a fix, decline and redirect them to the filed issue.
- **Refuse to file in four situations:** (1) the report is out of scope per step 1, (2) public filing is unsafe or not confirmed per step 2, (3) the minimum actionable information is still missing after the single clarifying round in step 3, or (4) step 4 found a clear docs match for what the user is asking about. In each case, explain the decision in one or two plain sentences and do not call the helper script. Never imply a feature is missing when docs already answer the question, and never file a placeholder issue just to acknowledge the user.

## Output

Use the bundled helper script `scripts/file_feedback_issue.py` to file the issue in `warpdotdev/warp` instead of calling `gh` directly. The script requires a `--use` flag that selects the filing method explicitly:

- `--use gh`: creates the issue with `gh issue create`. Requires `gh` to be installed and authenticated for `github.com`. Prints a `created` result with `issue_url` on success, or `unavailable` when `gh` is missing or unauthenticated. Does not silently fall back to the browser.
- `--use browser`: opens the prefilled new-issue page in the browser so the user can upload image attachments via GitHub's web UI. Prints a `browser_opened` result on success. If the browser cannot be opened, automatically falls back to `gh issue create` and prints a `created` result with `browser_unavailable: true`; if both are unavailable, prints `failed`. Use this whenever the user attached one or more images to the query.
- The script always targets `warpdotdev/warp` on `github.com`.

Write the final body to a temporary UTF-8 file and pass the final title directly as an argument. When the user has no image attachments:

```bash
python3 scripts/file_feedback_issue.py \
  --use gh \
  --title "<title>" \
  --body-file <body-file>
```

When the user has one or more image attachments:

```bash
# Opens the prefilled new-issue page in the browser so the user can drop
# their images into the issue body via GitHub's web UI.
python3 scripts/file_feedback_issue.py \
  --use browser \
  --title "<title>" \
  --body-file <body-file>
```

The title and body should be structured as follows:

Issue title: `<title>`

Issue body:

```md
<!-- warp-feedback-skill:v1 -->
<!-- public-github-safe:v1 -->
## Summary
...

## Problem
...
<!-- Include these sections for bugs, regressions, and UX issues when applicable. -->

## Expected behavior
...

## Actual behavior
...

## Reproduction steps or desired workflow
1. ...

## Artifacts
...

## Warp version
...

## Operating system
...

<!-- Omit this section if there are no grounded references. -->
## Possible source references
- path or symbol: why it may be relevant
```

After completing the duplicate-check and filing workflow:
- If the duplicate-check step finds an existing matching issue, respond with the existing issue link and a brief summary (2–4 sentences max) explaining that a likely duplicate already exists, including the matching title, whether that issue is open or closed, and why it appears to match. Do not create another issue.

- If the JSON result has `status: "created"` and `browser_unavailable: true`, explicitly tell the user that the browser could not be opened (include the `message` field from the result) and that the issue was filed programmatically with the available text contents. Make clear that image attachments were not uploaded to the issue. Then provide the issue link and a brief summary (3–5 sentences max) of what was filed.
- If the JSON result has `status: "created"` (without `browser_unavailable`), respond with only the created issue link and a brief summary (3–5 sentences max) of what was filed: the classification, the core problem, and any notable missing details.
- If the JSON result has `status: "unavailable"`, say that you could not file the issue because the GitHub CLI was not installed or authenticated, and include the returned message.
- If the JSON result has `status: "blocked_sensitive_context"`, say that no public issue was filed because the draft still appears to contain private or user-project context. Provide a redacted draft or ask for explicit public-filing confirmation, depending on what is missing.
- If the JSON result has `status: "browser_opened"` (seen when `--use browser` was used for image-bearing feedback), do not claim the issue has been filed. Pass the returned `message` to the user, and explicitly instruct them to paste or drag each attached image into the placeholder line(s) in the issue body and then submit the issue. When the result includes a `body` field (the drafted body was too long to prefill in the URL), surface that body so the user can paste it into the issue form before attaching images.
- If the JSON result has `status: "failed"`, say that you could not file the issue because the filing flow failed, and include the returned `error` or `gh_error` message. When `--use browser` was used and filing failed (meaning both the browser and the `gh` CLI fallback were unavailable), make it explicit that image attachments were not handed off and no issue was filed.
- If issue filing is not possible for any other reason, say explicitly that no issue was filed.

Read `references/examples.md` only if you need a compact example of the expected polish level.
