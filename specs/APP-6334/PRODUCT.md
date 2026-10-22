# Desktop execution-host selection
## Summary
A new cloud-agent run must honor the execution host selected in its desktop composer,
including a restored selection that the user has not clicked again. This fixes a state
mismatch without changing the host menu or the server's default-host policy.
Tracking: [APP-6334](https://linear.app/warpdotdev/issue/APP-6334/riot-desktop-preselected-self-hosted-host-can-differ-from-launch).
## Behavior
1. An explicit self-hosted choice shown in a new-run composer is the host requested when
   the user submits. Deselecting and reselecting it is never required to activate it.
2. Reopening the app or creating a composer restores the last saved explicit host choice,
   including Warp, and makes that choice effective before the first submission.
3. An explicit choice takes precedence over the configured default. Without an explicit
   choice, use the existing client host override, otherwise the current window's team
   default. This fix does not change that precedence.
4. With neither an explicit choice nor a client/team default, retain the existing Warp
   fallback label and unspecified-host behavior. That fallback is not an explicit choice
   to override defaults applied when the server creates the run.
5. Choosing Warp explicitly requests Warp-hosted execution even when a self-hosted
   default exists. This is distinct from the unspecified-host fallback in behavior 4.
6. A refresh of team information or a change to the current team's default keeps an
   explicit choice effective. Without an explicit choice, the composer adopts the new
   default, or returns to behavior 4 when the default is removed.
7. Moving a window to another team updates its inherited default, but does not silently
   replace its explicit choice. Existing team access checks and disconnected-host
   behavior remain unchanged; this fix does not grant access to another team's workers.
8. Changing the host or team in another window must not retroactively change this
   composer's active explicit choice or make its label disagree with its submission.
   New composers may continue to restore the shared last-saved preference.
9. An explicitly selected self-hosted worker going offline must not silently turn that
   selection into Warp-hosted execution. Existing validation, queueing, and error
   behavior applies; this change introduces no new fallback or retry policy.
10. The rules above apply both to a composer created immediately and to one attached
    later. Explicit selection and default refresh must finish updating the launch choice
    before a subsequent submission uses it.
11. Menu labels, connected/default/disconnected badges, keyboard selection, focus
    restoration, and the persisted preference format remain unchanged.
12. Follow-ups to an existing run retain that run's execution placement. This fix changes
    fresh-run host selection, not relocation of existing runs or their follow-ups.
## Scope
Client state synchronization only. No server routing changes, new settings, team-scoped
preference migration, host-access redesign, or visual redesign.
