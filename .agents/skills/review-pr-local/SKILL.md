---
name: review-pr-local
specializes: review-pr
specializes_source: warpdotdev/common-skills:.agents/skills/review-pr
description: Repo-specific review guidance for warp. Only the categories declared overridable by the core review-pr skill may be specialized here.
---

# Repo-specific review guidance for `warp`

## Prerequisite: install the parent skill

This skill specializes the core `review-pr` skill (named in the `specializes` frontmatter field) and is not functional on its own. Before applying its guidance, confirm the parent skill is installed and resolvable at `.agents/skills/review-pr/SKILL.md`. If it is missing, install it first by copying the skill directory from the source declared in the `specializes_source` frontmatter field (`warpdotdev/common-skills:.agents/skills/review-pr`). Then continue with the guidance below.

This file is a companion to the core `review-pr` skill. It does not
redefine the review output schema, severity labels, safety rules, or
evidence rules. It only specializes the override categories the core
skill marks as overridable.

## Repo-specific style and recurring review patterns

- Check every PR against the testing guidelines in `.agents/skills/rust-unit-tests/SKILL.md` and `.agents/skills/gui-integration-test/SKILL.md`: flag tests the PR adds that those skills would call out, and new code that should have a test and doesn't. Treat a clear violation of these guidelines as `⚠️ [IMPORTANT]`, not a nit.
- When a PR is clearly a V0 or initial implementation, frame robustness suggestions such as timeouts, retries, and lifecycle management as optional future work rather than blocking concerns, unless they risk correctness, security, data loss, or a persistent UI hang.
- For Rust changes, apply the repository conventions from `AGENTS.md`: avoid unnecessary type annotations, prefer imports over long path qualifiers, name context parameters `ctx` and place them last, remove unused parameters instead of prefixing them with `_`, and prefer inline format arguments in macros.
- Audit the comments a PR adds or changes against the "Comments" guidance under "Development Guidelines" in `AGENTS.md` — comments carry a maintenance cost, so a new comment should earn its place. Check each added/changed comment individually against every named sub-rule (Minimalist Comments, Strictly "Why" Only, No Line-by-Line Narrations, Clean Docstrings, Single-source of documentation, Don't enumerate function call sites, No "transformation comments") rather than forming one overall impression of the comment's quality. Common issues to flag: comments that restate what the code already says instead of explaining non-obvious *why*; "transformation" comments that describe the edit rather than the current state (e.g. "this used to ..."); doc comments that narrate a function's internal steps or enumerate its callers; explanations duplicated at a call site or reference that the declaration's doc comment already covers; and existing comments removed as collateral of an otherwise unrelated change. Read the full list in `AGENTS.md` rather than relying on these examples alone. A comment that is technically accurate, well-written, or explains a subtle/important issue is not exempt from these rules — do not let those qualities substitute for the rule-by-rule check. Treat a confirmed violation as `⚠️ [IMPORTANT]`, not a nit.
- When a PR adds or changes calls to log macros (`log::*` / `safe_*`), review the level choice against `.agents/skills/logging-and-error-reporting/SKILL.md`: using `log::error!` for a failure that should be a Sentry issue (only `report_error!` and panics create issues — `log::*` at Error/Warn/Info are just breadcrumbs), an inappropriate level for hot paths, and secrets/PII in Info-and-above logs (use the `safe_*` macros for sensitive detail). For `report_error!` / `report_if_error!` calls, run the mandatory audit below instead of relying on a narrative pass.
- Avoid wildcard `_` match arms when an enum can reasonably be matched exhaustively; exhaustive matches are preferred so future variants are surfaced during review.
- For new or changed feature flags, prefer high-level runtime checks with `FeatureFlag::YourFlag.is_enabled()` over `#[cfg(...)]` unless the code cannot compile without a compile-time gate.
- Flag nested or redundant `TerminalModel` locking when the call stack may already hold the model lock. Prefer passing locked references down the stack and keeping lock scopes short.
- In WarpUI code, flag inline `MouseStateHandle::default()` usage during render or event handling. Mouse state handles should be created during construction and then cloned/referenced where needed.
- For user-facing UI changes, mention missing validation only when it is tied to a concrete risk or when the PR changes behavior that should be verified visually.

## Pre-Verdict Audit: error-reporting form

This specializes the core skill's Pre-Verdict Audit (error-reporting category). Whenever the diff adds or changes a `report_error!` or `report_if_error!` call, this audit is mandatory, no matter how large the diff is — a holistic read-through is not sufficient, and skimming past most of a large migration is exactly how the mass `log::error!` → `report_error!` migration merged this form of bug undetected.

Before drafting the body or choosing a verdict: list every `report_error!` / `report_if_error!` call the diff adds or changes, one by one with its file:line. For each one, check it against `.agents/skills/logging-and-error-reporting/SKILL.md` (rules 1–5 and the Anti-patterns block define the exact forms; this list is a lookup index, not a restatement) for:
- a real, typed error demoted into `extra:` instead of reported as the payload
- a typed error stringified into the grouping message (`anyhow!("{e}")` / `"{e:?}"`) instead of preserved via `.context()` / `anyhow::Error::new`
- per-instance/variable data interpolated into the grouping message instead of carried via `.context()` / `extra:`
- the same failure reported more than once instead of once at the sink ("Report once, at the sink")

Also confirm hot/per-frame or per-message paths use `ReportErrorLogMode::OncePerRun` where the skill calls for it. Treat a confirmed violation as `⚠️ [IMPORTANT]`. The enumerated list is the evidence this audit ran — do not substitute a summary like "spot-checked the report_error! sites."

## Behavioral or UI-impacting changes require visual evidence

- If the PR changes anything user-visible (UI components, layout, styling, copy in surfaces users see, terminal/Warp app visuals, or other behavior a user can perceive), analyze both `pr_description.txt` and any PR comments available in the workflow context for attached screenshots, GIFs, or videos demonstrating the change end to end.
  - Treat markdown image/video embeds (`![...](...)`, `<img ...>`, `<video ...>`), GitHub user-attachment links (e.g. `https://github.com/user-attachments/...`, `https://user-images.githubusercontent.com/...`), Loom links, and similar hosted media as valid evidence.
  - The `Screenshots / Videos` section from `.github/pull_request_template.md` being present but empty does not count as evidence.
  - Unit tests, integration tests, `git diff --check`, code-path descriptions, and other textual explanations may supplement visual evidence but do not replace it for user-visible behavior.
- If the change is behavioral or UI-impacting and no screenshots or videos are attached in the description or comments, add an inline or summary-level comment requesting them. Use wording such as: "For this user-facing change, please include screenshots or a screen recording demonstrating it working end to end."
- When required visual evidence is missing for a behavioral or UI-impacting change that can be manually tested, set the final recommendation in the top-level `body` `## Verdict` section to `Request changes`, even if no other blocking issues were found. The top-level `verdict` field must be `"REJECT"` to match.
- Author environment limitations (e.g., headless runner, no desktop, environment can't capture) do not exempt UI-impacting changes from visual evidence. Suggest capturing the recording from a local desktop run or from a remote environment with desktop/computer-use support (for example, a coding agent such as Oz with [computer use](https://docs.warp.dev/agent-platform/warps-agent/capabilities-overview/computer-use) enabled). Reply with something like: _"This change is user-facing, so a screenshot or short recording is still required. If a local desktop isn't available, you can capture it from a coding agent that supports computer use (Oz is one option — see [Warp's computer use docs](https://docs.warp.dev/agent-platform/warps-agent/capabilities-overview/computer-use)) and attach it here."_ Set the verdict to `Request changes`.
- Exempt visual evidence only when the user-visible behavior truly cannot be meaningfully shown visually (for example, changes affecting only screen readers or non-visual side effects). If so, briefly state why screenshots or recordings would not be meaningful. Never exempt based on limitations of the author's environment.
- TUI caveat: for changes to the headless TUI (`crates/warp_tui` or the cell-grid element library at `crates/warpui_core/src/elements/tui`), acceptable "visual evidence" is a terminal transcript, a `render_to_lines` / `TuiBuffer::to_lines` snapshot diff, or a `./script/run-tui` capture — NOT a `computer_use` screenshot or real-display recording (those are for the GUI desktop app). See the `tui-verify-change` skill. The `MouseStateHandle` ownership rule still applies to TUI code: the TUI's hover/click elements (`TuiHoverable`, `tui_collapsible`) are built on the shared `MouseStateHandle` and must own the handle outside render (created once, reused) so hover/click state survives rebuilt element trees — so flag inline `MouseStateHandle::default()` there too. Only the GUI's *pixel-based hit-testing* specifics are GUI-only.
- If the PR is not user-visible at all (e.g. pure refactor, internal tools, build scripts, backend-only code, tests, or documentation), do not request screenshots or videos.

## User-facing strings

- Flag interpolated text that would read unnaturally at runtime or combine sentence fragments with the wrong casing.
- Link text should be descriptive rather than bare URLs or generic "click here" labels.
- Verify that product terminology is consistent across related UI, comments, workflow messages, and errors in the same PR.

## Graceful degradation and observability

- When optional dynamic data such as URLs, session links, workflow links, issue numbers, or metadata may be absent, prefer omitting the element or showing a short fallback over rendering empty or broken output.
- Do not suggest removing session links, workflow URLs, or diagnostic context from error paths. Those links are important for debugging failed automation and user reports.
- Prefer generic, user-safe error text in user-visible surfaces, but keep enough structured logging or diagnostic context for maintainers to investigate failures.
