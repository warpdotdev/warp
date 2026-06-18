# Product Spec: GitHub PR Prompt Chip — Default Inclusion with Validation

## Summary

Show the GitHub PR prompt chip by default in both the terminal prompt and the agent view footer. Reuse the chip's existing runtime behavior to validate whether the chip can work for the user. If the chip hits a deterministic `gh` readiness failure, suppress it from future default layouts and surface a warning in vertical tabs when relevant.

## Problem

The PR chip's current default inclusion is inconsistent:

- **Terminal prompt default:** chip is absent, so users with a working `gh` CLI do not discover it unless they manually customize their prompt.
- **Agent view footer default:** chip is present, so users without a working `gh` CLI can get a silent no-op.

The first proposed design added a separate proactive `gh` readiness model, but that duplicates capabilities already present in the chip runtime: required executable detection, local-session gating, command execution, failure suppression, and command-based invalidation.

## Goals

- Include the GitHub PR chip by default in both terminal prompt and agent view footer.
- Avoid mutating users' saved prompt or footer customizations.
- Reuse the PR chip runtime as the validation path rather than adding a redundant proactive readiness model.
- Suppress the chip from default layouts after deterministic readiness failures such as missing or unauthenticated `gh`.
- Keep the vertical tabs "Show: PR link" setting decoupled from validation, while showing a warning if PR links are enabled but cannot work.

## Non-Goals

- Adding a `gh auth` flow or prompting users to install `gh`.
- Detecting whether the current repo is hosted on GitHub beyond what the existing chip script already does.
- Changing CLI agent footer defaults.
- Changing the persisted `vertical_tabs_show_pr_link` setting default or type.
- Suppressing the chip permanently after transient failures such as timeouts, network errors, GitHub outages, or rate limits.
- Changing behavior for custom prompt/footer configurations except for the existing runtime chip behavior if the user manually included the chip.

## Figma

None provided.

## User Experience

### Definitions

- **Default prompt/footer:** the runtime-resolved chip set used when the user has not customized that surface (`PromptSelection::Default` or `AgentToolbarChipSelection::Default`).
- **Custom prompt/footer:** chip set explicitly saved by the user. This feature does not remove chips from custom configurations.
- **Validation state:** a hidden app state used to decide whether default layouts should keep showing the PR chip. It begins unvalidated.
- **Deterministic readiness failure:** a failure that shows the PR chip cannot work until the user changes local setup, such as `gh` missing from `$PATH` or `gh` installed but unauthenticated.
- **Transient failure:** a failure that should not suppress defaults, such as network errors, timeouts, GitHub outages, API/rate-limit errors, or unexpected `gh` failures.

### Behavior Rules

1. **Initial default terminal prompt:** The GitHub PR chip is included in the effective default prompt, positioned after `GitDiffStats`.

2. **Initial default agent view footer:** The GitHub PR chip is included in the effective default agent footer, in the existing agent-footer PR chip position after `GitDiffStats` and before `NLDToggle`.

3. **Successful validation:** If the PR chip successfully reaches the `gh`-backed path and resolves a PR URL, records "no PR found" as a benign empty result, or otherwise completes in a way that demonstrates `gh` is installed and authenticated, the chip remains in default layouts.

4. **Benign empty states:** Non-GitHub repo, no Git repo, detached HEAD, no `origin`, non-GitHub remote, and no open PR should not be treated as readiness failures. These states may produce no chip value, but they should not suppress the default.

5. **Deterministic readiness failure:** If the PR chip fails because `gh` is missing or unauthenticated, the chip is suppressed from future effective default terminal prompt and agent footer layouts.

6. **Transient failure:** If the PR chip fails for a transient reason, the chip remains in default layouts and may retry according to the existing chip invalidation/runtime behavior.

7. **Custom prompt or footer:** If a user explicitly added the GitHub PR chip to a custom prompt or footer, keep it there regardless of validation state. Existing runtime disabled/failure behavior still applies.

8. **Remote / SSH sessions:** The existing PR chip `local_only` runtime policy remains in effect. Remote-session failures should not rewrite custom settings. Whether they should mark the default as suppressed depends on implementation details, but the user-facing result should avoid showing a non-working PR chip in remote default layouts.

9. **No stored prompt/footer mutation:** The saved prompt/footer setting remains `Default` or `Custom` as-is. Suppression is applied to the effective default layout, not by rewriting a saved chip list.

10. **Re-enabling after setup changes:** If the user installs or authenticates `gh` after the default has been suppressed, they can manually add the PR chip from the prompt/footer editor. Automatic revalidation can be considered later but is not required for this change.

### Vertical Tabs "Show: PR link" Setting

The vertical tabs settings popup (expanded mode) has a "Show: PR link" toggle (`vertical_tabs_show_pr_link` in `TabSettings`). This setting's default value and persistence are not changed — it remains a plain `bool` defaulting to `true`. The toggle is decoupled from PR chip validation.

