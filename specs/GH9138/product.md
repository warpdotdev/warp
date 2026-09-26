# PRODUCT.md — JSON / YAML structured-data block viewer

Issue: https://github.com/warpdotdev/warp/issues/9138
Relates to: #9138
Figma: none provided

## Summary

When a terminal block's output is valid JSON or YAML, Warp renders it as an
interactive collapsible tree instead of raw text. A toggle button on the block
lets the user switch between the tree view and the original raw output at any
time. The tree is a read-only rendering lens; the block's canonical text content
— defined as the ANSI-stripped, PTY-processed output string produced by the
grid at block completion — is never altered and is always used for Copy, Share,
and AI context regardless of the current view mode.

## Goals / Non-goals

In scope:
- JSON detection and tree rendering for newly completed blocks in the current
  session.
- YAML detection and tree rendering for newly completed blocks in the current
  session.
- Toggle between tree view and raw text per block.
- Collapse / expand of individual nodes in the tree.
- Keyboard navigation inside the tree.
- "Copy value" and "Copy path" affordances on tree nodes.
- A Settings toggle to opt out of automatic structured-data rendering.
- A `WARP_RICH_OUTPUT=0` env-var escape hatch to suppress rendering for a
  specific command.

Out of scope for this spec (tracked separately in #9138):
- Image block rendering.
- Table block rendering (CSV/TSV/ASCII-box).
- TOML rendering.
- Editable / writable tree view.
- Streaming JSON rendering while the command is still running (the tree renders
  only after the block completes).
- Re-detecting JSON/YAML in restored sessions or shared blocks — restored and
  shared blocks always render as raw text on first load. A follow-up can add
  detection at restore/share time once the in-session path is stable.

## Behavior

### Detection

1. After a block completes (exit code is received), Warp strips ANSI escape
   sequences from the block's output text and attempts to parse it first as JSON
   (`serde_json`) and then, if JSON fails, as YAML (`serde_yaml`). Detection
   runs only when the "Render rich output in blocks" setting is enabled (on by
   default) and `WARP_RICH_OUTPUT` is not set to `0` in the command's
   environment.

2. A block is treated as JSON if the stripped output, after trimming leading and
   trailing whitespace, parses successfully as a JSON value. A block is treated
   as YAML if it fails JSON detection and the stripped output parses
   successfully as a YAML mapping or sequence (bare scalars like `42` or `true`
   that happen to be valid YAML are not treated as YAML blocks).

3. Detection runs in a background task after block completion. While detection
   is in progress the block renders normally as raw text. If detection succeeds,
   the block transitions to tree view without user action. Detection is not
   time-bounded by a cancellation mechanism; the input size cap (Behavior #4)
   and the structured-data limits (Behavior #4a) are the primary safeguards
   against excessive CPU use.

4. Input size limits — checked before any parsing, no error or indicator shown:
   - JSON detection is skipped if the block's output exceeds **5 MB**.
   - YAML detection is skipped if the block's output exceeds **1 MB** (stricter
     cap to limit anchor/alias amplification risk before the parser runs).
   - YAML detection is additionally skipped if the input contains YAML alias
     syntax (a `*` character immediately followed by an ASCII letter or
     underscore, e.g., `*ref`). Inputs with aliases are rejected pre-parse
     as a conservative defense against amplification attacks.

4a. Even within the size caps, detection is abandoned if the parsed structure
    exceeds **10,000 total nodes** or **50 levels of nesting**. These limits
    prevent pathological inputs from producing a tree that is impractical to
    render. The post-parse walk that enforces these limits is an additional
    safety check; the pre-parse limits in Behavior #4 are the primary defense.

5. If `WARP_RICH_OUTPUT=0` is set in the **process environment** (e.g.,
   `export WARP_RICH_OUTPUT=0` in the shell profile, or set before launching
   Warp), detection is skipped for all blocks in that session. This is a
   process-level escape hatch. Inline per-command usage (`WARP_RICH_OUTPUT=0
   my-command`) is not in scope and is not guaranteed to work.

6. Detection is not retried after failing. If the output is not valid JSON or
   YAML, the block remains raw text.

### Tree view

7. When a block is in tree view, it replaces the raw text grid with a
   collapsible tree. The root node corresponds to the top-level JSON value or
   YAML document root. Object/mapping keys are shown as labeled branches;
   array/sequence elements are shown as indexed branches (`[0]`, `[1]`, …).
   Scalar leaf values (strings, numbers, booleans, nulls) are shown inline next
   to their key or index.

8. The tree renders with syntax-aware coloring derived from the active Warp
   theme: keys in one color, string values in another, numeric/boolean/null
   values in a third. No hard-coded colors; all colors come from the theme.

9. At initial render, the top two levels of the tree are expanded; deeper levels
   are collapsed. The user can expand or collapse any node by clicking its
   disclosure triangle or pressing Space / Enter while the node is focused.

10. Clicking the disclosure triangle of a collapsed object or array expands it in
    place; clicking it again collapses it. The scroll position of the block is
    preserved across expand/collapse operations.

11. Long string values (over 120 characters) are truncated with a "… (show more)"
    inline affordance. Clicking "show more" expands the string value in place;
    clicking "show less" collapses it again.

12. Keyboard navigation: Tab moves focus to the next interactive element (node
    or affordance) in the tree; Shift-Tab moves backwards. Arrow-right expands a
    focused collapsed node; Arrow-left collapses an expanded node. Arrow-down /
    Arrow-up move focus between adjacent visible nodes.

13. Pressing Cmd-A (macOS) / Ctrl-A (Windows/Linux) while focused inside the
    tree selects the raw text content of the entire block (not the tree
    rendering), so that standard copy behavior copies the original bytes.

### Toggle

14. Each block in tree view has a "Raw" toggle button in the block's hover
    toolbar, alongside the existing Copy and other block actions. Clicking it
    switches the block to raw-text rendering. The button label changes to "Tree"
    when the block is in raw-text mode, switching back to tree view when clicked.

15. The toggle state is per-block and in-session only. Restoring a session or
    reopening Warp always renders all blocks as raw text on first load (including
    blocks that were in tree view before the session ended). The tree view is
    available only for blocks detected in the current session.

16. The toggle is visible only on hover of the block, consistent with existing
    block toolbar behavior.

### Copy behavior

17. Right-clicking a leaf value node in the tree shows a context menu with two
    items: "Copy value" and "Copy path".
    - "Copy value" copies the scalar value as a plain string (unquoted for
      strings, JSON-literal for numbers/booleans/nulls).
    - "Copy path" copies the path from the root to the node using the following
      deterministic rules:
      - Simple object keys (ASCII letters, digits, and underscores, not starting
        with a digit) use dot notation: `users.email`.
      - Array indices always use bracket notation: `users[0]`.
      - Object keys that contain dots, spaces, quotes, or any character outside
        the simple-key set use bracket-and-double-quote notation with internal
        double quotes escaped as `\"`: `data["user.name"]`,
        `data["key with spaces"]`, `data["has\"quote"]`.
      - Segments are concatenated: `users[0]["full.name"]`.

18. Right-clicking a non-leaf (object or array) node shows only "Copy value",
    which copies the subtree as pretty-printed JSON.

19. Using the block's existing Copy button (in the hover toolbar) always copies
    the raw text of the entire block, regardless of tree view state, consistent
    with how the block's serialized form is preserved.

19a. All copy affordances — including "Copy value", "Copy path", and the block
     Copy button — operate on the **obfuscated** canonical text. If the block
     contains redacted secrets (shown as `[SECRET]` or equivalent placeholders
     by Warp's secret-redaction system), those placeholders are what the tree
     displays and what is copied. The unobfuscated secret values are never
     exposed through the tree view or its copy actions.

### Settings

20. A "Render rich output in blocks" toggle in Settings → Terminal (or a
    dedicated subsection if one exists by the time this ships) controls whether
    JSON/YAML detection runs at all. Default: on. When toggled off:
    - No new detection runs for subsequently-completed blocks.
    - All blocks currently in tree view revert to raw-text rendering immediately,
      without requiring a restart or session reload. This reversion is a pure
      display switch; the parsed value and the canonical text are retained in
      memory for the lifetime of the session so toggling back on restores tree
      view instantly for blocks that were already detected in this session.
    When toggled back on, only blocks detected earlier in the same session
    reappear as tree view. Blocks that completed while the setting was off are
    not retroactively re-detected.

21. The setting applies globally across all sessions, terminals, and panes.

### Edge cases

22. A block whose output is a JSON array renders the array as the root node
    (indices `[0]`, `[1]`, …). A block whose output is a JSON scalar (bare
    number, boolean, or string at the top level) renders in tree view with a
    single root leaf.

23. A block that produces partial JSON (e.g., a command that was interrupted
    mid-output) fails detection and renders as raw text. No error indicator is
    shown.

24. A block whose command exited non-zero is still eligible for tree-view
    rendering if its output is valid JSON or YAML. Exit code does not affect
    detection.

25. A block shared via Warp Drive or a share link retains the raw text as the
    canonical form. Shared and restored blocks always render as raw text on
    first load (see Behavior #15). The shared form never bakes in the tree
    rendering.

26. When the block is in tree view and the terminal window is resized, the tree
    reflows to the new width. Node labels that no longer fit truncate with
    an ellipsis; expanding the window restores full display without requiring
    user interaction.

27. Agent Mode blocks that contain JSON or YAML output are subject to the same
    detection and rendering rules as user-command blocks.
