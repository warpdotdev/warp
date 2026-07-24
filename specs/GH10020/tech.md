# Technical spec: Inline image protocols at shell startup (GH-10020)

This spec is the implementation companion to `product.md`. It picks the
fix site, names the affected files and line ranges, and defines a
testable invariant set.

## Current routing of `handle_completed_iterm_image` /
## `handle_completed_kitty_action`

Trace of an inline image emitted from `~/.zshrc`:

1. PTY bytes reach the ANSI processor.
2. `TerminalModel::handle_completed_iterm_image` (terminal_model.rs:3354)
   stores `StoredImageMetadata::ITerm(...)` in `image_id_to_metadata`,
   then calls the model-level `delegate!` macro
   (terminal_model.rs:2347), which routes to either `alt_screen` or
   `block_list` based on `alt_screen_active`. The shell-init case is
   not on the alt screen, so it routes to `BlockList`.
3. `BlockList::handle_completed_iterm_image` (blocks.rs:3794) calls
   the block-list-level `delegate_to_block!` macro (blocks.rs:532),
   which is `self.active_block_mut().handle_completed_iterm_image(...)`.
   Note: this is **not** the gated `delegate!` (blocks.rs:519) that
   routes through `EarlyOutput` — image completions bypass the
   early-output check.
4. `Block::handle_completed_iterm_image` (block.rs:3292) calls the
   block-level `delegate!` macro (block.rs:2882), which routes:
   - to `header_grid` if a prompt is being received,
   - to `rprompt_grid` if the right prompt is being received,
   - **to `header_grid` if `state == BlockState::BeforeExecution`,**
   - to `output_grid` otherwise.
5. `GridHandler::handle_completed_iterm_image`
   (grid/ansi_handler.rs:1266) does the actual work: dispatches
   `Event::ImageReceived`, calls `self.images.add_image_placement_data`
   and `self.images.place(...)`, then advances the cursor.

The `BootstrapStage::ScriptExecution` block has not yet executed a
user command, so it is in `BlockState::BeforeExecution`. Per step 4,
**the image is placed in `header_grid`**, not `output_grid`. The
`header_grid` is the prompt area; its image-placement is not rendered
in the visible bootstrap block content. This is the architectural root
cause of B1 (init-phase image drop).

For B2 (post-prompt background subshell drop): when output arrives in
the `is_early_output()` window (blocks.rs:3088), the active block has
been advanced to a new block in `BlockState::BeforeExecution` waiting
for the user's first keystroke. Same routing in step 4 sends the image
to `header_grid` of that block, and it is again invisible.

For the working post-prompt typed-command case: by the time the user
hits Enter, `BlockState` has progressed past `BeforeExecution` and the
image lands in `output_grid`, which is rendered in the block body.
This is why typing the same command works.

## Chosen fix site

**Block-level `delegate!` macro (block.rs:2882) is amended for
image-protocol completions only.** Image-protocol completions route to
`output_grid` even when `state == BlockState::BeforeExecution`,
provided the active block is *not* receiving prompt characters (i.e.
not in the prompt-receiving branches of the existing match).

Why not the BlockList layer (blocks.rs:3794):
- The BlockList layer does not know about `header_grid` vs
  `output_grid`; that distinction lives in `Block`. Pushing the
  routing decision up would require leaking grid identity into
  BlockList.

Why not the GridHandler layer (grid/ansi_handler.rs:1266):
- Once the image reaches a grid, it is placed in *that* grid. The
  decision of *which* grid must be made above. Moving the fix lower
  would require a "redirect" message back up, which is more invasive.

Why not change `BlockState::BeforeExecution` semantics:
- The state machine is correct as a state machine. The invariant we
  are tightening is "image-protocol output is body output, not prompt
  output," which is a routing rule, not a state transition.

## Implementation sketch

### Change 1: image-completion-aware `delegate!` in block.rs

Introduce a sibling macro `delegate_image_completion!` (or a
`is_image_completion: bool` parameter to `delegate!`) that overrides
the `BeforeExecution → header_grid` arm to use `output_grid` instead.
The two prompt-receiving arms (`Initial`, `Right`) are unchanged: a
program emitting an image inside the prompt-string itself is
sufficiently unusual that preserving today's behavior is correct.

Apply the new macro at the two image-completion sites in `Block`'s
`ansi::Handler` impl:

- `Block::handle_completed_iterm_image` (block.rs:3292)
- `Block::handle_completed_kitty_action` (block.rs:3296)

These are the only two methods affected. All other handler methods
keep the existing `delegate!`.

### Change 2: skip the bootstrap-stage gate verification

The `BlockList::handle_completed_iterm_image` /
`handle_completed_kitty_action` site (blocks.rs:3794, 3798) does not
gate on `is_bootstrapping_precmd_done()` and must continue not to.
Add a one-line comment at both sites referencing this spec to prevent
a well-meaning future PR from "fixing the missing gate." This is the
cheapest defense against B5 regression.

### Change 3: drop in the WarpInput stage only

