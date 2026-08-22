# Open repository-browser Markdown links as local files — Tech Spec

See [`PRODUCT.md`](PRODUCT.md) for user-visible behavior.

Code reference: [`2fe6a4f567928c6f11b74021e55092e5f3e5bd79`](https://github.com/warpdotdev/warp/tree/2fe6a4f567928c6f11b74021e55092e5f3e5bd79)

## Context

Rendered Markdown in local files and notebooks uses `RichTextEditorView`. Activating a URL without the existing direct-open modifier creates a `LinkToolTipConfig`, resolves the raw target through `NotebookLinks`, and renders the existing link tooltip. The tooltip keeps the resolved target as its main link and adds Copy, Edit, and any `LinkTarget::secondary_action` buttons ([`app/src/notebooks/editor/view.rs:1907-2005`](https://github.com/warpdotdev/warp/blob/2fe6a4f567928c6f11b74021e55092e5f3e5bd79/app/src/notebooks/editor/view.rs#L1907-L2005), [`app/src/notebooks/editor/view.rs:2280-2485`](https://github.com/warpdotdev/warp/blob/2fe6a4f567928c6f11b74021e55092e5f3e5bd79/app/src/notebooks/editor/view.rs#L2280-L2485)). `LinkTarget::Url` deliberately has no current secondary action, so the remote URL remains the primary target ([`app/src/notebooks/link.rs:25-66`](https://github.com/warpdotdev/warp/blob/2fe6a4f567928c6f11b74021e55092e5f3e5bd79/app/src/notebooks/link.rs#L25-L66)). Read-only/selectable editors currently bypass the tooltip and open every URL directly, so the Markdown Viewer needs a narrowly configured exception for eligible repository links without changing comment chips or other selectable consumers. The local-repository action should extend this established tooltip pattern rather than claim a new mouse gesture or modify the shared rich-text event boundary.

`NotebookLinks::resolve` intentionally parses valid URLs before local paths, so an HTTP repository-browser URL always becomes `LinkTarget::Url` and opens through `ctx.open_url` ([`app/src/notebooks/link.rs:124-160`](https://github.com/warpdotdev/warp/blob/2fe6a4f567928c6f11b74021e55092e5f3e5bd79/app/src/notebooks/link.rs#L124-L160), [`app/src/notebooks/link.rs:253-299`](https://github.com/warpdotdev/warp/blob/2fe6a4f567928c6f11b74021e55092e5f3e5bd79/app/src/notebooks/link.rs#L253-L299)). Local file opening already centralizes the configured editor/Markdown Viewer choice and avoids handing executable-looking files to an unsafe system-default handler ([`app/src/notebooks/link.rs:353-400`](https://github.com/warpdotdev/warp/blob/2fe6a4f567928c6f11b74021e55092e5f3e5bd79/app/src/notebooks/link.rs#L353-L400)); the new path must finish through that code rather than introducing another opener.

Each `NotebookLinks` instance owns a `SessionSource`. A local Markdown file changes that source to the file's parent directory and its target session, while an unbound plan/notebook uses the active window session ([`app/src/notebooks/file/mod.rs:367-384`](https://github.com/warpdotdev/warp/blob/2fe6a4f567928c6f11b74021e55092e5f3e5bd79/app/src/notebooks/file/mod.rs#L367-L384), [`app/src/notebooks/link.rs:470-496`](https://github.com/warpdotdev/warp/blob/2fe6a4f567928c6f11b74021e55092e5f3e5bd79/app/src/notebooks/link.rs#L470-L496)). However, `FileNotebookView::open_remote` does not replace the model's initial `SessionSource::Active` or a previous local `Target`, so checking only whether the current source resolves to a local session would permit remote content to reuse unrelated local state ([`app/src/notebooks/file/mod.rs:586-679`](https://github.com/warpdotdev/warp/blob/2fe6a4f567928c6f11b74021e55092e5f3e5bd79/app/src/notebooks/file/mod.rs#L586-L679)). Repository detection already maps a working directory to its detected root; `FileSearchModel::repo_root_location` demonstrates the existing `DetectedRepositories::get_root_for_path` lookup and rejects local/remote type confusion ([`app/src/search/files/model.rs:72-99`](https://github.com/warpdotdev/warp/blob/2fe6a4f567928c6f11b74021e55092e5f3e5bd79/app/src/search/files/model.rs#L72-L99)).

`FilePane` and `NotebookPane` subscribe their `NotebookLinks` models to the shared `subscribe_to_link_model` adapter, which forwards local-file events to the containing pane group. `AIDocumentPane` does not currently make that subscription, so a plan can resolve a target but cannot complete the existing `NotebookLinks::open` event path ([`app/src/pane_group/pane/notebook_pane.rs:84-120`](https://github.com/warpdotdev/warp/blob/2fe6a4f567928c6f11b74021e55092e5f3e5bd79/app/src/pane_group/pane/notebook_pane.rs#L84-L120), [`app/src/pane_group/pane/notebook_pane.rs:168-212`](https://github.com/warpdotdev/warp/blob/2fe6a4f567928c6f11b74021e55092e5f3e5bd79/app/src/pane_group/pane/notebook_pane.rs#L168-L212), [`app/src/pane_group/pane/ai_document_pane.rs:62-143`](https://github.com/warpdotdev/warp/blob/2fe6a4f567928c6f11b74021e55092e5f3e5bd79/app/src/pane_group/pane/ai_document_pane.rs#L62-L143)).

Agent rich output has a separate detected-link and click path that opens `DetectedLinkType::Url` directly. It is intentionally outside this first implementation, matching the maintainer request to keep the initially separate Markdown surfaces scoped.

## Proposed changes

### 1. Add a contextual action to the existing link tooltip

In `app/src/notebooks/link.rs`:

- Add a context-level eligibility method on `NotebookLinks` that obtains the current eligible local base directory, asks `DetectedRepositories` for the containing repository root, and calls the pure parser from section 2 with the root's final directory name.
- Eligibility succeeds when the parser returns at least one syntactically possible path candidate. It must not stat, canonicalize, or open any candidate; a file may disappear, be ambiguous, or never have existed after the action is shown.
- Return only availability to the view. Do not retain the repository root or candidate paths in tooltip state, because the full action must re-read current context and revalidate the filesystem when selected.

In `app/src/notebooks/editor/view.rs`:

- Extend `LinkToolTipConfig` with contextual **Open local file** availability while retaining the raw URL and existing resolved `LinkTarget` state.
- Add a `RepositoryLinkTooltipMode` field to `RichTextEditorView`, initialized to `Disabled` by `RichTextEditorView::new`, plus a narrow setter used by owning surfaces after construction. `FileNotebookView` selects `SelectableFileViewer`, which both enables the contextual action and permits a plain eligible link click to open the tooltip while the editor is selectable. `NotebookView` and `AIDocumentView` select `ExistingTooltipsOnly`, which adds the action only when their normal interaction already opens a tooltip. Apply the owning surface's mode every time it installs or replaces an editor; in particular, `AIDocumentView::refresh`/`set_editor_model` must configure the replacement editor rather than relying on the constructor's initial handle. Every other consumer, including selectable comment chips, remains `Disabled`; no `RichTextEditorConfig` struct literal changes or broad selectable behavior changes are required. Existing `cmd`/primary-modifier direct-open behavior remains unchanged.
- Add a dedicated `EditorViewAction` for selecting **Open local file** and a persistent `MouseStateHandle` created with the view's other tooltip handles.
- Render the new `ButtonVariant::Text` action beside the existing tooltip actions only when the context-level eligibility check succeeds. Give it the visible label and accessibility content `Open local file`.
- Keep the main resolved URL, Copy, Edit, and `LinkTarget::secondary_action` behavior unchanged. Do not reinterpret the remote URL as `LinkTarget::LocalFile` and do not add this contextual operation to `LinkTarget::secondary_action`, which describes actions intrinsic to an already-resolved target.
- Subscribe `RichTextEditorView` directly to its `NotebookLinks` model. On `LinkEvent::RefreshLinks`, recompute availability for any open tooltip from its raw URL and notify the view; leave all other link events to the existing pane subscriptions. Treat the rendered flag only as discoverability state; the click handler never trusts it as authorization to open a path.

Do not change `RichTextAction`, mouse-down/up dispatch, multiselect behavior, key bindings, or settings. Existing direct-open modifiers remain available for their existing targets, and Command+Option/Control+Alt remains unclaimed by this feature.

### 2. Validate the original encoded URL and add a pure path extractor

Add a small pure helper next to `NotebookLinks` in `app/src/notebooks/link.rs` (with tests in `link_tests.rs`):

```rust
fn repository_relative_path_candidates_from_url(
    raw_url: &str,
    repository_name: &OsStr,
) -> Result<Vec<PathBuf>, RepositoryLinkError>
```

The helper:

- rejects a source string larger than `MAX_REPOSITORY_URL_BYTES` (16 KiB) before parsing or allocating candidate paths;
- rejects any raw backslash in the source, then isolates the original encoded path before query/fragment data and validates it before `Url::parse` can apply WHATWG dot-segment or separator normalization;
- validates complete `%HH` escapes, decodes each raw path segment exactly once, and rejects a decoded `.`/`..` segment, decoded `/` or `\` within a segment, NUL, and invalid UTF-8;
- accepts only `http` and `https`;
- walks decoded path segments and matches the repository name as one complete segment;
- requires the public `github.com` or `gitlab.com` host and recognizes GitHub `blob`/`raw` and GitLab `-/blob`/`-/raw` forms;
- requires at least one segment after the provider marker to remain the uninterpreted remote revision;
- returns every non-empty suffix that could be the repository-relative path, so a revision containing `/` does not make Warp silently choose the wrong boundary;
- rejects missing/empty remainder, root/prefix components, and repository names that cannot be compared to a decoded provider segment;
- rejects a decoded provider path larger than `MAX_DECODED_REPOSITORY_PATH_BYTES` (4 KiB), more than `MAX_REPOSITORY_PATH_SEGMENTS` (128), or more than `MAX_REPOSITORY_PATH_CANDIDATES` (127) before any filesystem lookup;
- ignores query and fragment data;
- does not read the filesystem, inspect git state, or perform fuzzy search.

`Url` may be used for the scheme, host, and provider-shape checks only after validation of the original encoded path. Do not change this API back to `&Url`: that would lose the information needed to reject normalization-altering encoded dot segments. Keep provider-shape parsing separate from filesystem validation so the URL matrix is deterministic and unit-testable without an app or repository fixture. The raw input is the Markdown link target, not its rendered label.

### 3. Resolve the candidate against the link's local repository context

Add `NotebookLinks::resolve_repository_url_as_local_file`:

1. Extend `SessionSource` with an explicit `Unavailable` state. `FileNotebookView::open_remote` and `open_static` set it before making the new content interactive, clearing either the initial `Active` source or a previous local `Target`. Local file context sets `Target` as today; normal local plans/notebooks retain `Active`. `Unavailable`, remote sessions, and non-local filesystem builds fail before repository or candidate filesystem lookup.
2. Add a separate repository-link-action availability flag to `NotebookLinks`, defaulting to enabled and consulted only by the new eligibility/resolution methods. Disabling it must not alter existing URL, local-path, anchor, or shared-session link resolution. Its setter emits `LinkEvent::RefreshLinks` when the value changes.
3. `AIDocumentView` disables that feature-specific flag before the first render when its conversation is already being viewed through a shared session. Make `BlocklistAIHistoryModel::set_viewing_shared_session_for_conversation` publish a focused event carrying the conversation id and new viewing state, and have `AIDocumentView` update the flag when the matching conversation enters or leaves shared-session viewing. This provides an observable transition seam without changing existing shared-session link behavior.
4. Require a local session and base directory from the eligible source, then ask `DetectedRepositories` for the local root containing that exact base directory. This makes a file-backed Markdown viewer use the repository containing the document, while an unbound plan/notebook uses the active local terminal repository.
5. Take the root's final directory name and pass it with the parsed URL to the pure extractor.
6. Join each relative candidate to the root and asynchronously obtain canonical paths for the root and candidates.
7. Retain only candidates whose metadata describes a regular file and whose canonical path starts with the canonical root. This rejects traversal and symlink escapes while still allowing an in-repository symlink whose canonical target remains in the repository.
8. Require exactly one distinct canonical match. Zero or multiple matches return a non-match error rather than picking a suffix.
9. Return the existing `LinkTarget::LocalFile` with the verified canonical candidate as `path`, `line_and_column: None`, the current session, and `is_markdown` derived from that canonical path. Never return the original joined path or symlink alias after validating a different canonical path.

Use a dedicated error enum that distinguishes unsupported URL, missing local repository context, invalid/unsafe path, and missing/non-file target for tests and coarse telemetry. Do not include sensitive paths or the source URL in safe logs.

This removes the actionable symlink-retarget validation/open gap while retaining the existing path-based opener. It does not introduce a file-descriptor lease or claim atomicity against a concurrent mutation of the already-canonical path after resolution; that broader filesystem capability is outside this first pass.

### 4. Route the tooltip action through the existing opener and one failure toast

When `RichTextEditorView` handles the dedicated **Open local file** action:

- Pass the tooltip's original raw link target to `resolve_repository_url_as_local_file`. That method must repeat the local-context and pure-parser checks before touching the candidate filesystem, so a stale tooltip cannot reuse an old repository association.
- On success, call `NotebookLinks::open` with the returned target so editor preferences and executable-file safety remain centralized.
- On every expected resolution failure, add one ephemeral `No matching local file found` toast to the current window.
- Consume the tooltip action in both success and failure cases. Never fall back to `ctx.open_url`; the tooltip's main link remains the only route that opens the remote URL.
- Do not change selection, focus, clipboard, or document contents. If the link or repository context changes while the tooltip is open, use current state or fail closed rather than opening a stale target.

Unexpected I/O failures may be logged at a non-sensitive level and use the same toast. No URL or local path is included in telemetry or logs.

To make the same centralized opener complete for plans, add a `NotebookLinks` getter to `AIDocumentView`, call the existing `subscribe_to_link_model` helper from `AIDocumentPane::attach`, and unsubscribe in `detach`, matching `FilePane` and `NotebookPane`. Do not add a second plan-specific file opener.

### 5. Keep surface boundaries explicit

This change applies to `RichTextEditorView` consumers with an eligible local `NotebookLinks` context: local Markdown Viewer and local plans/notebooks. The only plan-specific plumbing is the missing `NotebookLinks` pane subscription described above. Do not modify `app/src/util/link_detection.rs` or Agent block-list click handlers in this PR. The latter lack the same document/repository context and would otherwise turn this into a cross-renderer feature.

No feature flag is necessary: the main link target and all unrelated link behavior remain unchanged, failure is non-destructive, and the new branch runs only when the user selects a context-gated action in an existing tooltip.

## Testing and validation

### Pure URL/path tests

Extend `app/src/notebooks/link_tests.rs` with table-driven coverage mapped to PRODUCT invariants 7–12:

- Public GitHub `blob` and `raw` URLs and public GitLab `-/blob` and `-/raw` URLs yield the expected candidate suffixes.
- Owner/group prefixes, percent-encoded safe path segments, query strings, and fragments do not change the candidate.
- Repository-name substring matches, wrong or non-UTF-8 repository names, self-hosted/unsupported providers or shapes, missing revision/path, empty path, raw or encoded traversal/separators, absolute/root components, malformed encoding, NUL, and non-HTTP(S) schemes fail.
- A raw URL containing `src/%2e%2e/file`, `src/../file`, or a backslash fails before `Url` normalization; the normalized path is never accepted as if it were the original input.
- Inputs at each byte, segment, and candidate limit succeed or continue to normal resolution; one unit over each limit fails before filesystem resolution.
- Single-segment revisions and slash-containing revision prefixes can resolve an exact file without being used to select or mutate git state.

### Filesystem resolution tests

Using a temporary repository fixture and a local test session:

- an existing nested regular file resolves to `LinkTarget::LocalFile`;
- a slash-containing revision resolves when exactly one candidate suffix exists;
- two distinct existing suffixes fail as ambiguous rather than choosing one;
- a missing file, directory, and outside-root traversal fail;
- an in-root symlink to an in-root file succeeds where supported;
- a symlink escaping the root fails;
- the returned target contains the verified canonical file rather than the original symlink alias;
- missing repository detection, an explicitly unavailable source, remote session context, and remote/static Markdown content fail without consulting the local candidate filesystem;
- switching one file pane from local content to remote content cannot reuse the prior local target, and a fresh remote pane cannot use an active local terminal repository;
- success preserves the current session and sets no line/column selector.

### Tooltip eligibility tests

Cover PRODUCT invariants 1–8 and 21:

- a supported GitHub/GitLab file URL with eligible local repository context exposes **Open local file** when the pure parser returns at least one candidate, even when the candidate does not exist yet;
- ordinary web URLs, malformed/unsupported provider shapes, non-HTTP(S) targets, missing repository detection, unavailable/remote context, and browser/WASM builds do not expose the action;
- eligibility performs no candidate metadata or canonicalization calls;
- changing or clearing the link context refreshes the open tooltip's availability, and selecting a previously rendered action still revalidates current context;
- a conversation already in shared-session viewer state initializes with the repository action disabled; entering that state later removes the action from an open tooltip and performs no local candidate lookup; leaving it re-enables only the feature-specific gate;
- shared-session transitions do not change existing `NotebookLinks::resolve` results for URLs, local paths, or anchors;
- the main URL target and existing Copy, Edit, and target-specific secondary actions are unchanged.

### Editor interaction tests

Extend `app/src/notebooks/editor/view_tests.rs`:

- clicking an eligible HTTP link in the local Markdown Viewer opens the existing tooltip with **Open local file** and does not change the main link target;
- ordinary selectable web links and selectable comment-chip links retain direct-open behavior, while existing modified direct-open clicks bypass the tooltip as before;
- replacing an `AIDocumentView` editor during refresh preserves `ExistingTooltipsOnly` on the new editor handle;
- pointer and keyboard activation of **Open local file** open the local file event and do not open the URL;
- missing, ambiguous, or newly unavailable targets produce the toast and do not open the URL;
- ordinary web URLs, local-file links, anchors, non-links, normal URL activation, and existing modified clicks retain their current behavior;
- Copy, Edit, and existing target-specific secondary actions continue to dispatch their original actions;
- FilePane, NotebookPane, and AIDocumentPane receive the successful `NotebookLinks` event and forward the same canonical target to the pane group.

Run:

```bash
cargo nextest run -p warp -E 'test(link) | test(markdown_anchor)'
cargo nextest run -p warp_editor -E 'test(link) | test(mouse)'
./script/format
cargo clippy --workspace --all-targets --all-features --tests -- -D warnings
git diff --check
```

Manually open a local Markdown file containing one existing GitHub file link, one syntactically eligible missing link, and one ordinary web link. Record that a plain click shows the existing link tooltip with **Open local file** only for the two repository-browser links; its main link still opens the browser; the local action opens the existing checkout file; the missing target shows the toast; and the ordinary web link still opens directly. Also verify that an eligible link in a selectable comment chip retains its existing direct-open behavior. Capture the tooltip and the end-to-end success/failure behavior in screenshots or a short recording.

## Parallelization

Use one implementation worktree, `codex/gh13434-local-markdown-links`. Tooltip eligibility, the pure parser, resolver, remote-context transition, and pane subscription form one behavior chain; splitting them across branches would make intermediate states difficult to validate without duplicating fixtures. A second local validation agent can independently review the finished diff and run the targeted tests after the implementation branch is coherent.

## Risks and mitigations

- **Opening a file outside the repository:** canonicalize both paths and require the target to remain under the canonical root.
- **Remote/local path confusion:** require a local session and local repository root; never reinterpret a remote session's path on the host filesystem.
- **Provider URL ambiguity or parser normalization:** validate and bound the original encoded path before generic URL parsing, support only the documented GitHub/GitLab shapes and exact repository segment, and fail closed.
- **Unexpected browser navigation after failure:** keep the contextual button separate from the main URL action and never fall back to normal URL opening.
- **Tooltip false positives or stale context:** use the strict pure parser only for discoverability, then repeat context, canonical-path, and regular-file validation when the user selects the action.
- **Tooltip regression:** preserve the existing main target, Copy, Edit, and target-specific secondary actions and add focused interaction coverage for each.
- **Symlink retarget between validation and opening:** pass the verified canonical file to the existing opener, never the joined alias. Atomic protection against a later mutation of that canonical path would require a file-descriptor-based opening contract and remains out of scope.
- **Oversized crafted links:** enforce URL byte, decoded path byte, segment, and candidate limits before allocating suffixes or touching the candidate filesystem.
- **Scope expansion across renderers:** leave Agent rich output and generic detected-link code untouched.

## Follow-ups

- Evaluate Agent Mode rich-output support once it has an explicit repository-context contract.
- Evaluate line/column fragments separately.
- Add providers only with deterministic URL-shape tests rather than a suffix-search fallback.
