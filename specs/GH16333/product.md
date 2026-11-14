# File Manager host labels during SSH — Product Spec
GitHub issue: https://github.com/warpdotdev/warp/issues/16333
Figma: none provided

## Summary
When Warp's File Manager / Project Explorer shows both local and remote directory trees for an SSH session, each top-level tree root should identify the machine it belongs to. Users should be able to tell at a glance whether similarly named roots are local or remote, without expanding the tree or opening files.

## Problem
During SSH workflows, Warp can show a local tree and a remote tree side by side in the File Manager. If both machines have the same repository or directory name, the current root labels can be identical, for example two roots named `COHE`. That makes file navigation risky because users cannot reliably tell which machine a click, copy-path action, or context attachment will target.

## Goals
- Clearly distinguish local and remote File Manager root directories when an SSH session exposes both kinds of tree in the same pane.
- Use recognizable machine or connection names, not only generic icons, so duplicate directory names remain distinguishable.
- Preserve the existing File Manager layout, ordering, expansion state, selection behavior, and context-menu behavior.
- Keep labels concise enough to fit in the left panel while still exposing the full useful identity on hover or accessible text when truncated.

## Non-goals
- Changing which local or remote roots appear in the File Manager.
- Adding remote File Manager support for SSH modes that do not already provide remote file tree data.
- Changing file open, drag-and-drop, copy path, context attachment, rename, delete, or directory-loading behavior.
- Renaming directories on disk or changing terminal working directories.
- Adding a user setting for root-label formatting in the initial implementation.

## Behavior
1. When the File Manager displays at least one local root and at least one remote root at the same time, every visible top-level root header includes an origin label in addition to the directory name.

2. The local root label identifies the machine running the Warp client. The preferred text is the local hostname. If Warp cannot determine the local hostname, the label falls back to `Local`.

3. The remote root label identifies the SSH destination. The preferred text is the user-facing SSH connection label, for example `user@host` or `host`, matching the rest of Warp's remote-session UI when available. If no user-facing connection label is available, the label falls back to the remote session hostname. If that is unavailable, it falls back to `Remote`.

4. Root labels are attached only to top-level root headers. Child directories and files keep their current names and do not repeat the machine label.

5. The default label format is `directory-name (origin)`, for example `COHE (local-hostname)` and `COHE (remote-hostname)`. The implementation may use an equivalent visual treatment, such as a secondary muted origin suffix, as long as both the directory name and origin are visible in the root row and accessible to assistive technology.

6. If local and remote root directory names are identical, both rows remain distinguishable without relying on row order. The user can identify which row is local and which row is remote from the visible label alone.

7. If local and remote root directory names differ, the origin labels still appear while both local and remote roots are visible. This avoids a conditional UI that changes only in duplicate-name cases and makes the local/remote model consistent.

8. If the File Manager displays only local roots, labels remain unchanged from today's behavior; root headers show the directory name without an added local hostname suffix.

9. If the File Manager displays only remote roots, labels remain unchanged unless the same view also has local roots. Remote-only views should not gain redundant suffixes that consume left-panel space without resolving local-vs-remote ambiguity.

10. When a remote root is shown before remote metadata has finished loading, its placeholder or root row still uses the best available remote origin label. The label may update when a better connection label becomes available, but the row must remain stable enough that expansion, selection, and scroll position are not reset solely because the label text changed.

11. Multiple remote roots on the same remote host use the same remote origin label. Multiple remote hosts, if ever displayed together in one File Manager, use distinct labels when Warp has distinct connection labels or hostnames.

12. Label text should be truncated or clipped consistently with existing File Manager text if the left panel is narrow. The full root identity should be available through the same hover tooltip or accessibility mechanism used for truncated file-tree rows, or added as part of this feature if none exists.

13. Keyboard navigation, mouse selection, expansion/collapse, drag-and-drop, and context-menu actions continue to target the same underlying item as before. Adding origin labels must not change focus order, selected item identity, or path calculations.

14. The label updates automatically when the visible root set changes. Entering an SSH session that adds remote roots causes local and remote root labels to appear; leaving the SSH session or losing remote roots causes local-only labels to return to the existing directory-name-only presentation.

15. If the remote server disconnects while local roots remain, stale remote roots should follow existing removal or loading/error behavior. Any remaining local-only tree should return to unlabeled local root names once no remote roots are visible.

16. Labels are plain display text. Copy path, copy relative path, attach as context, file open, and drag operations continue to use the actual filesystem or remote path, never the decorated label.

## Success criteria
1. In an SSH session with local `/.../COHE` and remote `/.../COHE` roots visible, the File Manager shows two distinct root labels that identify local and remote machines.
2. In a local-only session, File Manager root labels remain unchanged from the current UI.
3. File Manager item selection, expansion state, and scroll position survive a label refresh caused by remote metadata or connection-label availability.
4. Context menu actions and file open behavior still route local roots to local paths and remote roots to remote host-scoped paths.
5. Long hostnames or `user@host` labels do not break left-panel layout and remain understandable through truncation and tooltip/accessibility behavior.

## Validation
- Add automated coverage for root-label formatting with local-only, remote-only, mixed local/remote, duplicate root names, missing local hostname, and missing remote label cases.
- Add automated coverage that decorated labels do not affect `FileTreeIdentifier`, selected item mapping, expansion state, context-menu routing, or local/remote file-open routing.
- Manually validate on Windows, because the issue was reported there, with an SSH session where local and remote directories share the same basename.
- Manually validate a narrow left panel and long hostname to confirm truncation is acceptable and the full identity remains discoverable.

## Open questions
- Should the origin suffix include the local username for parity with remote `user@host`, or is local hostname alone preferable to reduce noise? The initial spec prefers local hostname only.
- Should the suffix use literal `Local` / `Remote` words in addition to hostnames, for example `COHE (local: dev-laptop)`? The initial spec prefers concise host labels but allows an equivalent visual treatment if design review chooses a clearer format.