When "Show: PR link" is enabled but the PR chip has been suppressed due to a deterministic readiness failure, a warning icon is shown next to the toggle label in the settings popup.

11. **"Show: PR link" enabled and validation is not suppressed:** No warning. PR badges render in expanded vertical tab rows when a PR URL is available.

12. **"Show: PR link" enabled and validation is suppressed:** A warning icon appears inline next to the "PR link" label in the settings popup. Hovering the icon shows a tooltip explaining that the GitHub CLI must be installed and authenticated. No PR badges render unless the user manually re-enables/fixes the PR chip flow.

13. **"Show: PR link" disabled:** No warning icon, regardless of validation state. No PR badges render.

### Vertical Tabs Warning Details

When the warning is shown, it is rendered only in the expanded vertical-tabs settings popup's "Show" section on the "PR link" row.

- Row layout: `[check icon] [8px gap] [PR link label] [4px gap] [warning icon]`.
- Icon: `Icon::AlertTriangle`.
- Icon size: 12px by 12px.
- Icon color: the standard theme warning color (e.g., `theme.ui_warning_color()`), not the error color.
- Tooltip trigger: hovering the warning icon only. Hovering the rest of the row continues to behave like today: it highlights the clickable row and clicking toggles "Show: PR link".
- Tooltip text: "Requires the GitHub CLI to be installed and authenticated".
- Tooltip positioning: above the warning icon, using the standard Warp tooltip styling and overlay behavior.
- The warning icon is omitted entirely when "Show: PR link" is disabled or validation is not suppressed.

### What Does NOT Change

- The `GithubPrPromptChip` feature flag still gates the chip's existence entirely.
- The chip's shell scripts still handle repo/branch/no-PR cases.
- The chip's runtime behavior still handles local-session gating, dependencies, timeout, failure suppression, fingerprint caching, and invalidation.
- The `available_chips()` and `agent_footer_available_chips()` lists still expose `GithubPullRequest` when the feature flag is on so users can manually add it.
- The vertical tabs "Show: Diff stats" setting is unchanged.

## Success Criteria

1. A user with a working authenticated `gh` CLI on a fresh Warp install sees the PR chip in both terminal prompt and agent view footer without configuration.

2. A user without `gh` initially sees the PR chip in default layouts until the PR chip runtime deterministically detects the missing dependency; afterward, the chip is suppressed from default layouts.

3. A user with unauthenticated `gh` initially sees the PR chip in default layouts until the PR chip runtime deterministically detects the auth failure; afterward, the chip is suppressed from default layouts.

4. Users with custom prompt/footer configurations see no saved customization changes.

5. No saved prompt/footer chip lists are rewritten by suppression.

6. No-PR, non-GitHub repo, non-git directory, detached HEAD, and non-GitHub remote states do not suppress the PR chip by themselves.

7. Transient failures do not suppress the PR chip by themselves.

8. When suppression has occurred and "Show: PR link" is enabled, vertical tabs settings show the specified warning icon and tooltip.

9. When "Show: PR link" is disabled, vertical tabs settings do not show the warning icon.

## Validation

- **Unit tests:** Default terminal prompt includes `GithubPullRequest` before suppression.
- **Unit tests:** Default agent footer includes `GithubPullRequest` before suppression.
- **Unit tests:** Custom prompt/footer configurations are not modified by suppression.
- **Unit tests:** Missing `gh` and unauthenticated `gh` transition validation state to suppressed.
- **Unit tests:** Benign empty states and transient failures do not transition validation state to suppressed.
- **Manual test (happy path):** With authenticated `gh`, open Warp with default settings in a GitHub repo branch with an open PR. Confirm chip appears in terminal prompt and agent footer and renders "PR #N".
- **Manual test (missing `gh`):** Remove/rename `gh` from `$PATH`. Open Warp with default settings. Confirm the chip is suppressed after deterministic failure.
- **Manual test (unauthenticated):** Run `gh auth logout`. Open Warp with default settings. Confirm the chip is suppressed after deterministic auth failure.
- **Manual test (custom prompt):** Add the PR chip manually to a custom prompt. Confirm suppression does not remove it from the saved custom config.
- **Manual test (vertical tabs warning):** With "Show: PR link" enabled and validation suppressed, confirm the warning icon appears next to the "PR link" label and the tooltip text matches the spec.

## Open Questions

1. Which exact chip-runtime failure values should count as deterministic unauthenticated `gh` failures? The implementation should prefer narrow matching to avoid suppressing defaults after transient errors.

2. Should suppression ever automatically reset after the user runs a successful `gh auth login` or installs `gh`? This is not required for this change, but could be added later if the one-way suppression feels too sticky.
