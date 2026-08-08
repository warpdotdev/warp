# Open repository-browser Markdown links as local files — Tech Spec

See [`PRODUCT.md`](PRODUCT.md) for user-visible behavior.

Code reference: [`2fe6a4f567928c6f11b74021e55092e5f3e5bd79`](https://github.com/warpdotdev/warp/tree/2fe6a4f567928c6f11b74021e55092e5f3e5bd79)

## Context

Rendered Markdown in local files and notebooks uses `RichTextEditorView`. Mouse-up currently converts the mouse modifiers into `EditorViewAction::MaybeOpenFileOrUrl`, but Alt/Option is discarded one layer earlier: `RichTextElement::handle_left_mouse_up` forwards only `cmd` and `shift` through the shared `RichTextAction::left_mouse_up` trait ([`crates/editor/src/render/element/mod.rs:349-356`](https://github.com/warpdotdev/warp/blob/2fe6a4f567928c6f11b74021e55092e5f3e5bd79/crates/editor/src/render/element/mod.rs#L349-L356), [`crates/editor/src/render/element/mod.rs:672-697`](https://github.com/warpdotdev/warp/blob/2fe6a4f567928c6f11b74021e55092e5f3e5bd79/crates/editor/src/render/element/mod.rs#L672-L697)). Both `RichTextEditorView` and `CodeEditorView` implement that trait, so its signature cannot be changed in only the notebook crate ([`app/src/notebooks/editor/view.rs:3409-3569`](https://github.com/warpdotdev/warp/blob/2fe6a4f567928c6f11b74021e55092e5f3e5bd79/app/src/notebooks/editor/view.rs#L3409-L3569), [`app/src/code/editor/view/actions.rs:1162-1270`](https://github.com/warpdotdev/warp/blob/2fe6a4f567928c6f11b74021e55092e5f3e5bd79/app/src/code/editor/view/actions.rs#L1162-L1270)).

`RichTextEditorView::left_mouse_down` already receives full `ModifiersState`, but currently interprets every Alt/Option press as rich-text multiselect before mouse-up can decide to open a link. `maybe_open_file_or_url` later suppresses opening when the editor no longer has one cursor. The alternate gesture therefore needs a pressed-link state established on mouse-down, not only an extra bit on the final action. After that gate, `maybe_open_file_or_url` gives detected file paths priority and routes URL links through `NotebookLinks` ([`app/src/notebooks/editor/view.rs:1911-1993`](https://github.com/warpdotdev/warp/blob/2fe6a4f567928c6f11b74021e55092e5f3e5bd79/app/src/notebooks/editor/view.rs#L1911-L1993)).

`NotebookLinks::resolve` intentionally parses valid URLs before local paths, so an HTTP repository-browser URL always becomes `LinkTarget::Url` and opens through `ctx.open_url` ([`app/src/notebooks/link.rs:124-160`](https://github.com/warpdotdev/warp/blob/2fe6a4f567928c6f11b74021e55092e5f3e5bd79/app/src/notebooks/link.rs#L124-L160), [`app/src/notebooks/link.rs:253-299`](https://github.com/warpdotdev/warp/blob/2fe6a4f567928c6f11b74021e55092e5f3e5bd79/app/src/notebooks/link.rs#L253-L299)). Local file opening already centralizes the configured editor/Markdown Viewer choice and avoids handing executable-looking files to an unsafe system-default handler ([`app/src/notebooks/link.rs:353-400`](https://github.com/warpdotdev/warp/blob/2fe6a4f567928c6f11b74021e55092e5f3e5bd79/app/src/notebooks/link.rs#L353-L400)); the new path must finish through that code rather than introducing another opener.

Each `NotebookLinks` instance owns a `SessionSource`. A local Markdown file changes that source to the file's parent directory and its target session, while an unbound plan/notebook uses the active window session ([`app/src/notebooks/file/mod.rs:367-384`](https://github.com/warpdotdev/warp/blob/2fe6a4f567928c6f11b74021e55092e5f3e5bd79/app/src/notebooks/file/mod.rs#L367-L384), [`app/src/notebooks/link.rs:470-496`](https://github.com/warpdotdev/warp/blob/2fe6a4f567928c6f11b74021e55092e5f3e5bd79/app/src/notebooks/link.rs#L470-L496)). However, `FileNotebookView::open_remote` does not replace the model's initial `SessionSource::Active` or a previous local `Target`, so checking only whether the current source resolves to a local session would permit remote content to reuse unrelated local state ([`app/src/notebooks/file/mod.rs:586-679`](https://github.com/warpdotdev/warp/blob/2fe6a4f567928c6f11b74021e55092e5f3e5bd79/app/src/notebooks/file/mod.rs#L586-L679)). Repository detection already maps a working directory to its detected root; `FileSearchModel::repo_root_location` demonstrates the existing `DetectedRepositories::get_root_for_path` lookup and rejects local/remote type confusion ([`app/src/search/files/model.rs:72-99`](https://github.com/warpdotdev/warp/blob/2fe6a4f567928c6f11b74021e55092e5f3e5bd79/app/src/search/files/model.rs#L72-L99)).

`FilePane` and `NotebookPane` subscribe their `NotebookLinks` models to the shared `subscribe_to_link_model` adapter, which forwards local-file events to the containing pane group. `AIDocumentPane` does not currently make that subscription, so a plan can resolve a target but cannot complete the existing `NotebookLinks::open` event path ([`app/src/pane_group/pane/notebook_pane.rs:84-120`](https://github.com/warpdotdev/warp/blob/2fe6a4f567928c6f11b74021e55092e5f3e5bd79/app/src/pane_group/pane/notebook_pane.rs#L84-L120), [`app/src/pane_group/pane/notebook_pane.rs:168-212`](https://github.com/warpdotdev/warp/blob/2fe6a4f567928c6f11b74021e55092e5f3e5bd79/app/src/pane_group/pane/notebook_pane.rs#L168-L212), [`app/src/pane_group/pane/ai_document_pane.rs:62-143`](https://github.com/warpdotdev/warp/blob/2fe6a4f567928c6f11b74021e55092e5f3e5bd79/app/src/pane_group/pane/ai_document_pane.rs#L62-L143)).

Agent rich output has a separate detected-link and click path that opens `DetectedLinkType::Url` directly. It is intentionally outside this first implementation, matching the maintainer request to keep the initially separate Markdown surfaces scoped.

## Proposed changes

### 1. Preserve full mouse-up modifiers and establish a pressed-link gesture

At the shared editor boundary:

- Change `RichTextAction::left_mouse_up` in `crates/editor/src/render/element/mod.rs` to receive copyable `ModifiersState` rather than separate `cmd` and `shift` booleans, and pass the full value from `RichTextElement::handle_left_mouse_up`.
- Update both implementations. `CodeEditorViewAction` reads the same `cmd`/`shift` bits it uses today and otherwise remains behaviorally unchanged. `EditorViewAction::MaybeOpenFileOrUrl` receives explicit `primary_modifier` and `alt` fields, or the small copied modifier value.
- Update direct trait/element tests so macOS Command+Option and Windows/Linux Control+Alt reach the rich-text action without changing code-editor mouse behavior.

In `app/src/notebooks/editor/view.rs`:

- Add invocation-local `PendingAlternateLinkPress` state containing the raw target plus the existing hit-tested `block_start` and pressed `char_offset`. Set it only when primary+Alt mouse-down hits an HTTP(S) rendered link. Do not reduce identity to the URL alone: two separately rendered links may share a target.
- For that eligible press, do not start Alt multiselect. Alt-only and primary+Alt over non-link or non-HTTP(S) content retain the current multiselect path.
- On mouse-up, require the same raw target and block, and require the selection to remain the single cursor anchored by that press; a changed character offset that produced a range is a selection, not a click. Clear the pending state after mouse-up, drag selection, cancellation, focus loss, or content reset so it cannot be replayed against a later render.
- Preserve the existing primary-modifier behavior for anchors, local files, editable-link tooltips, and normal URL opening.
- Treat `primary_modifier && alt` as the local-repository alternate action only for that confirmed pending HTTP(S) link.

Do not add a key binding or setting. This is pointer state already available at the event boundary.

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

1. Extend `SessionSource` with an explicit `Unavailable` state. `FileNotebookView::open_remote` and `open_static` set it before making the new content interactive, clearing either the initial `Active` source or a previous local `Target`. Local file context sets `Target` as today; plans/notebooks intentionally retain `Active`. `Unavailable`, remote sessions, and non-local filesystem builds fail before repository or candidate filesystem lookup.
2. Require a local session and base directory from the eligible source, then ask `DetectedRepositories` for the local root containing that exact base directory. This makes a file-backed Markdown viewer use the repository containing the document, while an unbound plan/notebook uses the active local terminal repository.
3. Take the root's final directory name and pass it with the parsed URL to the pure extractor.
4. Join each relative candidate to the root and asynchronously obtain canonical paths for the root and candidates.
5. Retain only candidates whose metadata describes a regular file and whose canonical path starts with the canonical root. This rejects traversal and symlink escapes while still allowing an in-repository symlink whose canonical target remains in the repository.
6. Require exactly one distinct canonical match. Zero or multiple matches return a non-match error rather than picking a suffix.
7. Return the existing `LinkTarget::LocalFile` with the verified canonical candidate as `path`, `line_and_column: None`, the current session, and `is_markdown` derived from that canonical path. Never return the original joined path or symlink alias after validating a different canonical path.

Use a dedicated error enum that distinguishes unsupported URL, missing local repository context, invalid/unsafe path, and missing/non-file target for tests and coarse telemetry. Do not include sensitive paths or the source URL in safe logs.

This removes the actionable symlink-retarget validation/open gap while retaining the existing path-based opener. It does not introduce a file-descriptor lease or claim atomicity against a concurrent mutation of the already-canonical path after resolution; that broader filesystem capability is outside this first pass.

### 4. Route success through the existing opener and failure through one toast

In `RichTextEditorView::maybe_open_url`:

- For a confirmed pending primary+Alt HTTP(S) press, pass the original raw link target to `resolve_repository_url_as_local_file`.
- On success, call `NotebookLinks::open` with the returned target so editor preferences and executable-file safety remain centralized.
- On every expected resolution failure, add one ephemeral `No matching local file found` toast to the current window.
- Consume the alternate action in both success and failure cases. Never fall back to `ctx.open_url`; an accidental fallback would make a failed local-only request navigate away.
- Do not change tooltip state, selection, focus, clipboard, or document contents.

Unexpected I/O failures may be logged at a non-sensitive level and use the same toast. No URL or local path is included in telemetry or logs.

To make the same centralized opener complete for plans, add a `NotebookLinks` getter to `AIDocumentView`, call the existing `subscribe_to_link_model` helper from `AIDocumentPane::attach`, and unsubscribe in `detach`, matching `FilePane` and `NotebookPane`. Do not add a second plan-specific file opener.

### 5. Keep surface boundaries explicit

This change applies to `RichTextEditorView` consumers with an eligible local `NotebookLinks` context: local Markdown Viewer and local plans/notebooks. The only plan-specific plumbing is the missing `NotebookLinks` pane subscription described above. Do not modify `app/src/util/link_detection.rs` or Agent block-list click handlers in this PR. The latter lack the same document/repository context and would otherwise turn this into a cross-renderer feature.

No feature flag is necessary: normal link behavior is unchanged, failure is non-destructive, and the new branch runs only for a previously unused modifier combination.

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

### Editor interaction tests

Extend `app/src/notebooks/editor/view_tests.rs`:

- full modifiers survive `RichTextElement` dispatch for the notebook implementation while CodeEditor behavior remains unchanged;
- primary+Alt mouse-down/up on the same eligible HTTP link opens the local file event and does not open the URL;
- failure produces the toast and does not open the URL;
- normal click, primary-only click, local-file links, anchors, and non-links retain their existing behavior;
- Alt-only multiselect remains available, while primary+Alt over an eligible HTTP link does not create an extra cursor;
- primary+Alt over non-link or non-HTTP(S) content preserves existing multiselect/selection behavior;
- non-link mouse-down followed by link mouse-up, different-link down/up, a drag-created selection, focus loss, and content reset suppress and clear the pending action;
- FilePane, NotebookPane, and AIDocumentPane receive the successful `NotebookLinks` event and forward the same canonical target to the pane group.

Run:

```bash
cargo nextest run -p warp -E 'test(link) | test(markdown_anchor)'
cargo nextest run -p warp_editor -E 'test(mouse)'
./script/format
cargo clippy --workspace --all-targets --all-features --tests -- -D warnings
git diff --check
```

Manually open a local Markdown file containing one GitHub file link, one missing link, and one ordinary web link. Record that normal click opens the browser, primary+Alt opens the checkout file, and primary+Alt on the missing link stays in Warp and shows the toast.

## Parallelization

Use one implementation worktree, `codex/gh13434-local-markdown-links`. The shared editor trait/dispatcher, both action implementations, pressed-link state, resolver, remote-context transition, and pane subscription form one behavior chain; splitting them across branches would make intermediate states uncompilable or untestable. A second local validation agent can independently review the finished diff and run the targeted tests after the implementation branch is coherent.

## Risks and mitigations

- **Opening a file outside the repository:** canonicalize both paths and require the target to remain under the canonical root.
- **Remote/local path confusion:** require a local session and local repository root; never reinterpret a remote session's path on the host filesystem.
- **Provider URL ambiguity or parser normalization:** validate and bound the original encoded path before generic URL parsing, support only the documented GitHub/GitLab shapes and exact repository segment, and fail closed.
- **Unexpected browser navigation after failure:** consume the alternate gesture before async resolution and never fall back to normal URL opening.
- **Rich-text selection regression:** preserve full modifiers through the shared trait, record the pressed link on mouse-down, and suppress Alt multiselect only for the confirmed alternate-link gesture.
- **Symlink retarget between validation and opening:** pass the verified canonical file to the existing opener, never the joined alias. Atomic protection against a later mutation of that canonical path would require a file-descriptor-based opening contract and remains out of scope.
- **Oversized crafted links:** enforce URL byte, decoded path byte, segment, and candidate limits before allocating suffixes or touching the candidate filesystem.
- **Scope expansion across renderers:** leave Agent rich output and generic detected-link code untouched.

## Follow-ups

- Evaluate Agent Mode rich-output support once it has an explicit repository-context contract.
- Evaluate line/column fragments and an accessible non-pointer action separately.
- Add providers only with deterministic URL-shape tests rather than a suffix-search fallback.
