# SSH Profiles Panel - Product Spec
GitHub issue: https://github.com/warpdotdev/warp/issues/2127
Figma: none provided

## Summary
Add a first-class SSH Profiles panel to Warp so users can save frequently used SSH connections and open them with one click. Profiles store connection metadata in Warp settings, store SSH secrets only in local secure storage, support jump-host chaining through other saved profiles, and respect the user's existing SSH Warpify settings.

## Problem
Users who connect to the same SSH hosts many times per day currently need to type or maintain repeated `ssh` commands, shell aliases, launch configurations, or external connection managers. Launch configurations can run an SSH command, but they do not provide a compact connection list, editable per-host metadata, password storage, jump-host composition, or a Termius/iTerm-style workflow inside Warp.

## Goals
- Provide a dedicated SSH Profiles panel in the existing toolbar/panel system.
- Let users add, edit, delete, and connect saved SSH profiles without editing settings files by hand.
- Keep SSH secrets out of plaintext settings and out of cloud sync.
- Let jump hosts be selected from other saved profiles instead of requiring users to type raw `-J` strings.
- Preserve Warp's existing SSH/Warpify behavior: profile connections should use the same Warpify on/off settings as manual SSH commands.
- Make the first version small enough to review safely: host profiles, tags, jump hosts, one-click connect, and direct-target password auto-fill.

## Non-goals
- Importing or editing `~/.ssh/config`.
- Folder hierarchies, connect-to-all-in-folder, or bulk SSH actions.
- SFTP, port-forward management UI, terminal multiplexing UI, or remote file browsing.
- Syncing profiles or passwords to Warp cloud.
- Replacing Warpify, `SshTmuxWrapper`, `SshRemoteServer`, or other existing SSH session internals.
- Automatically entering multiple passwords across jump hosts. Jump-host password automation is deferred because a prompt from an intermediate bastion is not safely distinguishable from the final target prompt in all cases.

## Behavior

1. The toolbar contains an SSH Profiles item that behaves like the existing panel items. Opening or closing SSH Profiles does not open, close, or otherwise mutate the Tabs, Tools, Code Review, Agent Management, or Notifications panels.

2. When the SSH Profiles panel is open and no profiles exist, it shows an empty state that clearly indicates no profiles have been saved yet and offers the panel add affordance.

3. When profiles exist, the panel shows a compact list of saved profiles. Each row shows:
   - the profile label as the primary text
   - `user@host` as secondary text when a username exists, or `host` otherwise
   - tags when present, using a subdued treatment that does not dominate the row

4. Row hover affordances match Warp's existing tab-card interaction pattern: destructive/edit actions appear only on hover, are visually grouped at the row's upper-right corner, and do not trigger the row's connect action when clicked.

5. Clicking a profile row opens a new terminal tab and runs the profile's SSH command. If the new tab is not bootstrapped yet, Warp queues the command and executes it once the tab can accept input.

6. Profile connection commands are rendered from structured fields rather than from a raw command string. A profile can include:
   - label, required
   - host, required
   - username, optional
   - port, defaulting to 22
   - identity file, optional
   - jump hosts, zero or more
   - tags, zero or more

7. Profile command rendering quotes arguments safely. Spaces or shell metacharacters in usernames, hostnames, identity paths, tags, or other fields must not cause shell injection or accidental argument splitting. The final SSH target must not be interpreted as another SSH option even if it begins with `-`.

8. The add/edit dialog validates before saving:
   - label is required
   - host is required
   - port must be 1 through 65535
   - invalid forms keep Save disabled and Enter must not submit

9. Editing an existing profile preserves its stable identity. Renaming the label or changing the host must not accidentally reuse another profile's secure-storage secret entries or orphan the profile's own secret entries.

10. SSH secrets are never stored in the settings file and never synced to the cloud. A password entered in the profile dialog is stored only in a local OS secure-storage entry associated with the profile's stable id and explicit credential kind.

11. The password field in the add/edit dialog is masked by default. The user can toggle visibility while the dialog is open, clear the stored password explicitly, or leave the field unchanged when editing. Closing the dialog by Save, Cancel, Escape, or backdrop close clears sensitive password editor state from the dialog.

12. Stored SSH secrets have an explicit credential kind. The first version supports at least a host account password kind; if key passphrase auto-entry is implemented, it must be stored as a distinct key-passphrase kind tied to the matching identity file. Warp enters a stored secret only when the login prompt type matches the stored credential kind: host passwords go only to strict OpenSSH `user@host's password:` prompts, and key passphrases go only to matching `Enter passphrase for key ...:` prompts. Warp must not enter the same secret again after a failed password attempt or after login has completed.

13. Password auto-entry is guarded against non-SSH prompts. Prompts such as `[sudo] password for ...`, generic `Password:`, host trust prompts, and arbitrary command prompts must not receive stored profile passwords.

14. Password auto-entry is scoped to the tab/block created for the profile connection. If focus moves to another active block, the SSH command changes, the login times out, or the user starts a different command, the pending password state is discarded.

15. Profiles can use jump hosts by selecting from the user's other saved profiles. The add/edit dialog's jump-host dropdown excludes the profile being edited and excludes already selected profiles. Selected jump hosts render as removable chips.

16. Jump-host profile selection preserves both the selected source profile id and a structured metadata snapshot from the selected profile, including host, username, port, and identity file. In the first version, selecting a jump profile includes only that profile's direct host metadata, not that profile's own jump-host chain. If a user wants multiple hops, they select each hop explicitly in order. Connecting a profile with jump hosts chains through the selected direct-hop snapshots in order.

17. If a selected jump profile is edited later, existing dependents retain their saved snapshot rather than mutating silently. The edit dialog should surface enough information for users to remove and re-add the jump host if they want to refresh the snapshot. If a selected jump profile is deleted later, any profiles that referenced its source profile id remove that stale jump-host reference rather than retaining an unreachable hidden dependency.

18. When a profile uses one or more jump hosts, password auto-entry for the final target is disabled in the first version. SSH may still authenticate automatically through identity files, SSH agent, or user's existing SSH configuration; otherwise the user types prompts manually.

19. Profile connections respect the user's Warpify SSH settings:
   - when "Warpify SSH Sessions" is enabled, the profile connection should follow the same Warpify prompt/flow as an equivalent manual SSH command
   - when it is disabled, the profile connection should behave like plain SSH and must not show a successful Warpified session state just because the command came from a profile
   - changing "Use Tmux Warpification" must not invert the main Warpify on/off meaning

20. Profile storage is local to the user's Warp settings file and is marked private/non-cloud. A settings file containing profiles must not contain password material.

21. Removing a profile removes its local secure-storage secret entries when possible. Failure to remove a missing secure-storage entry must not block profile deletion.

22. The feature should degrade safely when secure storage is unavailable or a secret write fails. Users can still save non-secret profile metadata and connect, but Warp shows a visible warning that the secret was not saved, clears the password editor buffer, and disables password auto-entry for that profile until a compatible credential is successfully saved.

23. The profile panel and dialog support light/dark themes and compact window sizes without text overlapping controls or action buttons escaping the modal body.

24. Keyboard and focus behavior follows existing Warp modal conventions: Escape cancels, Enter submits only when valid, tab traversal reaches form fields and actions, and focus returns cleanly after the modal closes.

25. Logs and telemetry must not include password values. It is acceptable to log profile ids or high-level connection state for debugging, but never the secret contents.
