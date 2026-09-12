# APP-5583: Route cloud-run links to Platform when the viewer has access

Linear: [APP-5583](https://linear.app/warpdotdev/issue/APP-5583/cloudmode-view-in-oz-links-to-old-oz-ui-instead-of-platformwarpdev)

Slack: [ChargePoint feedback and design discussion](https://warpdev.slack.com/archives/C08KTPNQN65/p1787354137931149)

Approver: Varoon Kodithala

## Summary
Warp must open cloud-run links in Platform for viewers who can use Platform. Warp must keep opening the legacy Oz web app for viewers who cannot use Platform. This temporary split keeps every run openable while the Factory waitlist exists.

## Figma
Figma: none provided. This change keeps the existing layout. The Slack thread contains a screenshot of the reported details-panel button.

## Problem
The desktop client currently builds every cloud-run URL from the legacy Oz origin. An unconditional move to Platform would fix the reported experience for enrolled viewers, but it would send waitlisted viewers to an access-request screen instead of their run.

## Behavior
### Destination selection
1. Warp selects the cloud-run destination from the signed-in viewer's current Factory access. The run's source does not affect the destination.

2. When the viewer has Factory access:
   - A production build opens `https://platform.warp.dev/runs/<run_id>`.
   - A staging build opens `https://platform.staging.warp.dev/runs/<run_id>`.
   - “View all cloud runs” opens the same origin at `/runs`.
   - A recording link preserves its `?artifact=<artifact_uid>` query.

3. When the viewer does not have Factory access, Warp opens the equivalent legacy Oz URL:
   - Production uses `https://oz.warp.dev`.
   - Staging uses `https://oz.staging.warp.dev`.

4. Warp also uses Oz while the viewer's access is unknown, and when the access check times out, fails, or returns an invalid response.

5. Warp never delays a click while it waits for the access check. A link always opens immediately.

6. The fail-to-Oz rule has a visible consequence. An enrolled viewer who clicks during the short startup window can still land in the old UI. Warp accepts this temporary regression because the run remains openable. The opposite choice can strand a waitlisted viewer on a request-access screen. The access check starts eagerly, so the startup window normally lasts only for the first network round trip and ends before the viewer opens a details panel or menu.

7. Warp uses the first successful access result for the rest of the authenticated session. If the result arrives while a panel or menu is open, the next click uses it. An already-open browser page does not move between hosts.

8. Warp does not refresh or retry the access check during the session. If the initial check fails, Warp uses Oz until the next authenticated session. If the viewer's eligibility changes during a session, Warp picks up the change on the next launch or account login. This limitation is acceptable because eligibility changes during an active session are rare and this branch will be removed when Platform reaches general availability.

### Included user-facing links
9. The cloud-task details panel:
   - Renames the button from “View in Oz” to “View cloud run.”
   - Renames its tooltip to “View this cloud run in the web app.”
   - Opens the selected `/runs/<run_id>` destination.

10. The clickable status chip:
   - Uses the tooltip “View cloud run in the web app.”
   - Opens the same selected destination as the details-panel button.

11. The orchestration pill overflow menu:
   - Renames “View in Oz” to “View cloud run.”
   - Replaces the Oz brand icon with a neutral external-link icon.
   - Opens the same selected destination as the details-panel button.

12. “View all cloud runs” keeps its current copy and opens the selected `/runs` destination.

13. “Open recording” opens the selected run destination and preserves the artifact query.

14. User-visible run links produced for remote child agents use the same selected destination.

15. The skill link in the details panel changes from “Open in Oz” to “Open in web app.” Its destination remains the legacy skill page because Platform has no equivalent global skill route.

### Deliberate compatibility boundary
16. The following destinations remain on Oz:
   - Global agent pages used by the details panel and billing usage. Platform only has factory-scoped agent routes, which require a factory ID that these surfaces do not have.
   - Global skill pages. Platform has no equivalent global skill route.
   - Agent memory citations. Platform has no memory route.
   - Cloud environment links and the “Visit Oz” setup guide. The standalone Platform router does not currently expose the same global destination.

17. Human-readable CLI output and machine-readable SDK `run_url` output remain unchanged. These outputs can be consumed outside the signed-in desktop viewer's context. Changing their host is a separate compatibility decision.

18. The telemetry event value for the pill action remains `view_in_oz` even though the menu copy changes. This preserves historical analytics continuity. The mismatch is deliberate temporary debt.

### Temporary lifetime
19. This conditional behavior remains only while Factory access is restricted. [APP-5602](https://linear.app/warpdotdev/issue/APP-5602/remove-app-5583-factory-access-routing-branch-after-platform-access) removes the access check and Oz fallback after production access becomes universal.

20. The removal trigger is the production access policy, not a calendar date. APP-5602 starts only after the Factory access endpoint returns allowed for every authenticated production viewer.

## Non-goals
- Infer Factory access from a run's provenance.
- Add a factory ID to cloud-run payloads or construct factory-scoped run paths.
- Add server-side redirects between Oz and Platform.
- Migrate agent, skill, memory, environment, setup-guide, CLI, or SDK destinations to Platform.
- Rename internal `OpenInOz` actions or the existing telemetry value in this change.
