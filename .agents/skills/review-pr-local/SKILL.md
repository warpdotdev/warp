---
name: review-pr-local
specializes: review-pr
description: Repo-specific review guidance for warp. Only the categories declared overridable by the core review-pr skill may be specialized here.
---

# Repo-specific review guidance for `warp`

This file is a companion to the core `review-pr` skill. It does not
redefine the review output schema, severity labels, safety rules, or
evidence rules. It only specializes the override categories the core
skill marks as overridable.

## Repo-specific style and recurring review patterns

- Do not suggest adding test cases that only vary constructor inputs or struct fields when an existing test already covers the meaningful behavior. Only suggest new tests when they exercise a distinct code path or edge case.
- When a PR is clearly a V0 or initial implementation, frame robustness suggestions such as timeouts, retries, and lifecycle management as optional future work rather than blocking concerns, unless they risk correctness, security, data loss, or a persistent UI hang.
- For Rust changes, apply the repository conventions from `WARP.md`: avoid unnecessary type annotations, prefer imports over long path qualifiers, name context parameters `ctx` and place them last, remove unused parameters instead of prefixing them with `_`, and prefer inline format arguments in macros.
- Avoid wildcard `_` match arms when an enum can reasonably be matched exhaustively; exhaustive matches are preferred so future variants are surfaced during review.
- For new or changed feature flags, prefer high-level runtime checks with `FeatureFlag::YourFlag.is_enabled()` over `#[cfg(...)]` unless the code cannot compile without a compile-time gate.
- Flag nested or redundant `TerminalModel` locking when the call stack may already hold the model lock. Prefer passing locked references down the stack and keeping lock scopes short.
- In WarpUI code, flag inline `MouseStateHandle::default()` usage during render or event handling. Mouse state handles should be created during construction and then cloned/referenced where needed.
- For user-facing UI changes, mention missing validation only when it is tied to a concrete risk or when the PR changes behavior that should be verified visually.

## Behavioral or UI-impacting changes require visual evidence

- If the PR changes anything user-visible (UI components, layout, styling, copy in surfaces users see, terminal/Warp app visuals, or other behavior a user can perceive), analyze both `pr_description.txt` and any PR comments available in the workflow context for attached screenshots, GIFs, or videos demonstrating the change end to end.
  - Treat markdown image/video embeds (`![...](...)`, `<img ...>`, `<video ...>`), GitHub user-attachment links (e.g. `https://github.com/user-attachments/...`, `https://user-images.githubusercontent.com/...`), Loom links, and similar hosted media as valid evidence.
  - The `Screenshots / Videos` section from `.github/pull_request_template.md` being present but empty does not count as evidence.
  - Unit tests, integration tests, `git diff --check`, code-path descriptions, and other textual explanations may supplement visual evidence but do not replace it for user-visible behavior.
- If the change is behavioral or UI-impacting and no screenshots or videos are attached in the description or comments, add an inline or summary-level comment requesting them. Use wording such as: "For this user-facing change, please include screenshots or a screen recording demonstrating it working end to end."
- When required visual evidence is missing for a behavioral or UI-impacting change that can be manually tested, set the final recommendation in the top-level `body` `## Verdict` section to `Request changes`, even if no other blocking issues were found. The top-level `verdict` field must be `"REJECT"` to match.
- Author environment limitations (e.g., headless runner, no desktop, environment can't capture) do not exempt UI-impacting changes from visual evidence. Suggest capturing the recording from a local desktop run or from a remote environment with desktop/computer-use support (for example, a coding agent such as Oz with [computer use](https://docs.warp.dev/agent-platform/warps-agent/capabilities-overview/computer-use) enabled). Reply with something like: _"This change is user-facing, so a screenshot or short recording is still required. If a local desktop isn't available, you can capture it from a coding agent that supports computer use (Oz is one option — see [Warp's computer use docs](https://docs.warp.dev/agent-platform/warps-agent/capabilities-overview/computer-use)) and attach it here."_ Set the verdict to `Request changes`.
- Exempt visual evidence only when the user-visible behavior truly cannot be meaningfully shown visually (for example, changes affecting only screen readers or non-visual side effects). If so, briefly state why screenshots or recordings would not be meaningful. Never exempt based on limitations of the author's environment.
- If the PR is not user-visible at all (e.g. pure refactor, internal tools, build scripts, backend-only code, tests, or documentation), do not request screenshots or videos.

## User-facing strings

- Flag interpolated text that would read unnaturally at runtime or combine sentence fragments with the wrong casing.
- Link text should be descriptive rather than bare URLs or generic "click here" labels.
- Verify that product terminology is consistent across related UI, comments, workflow messages, and errors in the same PR.

## Graceful degradation and observability

- When optional dynamic data such as URLs, session links, workflow links, issue numbers, or metadata may be absent, prefer omitting the element or showing a short fallback over rendering empty or broken output.
- Do not suggest removing session links, workflow URLs, or diagnostic context from error paths. Those links are important for debugging failed automation and user reports.
- Prefer generic, user-safe error text in user-visible surfaces, but keep enough structured logging or diagnostic context for maintainers to investigate failures.