The `WarpInput` block (the hidden block containing Warp's own
bootstrap script) must not render images, even with Change 1 in
place. Add a guard in the new `delegate_image_completion!` arm: if
`self.bootstrap_stage == BootstrapStage::WarpInput`, return without
placing. This preserves the invariant from product.md §3 (hidden
bootstrap output is not user content). `Block` already exposes
`bootstrap_stage()` (used by `bootstrap_block_contents`,
blocks.rs:3325), so no new accessor is needed.

### Change 4: warn-level telemetry on silent drop

In `GridHandler::handle_completed_iterm_image`
(grid/ansi_handler.rs:1266) and
`handle_completed_kitty_action_internal`
(grid/ansi_handler.rs:1718), every early-return path
(`!FeatureFlag::ITermImages.is_enabled()`, zero-dimension, decode
failure) emits a `log::warn!` with:
- protocol name (`"iterm"` / `"kitty"`),
- failure reason (`"feature_flag_off"`, `"zero_dimension"`,
  `"decode_failure"`),
- the active `BootstrapStage` (passed in via existing handler context
  or a new optional parameter).

The feature-flag-off case is `log::debug!`, not `warn!` (B7: no noise
when the user has explicitly disabled the protocol).

## Test plan

### Unit tests

`app/src/terminal/model/block_test.rs` — add cases that:

- T1: With `BlockState::BeforeExecution`, calling
  `handle_completed_iterm_image` routes to `output_grid`, not
  `header_grid` (assert by inspecting which grid received the image).
- T2: With `BlockState::Started`, calling
  `handle_completed_iterm_image` continues to route to `output_grid`
  (no regression).
- T3: With `header_grid.receiving_chars_for_prompt =
  Some(PromptKind::Initial)`, image completion still routes to
  `header_grid` (prompt-string image case unchanged).
- T4: With `bootstrap_stage == BootstrapStage::WarpInput`, image
  completion is dropped (no placement on either grid).

### Integration test

`app/src/integration_testing/terminal/` — add a new file
`image_protocol_at_startup_test.rs` that:

- IT1: Creates a session, advances bootstrap to `ScriptExecution`,
  feeds the exact byte sequence `\x1b_Ga=T,f=100,t=f,c=22,r=22;<b64>\x1b\\`
  (the Kitty bytes from the issue), then advances to
  `PostBootstrapPrecmd`. Asserts `Event::ImageReceived` was dispatched
  exactly once with `image_protocol: ImageProtocol::Kitty`, and the
  image is `place`d in the bootstrap block's `output_grid`.
- IT2: Same as IT1 but for iTerm bytes
  (`\x1b]1337;File=...:<b64>\x07`) and
  `image_protocol: ImageProtocol::ITerm`.
- IT3: Advances all the way to `PostBootstrapPrecmd`, then feeds
  image bytes while `is_early_output()` is true (no preexec yet).
  Asserts the image is placed in the new active block's `output_grid`.
- IT4: Existing post-prompt typed-command path continues to render
  (regression guard).
- IT5: With `FeatureFlag::ITermImages` disabled, IT2's bytes produce
  no `Event::ImageReceived` and no `log::warn!` (only `log::debug!`).

### Negative-space test

- IT6: With `bootstrap_stage == BootstrapStage::WarpInput`, image
  bytes are consumed by the parser (no panic, no error log) and no
  `Event::ImageReceived` fires. This catches a future "always render"
  regression that would surface Warp's own bootstrap-script bytes if
  any happened to look like image protocols.

## Files touched

- `app/src/terminal/model/block.rs` — new
  `delegate_image_completion!` macro near block.rs:2882; the two
  image-completion handler sites at block.rs:3292 and 3296 use it.
- `app/src/terminal/model/blocks.rs` — comment-only change at
  blocks.rs:3794 and 3798 referencing this spec to prevent a "missing
  gate" regression.
- `app/src/terminal/model/grid/ansi_handler.rs` — telemetry
  `log::warn!` / `log::debug!` calls on early-return paths in
  `handle_completed_iterm_image` and
  `handle_completed_kitty_action_internal`.
- `app/src/terminal/model/block_test.rs` — T1–T4.
- `app/src/integration_testing/terminal/image_protocol_at_startup_test.rs`
  — new file, IT1–IT6.

## Out-of-scope follow-ups (linked, not addressed in this PR)

- `precmd` zsh hook deferral failure (issue mentions output being
  "suppressed by Warp's block UI" — different code path, deserves its
  own issue).
- Sixel protocol support if/when added.
- Backfilling images into restored historical bootstrap blocks.
- Image-protocol behavior when the alt screen is entered during
  bootstrap.

## Open questions for maintainer review

1. Is `delegate_image_completion!` an acceptable name, or would the
   team prefer the boolean parameter form on the existing `delegate!`?
2. Is `log::warn!` the right level for unexpected silent drops, or is
   `log::debug!` preferred (and tracked via a separate metric instead)?
3. The bootstrap-stage gate at blocks.rs:3789 (`on_reset_grid`) and
   3724/3763 (`precmd`/`preexec`) is left alone. Confirm these gates
   are intentional and not symptoms of the same family of bug.
