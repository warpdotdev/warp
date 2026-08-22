# Open repository-browser Markdown links as local files

GitHub: [warpdotdev/warp#13434](https://github.com/warpdotdev/warp/issues/13434)

Figma: none provided. This first pass adds an action to the existing link tooltip without changing the rendered Markdown layout.

## Summary

Let users deliberately open the local checkout copy of a file referenced by a remote repository-browser link in rendered Markdown. When a link is a possible local-repository file, its existing tooltip exposes an **Open local file** action that resolves an exact repository-relative path against the active local checkout; the tooltip's main target and all other link-opening behavior remain unchanged.

## Goals / Non-goals

Goals:

- Make remote GitHub and GitLab file links useful while reviewing Markdown inside a local checkout.
- Make the local-file action discoverable in the existing link tooltip.
- Keep the action explicit and deterministic: resolve one exact local file or report that no local match exists.
- Reuse the user's existing local-file opening behavior.
- Avoid network access, repository mutation, background indexing, and branch or commit checkout.

Non-goals:

- No fuzzy filename search, link-text matching, multiple-match picker, or repository selector.
- No checkout, fetch, branch switch, detached worktree, or attempt to make the local file match the URL's revision.
- No extraction of line or column locations from URL fragments in this first pass.
- No automatic hyperlinking of bare paths or code spans.
- No change to Agent Mode rich-output links, terminal hyperlinks, shared-session links, or remote-only files in this first pass.
- No new mouse modifier gesture, key binding, or setting.
- No new setting or default change for ordinary Markdown links.

## Behavior

1. Activating an `http` or `https` link uses the existing link tooltip as follows:
   - Editable local plans/notebooks retain their current tooltip behavior.
   - In the read-only local Markdown Viewer, a plain click on a link eligible for **Open local file** shows that same tooltip instead of immediately opening the remote URL, so the alternative is discoverable.
   - Other read-only/selectable surfaces, ordinary web links, and modified direct-open clicks retain their current behavior.
2. That tooltip includes a text action labeled **Open local file** only when all of the following are true at tooltip resolution time:
   - the Markdown surface has an eligible local repository context;
   - the raw URL has a supported public GitHub or GitLab repository-browser shape; and
   - the pure URL/path parser can derive at least one syntactically possible repository-relative file candidate for that local repository name.
3. Determining whether to show **Open local file** performs no candidate filesystem reads, network requests, git operations, or remote-revision checks. A visible action means the URL is a possible local-file reference, not that a matching file has already been proven to exist.
4. The action is absent for ordinary web URLs, unsupported or malformed repository URLs, local paths, `file:` URLs, same-document anchors, `warp:` URLs, email links, and any surface without eligible local repository context. Those targets retain their existing tooltip and opening behavior.
5. The tooltip's main link target remains the remote URL, and its Copy, Edit, and any existing link-specific action remain unchanged. Selecting the main target opens the URL. Existing modified direct-open behavior remains unchanged; this feature does not claim Command+Option-click, Control+Alt-click, or another new shortcut.
6. **Open local file** is available through the same pointer, keyboard-focus, and accessibility interaction conventions as the tooltip's existing text actions. Selecting it revalidates the URL, repository context, and filesystem state at that moment.
7. Warp resolves against the repository that contains the rendered Markdown document when that repository is known. For a plan or notebook that is not backed by a local file, Warp resolves against the active local terminal's detected repository. Warp never searches every repository known to the app.
8. If the repository context becomes unavailable after the tooltip appeared, selecting **Open local file** shows a non-blocking `No matching local file found` toast and leaves the current view unchanged. It never opens the remote URL as a fallback.
9. The repository root's final directory name is the repository identity used for matching. The decoded URL path must contain that name as a complete path segment in the position used by a supported repository-browser file URL.
10. This first pass recognizes file-view URLs on the public GitHub and GitLab hosts:
   - `github.com/.../<repository>/blob/<revision>/<path>` and `github.com/.../<repository>/raw/<revision>/<path>`.
   - `gitlab.com/.../<repository>/-/blob/<revision>/<path>` and `gitlab.com/.../<repository>/-/raw/<revision>/<path>`.
   Self-hosted forges and other HTTP(S) URL shapes are not interpreted as local paths.
11. The repository owner/group, browser marker, and revision portion are used only to identify the possible file-path boundary. Warp does not compare the remote owner, fetch the remote repository, verify the revision, or change the local checkout.
12. Repository-browser URLs do not unambiguously encode where a branch name containing `/` ends and the file path begins. Warp therefore considers each non-empty decoded suffix after the browser marker, while leaving at least one preceding segment as the remote revision. Query parameters and fragments are ignored. Warp validates the original encoded path before a generic URL parser can normalize dot segments or backslashes, then decodes each segment exactly once. Incomplete or malformed percent escapes, raw or encoded separators within a segment, NUL, and raw or encoded `.`/`..` path segments make the request invalid.
13. Every considered suffix is checked as an exact repository-relative path according to the local filesystem when the user selects **Open local file**. Warp does not use the rendered link text, final filename alone, fuzzy ranking, or case folding beyond the filesystem's own behavior.
14. A considered suffix is eligible only when it resolves to an existing regular file whose canonical location remains inside the canonical local repository root. Absolute paths, empty paths, `.`/`..` traversal, NUL bytes, directories, and symlinks that resolve outside the repository are rejected. The path retained after validation is the canonical in-repository file path, not the original joined path or a symlink alias.
15. When exactly one distinct canonical file resolves, Warp passes that verified canonical path through the same configured file-opening behavior used by an ordinary local Markdown file link. Existing Markdown Viewer and external-editor preferences remain authoritative. Returning the canonical path prevents an original symlink alias from being retargeted between validation and opening.
16. The URL's branch, tag, or commit is not a selector. Warp opens the file currently present in the checkout, even if its contents differ from the remote revision named by the URL.
17. When no suffix resolves, more than one suffix resolves, or the URL/path is invalid, over the documented resource limits, outside the repository, a directory, or unsupported, Warp shows `No matching local file found`. It does not guess between matches, open the web URL, create a file, open a picker, or change repository state.
18. Resolution uses the repository and filesystem state when **Open local file** is selected. If the document, active session, working directory, repository detection, or target file changes while the tooltip is open, the action uses the new current state. This path-based first pass does not claim an atomic file-descriptor lease against a concurrent mutation of the already-canonical path after resolution; it does guarantee that the unverified joined or symlink path is never handed to the opener.
19. The tooltip eligibility check and local-file action perform no network request and do not send the URL, candidate path, repository path, or file contents to a service.
20. Telemetry may record that the local-file action was shown, succeeded, failed, or lacked repository context, but it must not record the full URL, repository path, candidate path, link text, or file contents.
21. Remote sessions, remote Markdown files, shared-session viewers, static file panes without local repository context, and browser/WASM contexts without an eligible local filesystem do not show **Open local file**. Opening or reusing a Markdown pane for remote/static content clears any prior local link context before the content becomes interactive. Warp must not accidentally resolve a remote path against an active or previously associated same-named directory on the local machine.
22. The feature adds no background scan or persistent mapping. Closing the tooltip or Markdown surface leaves no link-to-file association behind.
23. Failure is non-destructive: it does not move focus away from the Markdown surface, modify the document, dismiss an open pane, or replace the user's clipboard.
24. Eligibility and resolution bound the input before constructing suffix candidates or performing candidate filesystem work: the complete source URL may contain at most 16 KiB, its decoded path at most 4 KiB, at most 128 decoded path segments, and at most 127 candidate suffixes. Exceeding any limit hides the action during eligibility evaluation or produces the same no-match behavior if state changed after the tooltip appeared, and performs no candidate filesystem reads.
