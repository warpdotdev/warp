# Open repository-browser Markdown links as local files

GitHub: [warpdotdev/warp#13434](https://github.com/warpdotdev/warp/issues/13434)

Figma: none provided. This first pass adds an alternate link action without changing the rendered Markdown layout.

## Summary

Let users deliberately open the local checkout copy of a file referenced by a remote repository-browser link in rendered Markdown. A platform-specific alternate-click gesture resolves an exact repository-relative path against the active local checkout; normal link opening remains unchanged.

## Goals / Non-goals

Goals:

- Make remote GitHub and GitLab file links useful while reviewing Markdown inside a local checkout.
- Keep the action explicit and deterministic: resolve one exact local file or report that no local match exists.
- Reuse the user's existing local-file opening behavior.
- Avoid network access, repository mutation, background indexing, and branch or commit checkout.

Non-goals:

- No fuzzy filename search, link-text matching, multiple-match picker, or repository selector.
- No checkout, fetch, branch switch, detached worktree, or attempt to make the local file match the URL's revision.
- No extraction of line or column locations from URL fragments in this first pass.
- No automatic hyperlinking of bare paths or code spans.
- No change to Agent Mode rich-output links, terminal hyperlinks, shared-session links, or remote-only files in this first pass.
- No new setting or default change for ordinary Markdown links.

## Behavior

1. In a rendered local Markdown file, plan, or notebook that has an active local repository context, the user can invoke **Open local repository file** on an `http` or `https` Markdown link by holding the platform's primary modifier plus Alt/Option while clicking it:
   - macOS: Command+Option-click.
   - Windows and Linux: Control+Alt-click.
2. A normal click, Command-click on macOS, or Control-click on Windows/Linux keeps the existing link behavior. In particular, an ordinary remote link continues to open its web URL and an ordinary local-path link continues to use the existing local-file behavior.
3. The alternate action applies only when the primary+Alt/Option mouse-down starts on an HTTP(S) rendered Markdown link and the mouse-up still identifies that same pressed link without producing a text selection. Warp clears the pending link press after mouse-up, cancellation, or a drag-created selection. Pressing on non-link text and releasing over a link, or pressing one link and releasing over another, never opens either the URL or a local file.
4. When the alternate gesture is used on a local path, `file:` URL, same-document anchor, `warp:` URL, email link, or another non-HTTP(S) target, Warp preserves that target's existing behavior rather than reinterpreting it as a remote repository link.
5. Warp resolves against the repository that contains the rendered Markdown document when that repository is known. For a plan or notebook that is not backed by a local file, Warp resolves against the active local terminal's detected repository. Warp never searches every repository known to the app.
6. If no local repository context is available, Warp does not open the remote URL as a fallback for the alternate gesture. It shows a non-blocking `No matching local file found` toast and leaves the current view unchanged.
7. The repository root's final directory name is the repository identity used for matching. The decoded URL path must contain that name as a complete path segment in the position used by a supported repository-browser file URL.
8. This first pass recognizes file-view URLs on the public GitHub and GitLab hosts:
   - `github.com/.../<repository>/blob/<revision>/<path>` and `github.com/.../<repository>/raw/<revision>/<path>`.
   - `gitlab.com/.../<repository>/-/blob/<revision>/<path>` and `gitlab.com/.../<repository>/-/raw/<revision>/<path>`.
   Self-hosted forges and other HTTP(S) URL shapes are not interpreted as local paths.
9. The repository owner/group, browser marker, and revision portion are used only to identify the possible file-path boundary. Warp does not compare the remote owner, fetch the remote repository, verify the revision, or change the local checkout.
10. Repository-browser URLs do not unambiguously encode where a branch name containing `/` ends and the file path begins. Warp therefore considers each non-empty decoded suffix after the browser marker, while leaving at least one preceding segment as the remote revision. Query parameters and fragments are ignored. Warp validates the original encoded path before a generic URL parser can normalize dot segments or backslashes, then decodes each segment exactly once. Incomplete or malformed percent escapes, raw or encoded separators within a segment, NUL, and raw or encoded `.`/`..` path segments make the request invalid.
11. Every considered suffix is checked as an exact repository-relative path according to the local filesystem. Warp does not use the rendered link text, final filename alone, fuzzy ranking, or case folding beyond the filesystem's own behavior.
12. A considered suffix is eligible only when it resolves to an existing regular file whose canonical location remains inside the canonical local repository root. Absolute paths, empty paths, `.`/`..` traversal, NUL bytes, directories, and symlinks that resolve outside the repository are rejected. The path retained after validation is the canonical in-repository file path, not the original joined path or a symlink alias.
13. When exactly one distinct canonical file resolves, Warp passes that verified canonical path through the same configured file-opening behavior used by an ordinary local Markdown file link. Existing Markdown Viewer and external-editor preferences remain authoritative. Returning the canonical path prevents an original symlink alias from being retargeted between validation and opening.
14. The URL's branch, tag, or commit is not a selector. Warp opens the file currently present in the checkout, even if its contents differ from the remote revision named by the URL.
15. When no suffix resolves, more than one suffix resolves, or the URL/path is invalid, over the documented resource limits, outside the repository, a directory, or unsupported, Warp shows `No matching local file found`. It does not guess between matches, open the web URL, create a file, open a picker, or change repository state.
16. Resolution uses the repository and filesystem state at invocation time. If the document, active session, working directory, repository detection, or target file changes between renders, the next alternate click uses the new current state. This path-based first pass does not claim an atomic file-descriptor lease against a concurrent mutation of the already-canonical path after resolution; it does guarantee that the unverified joined or symlink path is never handed to the opener.
17. The alternate action performs no network request and does not send the URL, candidate path, repository path, or file contents to a service.
18. Telemetry may record that the alternate action succeeded, failed, or lacked repository context, but it must not record the full URL, repository path, candidate path, link text, or file contents.
19. Remote sessions, remote Markdown files, shared-session viewers, static file panes without local repository context, and browser/WASM contexts without an eligible local filesystem report no local match. Opening or reusing a Markdown pane for remote/static content clears any prior local link context before the content becomes interactive. Warp must not accidentally resolve a remote path against an active or previously associated same-named directory on the local machine.
20. Holding Alt/Option, including primary+Alt/Option, over non-link text preserves existing selection and multi-select behavior. On an HTTP(S) rendered link, primary+Alt/Option records a pending alternate link press instead of starting Alt multi-select; Alt/Option without the primary modifier remains the existing multi-select gesture.
21. The feature adds no background scan or persistent mapping. Closing the Markdown surface leaves no link-to-file association behind.
22. Failure is non-destructive: it does not move focus away from the Markdown surface, modify the document, dismiss an open pane, or replace the user's clipboard.
23. Resolution bounds the input before constructing suffix candidates or performing candidate filesystem work: the complete source URL may contain at most 16 KiB, its decoded path at most 4 KiB, at most 128 decoded path segments, and at most 127 candidate suffixes. Exceeding any limit fails with the same no-match behavior and performs no candidate filesystem reads.
