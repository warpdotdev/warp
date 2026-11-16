# APP-3585: Setting for agent commit and PR attribution
Linear: https://linear.app/warpdotdev/issue/APP-3585
## Summary
Oz agent currently always adds a `Co-Authored-By: Oz <oz-agent@warp.dev>` attribution line to commit messages and pull request descriptions it creates. This behavior is gated by a binary team-level setting managed by admins. This feature upgrades that gate to a two-level setting: admins choose Yes / No / Respect User Choice, and individual users can control the setting when their team has not forced it.
## Problem
Teams and individual users may not want Oz's name in their git history or PR descriptions. Currently, admins can only turn attribution on or off for the entire team. There is no per-user opt-out, and users who want to control the behavior independently cannot do so unless their admin acts first.
## Goals
- Add a user-level toggle in the AI settings page for "Enable agent attribution."
- Give team admins a three-way choice (Yes / No / Respect User Choice) rather than a binary toggle.
- When a team has forced the setting on or off, reflect that in the user's UI and prevent the user from overriding it.
- Gate the attribution instructions in the Oz agent prompt on the effective combined setting.
## Non-goals
- Changing the content or format of the attribution line itself.
- Controlling attribution granularly per-repo or per-project.
- Any changes to attribution on shared agent runs.
## Figma / design references
Figma: none provided. The toggle follows the existing pattern used for "Store AI conversations in the cloud" and "Codebase Context."
## User experience
### AI settings page — user toggle
- A new widget appears in the AI settings page under the **Oz** section, in its own row.
- Label: **"Enable agent attribution"**
- Description: **"Oz can add attribution to commit messages and pull requests it creates"**
- Rendered as a standard boolean toggle switch.
**Default behavior:**
- When the user has no team, or their team's setting is "Respect User Choice," the toggle defaults to **on** (attribution enabled).

**Team-forced-on:**
- When the admin panel setting is "Yes" (force-enable), the toggle is shown in the **checked / on** state and is **non-interactive** (grayed out).
- A tooltip on the disabled toggle reads: **"This option is enforced by your organization's settings and cannot be customized."** (the standard AI-page wording shared with other AI settings such as Computer use in Cloud Agents).
**Team-forced-off:**
- When the admin panel setting is "No" (force-disable), the toggle is shown in the **unchecked / off** state and is **non-interactive**.
- Same tooltip as above.

**Team respects user choice:**
- When the admin panel setting is "Respect User Choice," the toggle is **interactive** and reflects the user's stored preference (default: on).
- Toggling it updates the user's preference immediately and persists across sessions.

### Admin panel — team-level setting
- The existing binary (on/off) control for agent commit attribution in the admin panel is replaced with a three-way selector: **Yes / No / Respect User Choice**.
- Default: **Respect User Choice** (matching the current behavior for teams that have not set this explicitly).

### Agent prompt
- Attribution instructions are only included in the Oz agent prompt when the **effective** setting resolves to **on**:
  - Team = Yes → always on (attribution instructions included).
  - Team = No → always off (attribution instructions excluded).
  - Team = Respect User Choice → on if user's preference is on, off if user's preference is off.
## Success criteria
- The "Enable agent attribution" toggle appears in the AI settings page under the Oz section.
- When no team or team setting is "Respect User Choice": the toggle is interactive, defaults to checked, and the user can turn it off.
- When team setting is "Yes": the toggle is visible, checked, and non-interactive with the managed-by-org tooltip.
- When team setting is "No": the toggle is visible, unchecked, and non-interactive with the managed-by-org tooltip.
- A commit or PR created by Oz **contains** the attribution line when the effective setting is on.
- A commit or PR created by Oz **does not contain** the attribution line when the effective setting is off.
- The user's preference persists across app restarts and across devices (cloud-synced).
- The admin panel shows a three-way selector (Yes / No / Respect User Choice) for team attribution.
## Validation
- Manual: verify toggle appears in Settings > AI under the Oz section with the correct label and description.
- Manual: verify toggle is locked (with tooltip) when team setting is Yes or No.
- Manual: verify toggling the setting off and creating a commit/PR via Oz does not include the attribution line.
- Manual: verify toggling the setting on and creating a commit/PR via Oz includes the attribution line.
- Manual: verify user preference persists after restarting the app.
- Manual: verify admin panel shows the three-way selector, and that each team-level option is correctly reflected in the user's toggle state.
## Open questions
- None at this time.
