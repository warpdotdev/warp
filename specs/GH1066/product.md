# Update Onboarding Agent Autonomy Settings for Full and Partial — Product Spec
GitHub issue: https://github.com/warpdotdev/warp-external/issues/1066
Figma: none provided
## Summary
Update the agent autonomy presets applied when a user picks **Full** or **Partial** on the onboarding agent slide so the resulting default execution profile matches the subtitle each option advertises. Update the **Partial** subtitle to describe the new, more permissive default. **Full** now unconditionally allows reading files, applying code diffs, and executing commands without prompting; **Partial** always allows reading files, always asks before applying code diffs, and lets the agent decide on command execution (asking only for sensitive commands).
In addition, this change fixes three pre-existing bugs in the path that applies those presets onto the user's default `AIExecutionProfile` so the selections actually land where the user can see them, while also not clobbering an existing user's stored cloud profile when they log in at the end of onboarding.
## Problem
**1. Autonomy presets did not match the subtitles.** The onboarding agent slide (`crates/onboarding/src/slides/agent_slide.rs`) advertises Full autonomy as "Runs commands, writes code, and reads files without asking." However, picking Full currently seeds the default execution profile with `AgentDecides` for `apply_code_diffs`, `read_files`, and `execute_commands`. `AgentDecides` is defined as "The Agent chooses the safest path: acting on its own when confident, and asking for approval when uncertain" — which means Full autonomy can still prompt the user, contradicting the onboarding copy. Partial autonomy today sets `read_files = AgentDecides`, `apply_code_diffs = AlwaysAsk`, `execute_commands = AlwaysAsk`, and `mcp_permissions = AlwaysAsk`, with the subtitle "Can plan and read files. Asks before making any changes." That is stricter than the experience we want to ship for Partial: file reads should not be gated behind prompts, and low-risk commands should be able to run without explicit approval while the agent still defers to the user for risky commands and any changes.
**2. Onboarding autonomy selections were silently dropped for logged-out users.** When the user completes onboarding before logging in and opts to skip login, every onboarding-driven edit on the default profile (base model + all permission fields) is dropped on the floor — the default profile ends up at the bare `AIExecutionProfile::default()` values. The underlying cause is that the default profile is in an `Unsynced` state and the code path that creates the cloud-backed object bails early when no personal Warp Drive owner is available, discarding the locally-modified profile.
**3. Existing users logging in at the end of onboarding saw duplicate / default-valued profiles.** When a user completes onboarding pre-login and then logs into an existing account, their cloud-stored default profile arrives via the initial bulk cloud load. That load path does not fire per-object `ObjectCreated` events (it emits a single `InitialLoadCompleted` instead), so the execution-profiles model never learns about the existing cloud profile. The subsequent onboarding edits hit the `Unsynced` branch and create a **duplicate** cloud default profile that is mostly default values with a few onboarding fields written, leaving the user with a default profile that matches neither their stored cloud values nor the onboarding selections.
**4. Existing users had their cloud-stored profile silently overwritten by onboarding.** Even with the reconciliation above in place, `apply_agent_settings` still writes the onboarding-selected base_model and permissions on top of the existing cloud profile, discarding the user's prior customizations. Existing users expect their stored profile to be respected when they log in, not replaced by the defaults they clicked through in an onboarding flow.
## Goals
- Make Full autonomy seed the default execution profile so the agent never asks for approval for read-file, apply-code-diff, or execute-command actions, matching the existing "Runs commands, writes code, and reads files without asking" subtitle.
- Make Partial autonomy seed the default execution profile so reading files is always allowed, applying code diffs always asks, and executing commands delegates the decision to the agent (agent asks only for sensitive commands).
- Update the Partial subtitle on the onboarding slide to accurately describe the new defaults.
- Preserve existing workspace-override behavior: when a team workspace enforces a given permission, the onboarding selection continues to skip writing that permission (the workspace value wins).
- Preserve the existing behavior of the **None** autonomy option (all permissions `AlwaysAsk`).
- Make onboarding's autonomy selections stick for users who finish onboarding without logging in, so their choices are visible on the default execution profile rather than silently dropped.
- Respect an existing user's cloud-stored default execution profile when they log in at the end of onboarding: the onboarding-selected base_model and autonomy permissions must not overwrite values the user has previously stored in the cloud.
- For brand-new users (no cloud default profile yet) logging in at the end of onboarding, the onboarding selections continue to seed the default profile and cleanly promote to a single cloud object on first login (no duplicate default profiles created).
## Non-goals
- Changing the post-onboarding Settings UI, the execution-profiles editor, or the permissions surface outside of the onboarding-driven defaults.
- Changing Full autonomy's `write_to_pty` behavior, which is already `AlwaysAllow`.
- Changing MCP permission defaults for **None**. Full and Partial have their MCP defaults updated alongside the other permissions so the resulting execution profile is internally consistent with the subtitle each option advertises (Full: never asks; Partial: agent decides on low-risk actions). None continues to seed `mcp_permissions = AlwaysAsk`.
- Retroactively updating execution profiles for users who already completed onboarding. This change only affects the defaults written at the moment the onboarding Agent slide is confirmed.
- Visual layout or copy changes to the Full, None, or workspace-enforced subtitles on the onboarding agent slide. Only the Partial subtitle string changes.
- Merging onboarding selections with existing cloud-stored profile values (e.g. taking the onboarding base_model but preserving cloud permissions). Existing cloud profiles are preserved wholesale; onboarding selections are preserved wholesale for fresh profiles. No per-field cherry-picking.
- Changing the non–execution-profile AISettings that onboarding also writes (e.g. `default_session_mode`, `should_render_cli_agent_footer`, `show_agent_notifications`, UI customization flags). Those continue to follow the onboarding selection regardless of whether the user is new or existing.

## Behavior

1. When a user lands on the onboarding agent slide, three autonomy options are shown in the same order and layout as today: **Full**, **Partial** (default), **None**.

2. The **Full** subtitle remains exactly `Runs commands, writes code, and reads files without asking.` — no copy change on this row.

3. The **Partial** subtitle changes to `Can plan, read files, and execute low-risk commands. Asks before making any changes or executing sensitive commands.` This replaces the current string `Can plan and read files. Asks before making any changes.`

4. The **None** subtitle remains exactly `Takes no actions without your approval.` — no copy change on this row.

5. When the user completes onboarding with **Full** selected and the default execution profile is not overridden by a team workspace for a given permission, the default profile is written with:
   - `apply_code_diffs = AlwaysAllow`
   - `read_files = AlwaysAllow`
   - `execute_commands = AlwaysAllow`
   - `mcp_permissions = AlwaysAllow`
   - `write_to_pty = AlwaysAllow` (unchanged)

6. When the user completes onboarding with **Partial** selected and the default execution profile is not overridden by a team workspace for a given permission, the default profile is written with:
   - `apply_code_diffs = AlwaysAsk`
   - `read_files = AlwaysAllow`
   - `execute_commands = AgentDecides`
   - `mcp_permissions = AgentDecides`
   - `write_to_pty = AlwaysAsk` (unchanged)

7. When the user completes onboarding with **None** selected, the default profile is written with every `ActionPermission` field set to `AlwaysAsk` and `write_to_pty = AlwaysAsk`. No behavior change from today.

8. Workspace overrides continue to take precedence. For any permission field the user's team workspace enforces, onboarding does not write that field — regardless of which autonomy option was chosen. The user's selection still applies to every other, non-overridden permission field on the same profile.

9. The workspace-enforced autonomy view of the onboarding slide (rendered via `render_autonomy_workspace_enforced`) is unchanged: the user still sees the "Set by Team Workspace" panel and cannot pick Full/Partial/None.

10. The agent SDK and execution-profile enforcement paths read the same `ActionPermission` fields as today — after onboarding applies its defaults, the behavior of the agent at runtime (including prompts, auto-approvals, and denylist/allowlist enforcement for execute commands) follows directly from those permission values. In particular:
    - With Full, the agent does not pause for approval on read/apply/execute actions in the default profile.
    - With Partial, the agent never pauses for read-file actions, always pauses for apply-code-diff actions, and — for execute commands — pauses only when the command falls outside the existing auto-approve allowlist / is on the denylist (the existing `AgentDecides` runtime behavior).

11. A user who completes onboarding, then changes their autonomy-related permissions in Settings, keeps those Settings changes. Re-running onboarding (for users who can) reapplies the defaults in this spec to the default profile only if that profile is not already backed by a cloud object — see invariants 13–15 for the preservation rules that now apply in the logged-in case.
12. No change to keyboard navigation, focus order, "Disable Oz" checkbox interaction, or any other behavior of the onboarding agent slide beyond the subtitle text and the permission values written at completion.
13. **Brand-new user, onboarding then skip login.** When the user completes onboarding, the onboarding-selected model and autonomy values (per invariants 5–7) are applied to the local default execution profile and remain visible in Settings → AI → Execution Profiles after onboarding — even without logging in. Before this change these edits were silently dropped. The next time the user triggers an edit after eventually logging in, that local profile is promoted to a single cloud-backed default profile carrying those same values.
14. **Existing user, onboarding then log in.** When the user completes onboarding and then logs into an account that already has a cloud-stored default execution profile, the cloud profile is adopted unchanged. The onboarding-selected `base_model`, `apply_code_diffs`, `read_files`, `execute_commands`, `mcp_permissions`, and `write_to_pty` are **not** written onto the existing cloud profile; the user sees their previously stored values in Settings → AI → Execution Profiles. Non–execution-profile onboarding settings (session default mode, CLI agent toolbar visibility, agent notifications, UI customization flags) continue to follow the onboarding selection.
15. **Brand-new account, onboarding then sign up.** When the user completes onboarding and then signs up for a new account (no cloud default profile yet), the onboarding selections from invariants 5–7 are applied to the default profile and saved to cloud as a single default execution profile for that account. No duplicate default profiles are created and the user sees the onboarding-derived values in Settings.
16. Workspace overrides (invariant 8) continue to apply in all three above cases — even when a cloud profile is being preserved, an enforced workspace autonomy value supersedes the stored profile value at enforcement time, as it does today.
