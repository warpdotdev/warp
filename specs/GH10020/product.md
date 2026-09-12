# Product spec: Inline image protocols render at shell startup (GH-10020)

## Problem

Inline image-protocol escape sequences emitted during shell initialization
(e.g., from `~/.zshrc`, `~/.bashrc`, `~/.config/fish/config.fish`) are
silently discarded. The same escape sequence, typed manually after the
prompt is shown in the same tab, renders correctly.

This breaks the most common consumer of inline image protocols on every
platform: shell-startup banner tools (`fastfetch`, `neofetch`, `macchina`,
`viu`, `imgcat`, `chafa` in image-protocol mode, custom OSC scripts).
Other terminals supporting these protocols — iTerm2, WezTerm, Kitty,
Ghostty — render images at startup as documented in their respective
specifications. The protocols themselves do not require the parser to be
in any "post-prompt" or "interactive" state.

The reporter additionally verified that **deferring fastfetch into a
detached background subshell** (`( sleep 0.2 && fastfetch ... ) &!`) so
its output arrives **after** the first prompt also fails to render the
image. Plain text and ANSI color escapes from the same byte stream do
render. The drop is therefore specific to the inline image-protocol
parser path under early-session state, not general PTY routing.

## Goal

Inline image-protocol bytes — Kitty graphics protocol, iTerm2 inline
image protocol, and any future image protocol Warp adds — render
identically regardless of the session phase in which they arrive,
provided the relevant feature flag is enabled and the bytes are
well-formed.

## Affected protocols

- **iTerm2 inline image protocol** — `OSC 1337;File=...:<base64>BEL`
  (gated by `FeatureFlag::ITermImages`).
- **Kitty graphics protocol** — `APC G<control_data>;<payload>ST`
  (gated by `FeatureFlag::KittyGraphicsProtocol` if present, otherwise
  by the default protocol enablement path).
- Any subsequent inline image protocol Warp adds (e.g. Sixel, if added).
  The fix must not be protocol-specific.

## Non-goals (V1)

- Adding support for new image protocols not currently parsed.
- Changing image-protocol *placement* semantics (z-order, scrolling
  behavior, cell-vs-pixel sizing). The placement that already works
  post-prompt is the desired behavior — early-session emissions should
  reach the same code path with the same outputs.
- Rendering images that arrive while the alt screen is active and that
  intentionally use the alt-screen image lifecycle. (Out of scope; this
  bug is about the main screen / block list.)
- Backfilling images into restored historical blocks during session
  restoration. (Restoration replays bytes through the same parser, so
  if the V1 fix routes them correctly, restoration benefits for free.
  But session restoration of pre-fix sessions is not a goal.)

## Behavior contract (V1)

### B1 — Renders during ScriptExecution stage

Given a shell init file that emits a well-formed Kitty or iTerm2 image
escape sequence between `BootstrapStage::WarpInput` and
`BootstrapStage::PostBootstrapPrecmd`, when the user opens a new tab,
the image is visible in the same scrollback position where its
surrounding text appears. The image must not appear above the first
visible prompt, must not appear below the next user-executed block, and
must not require a re-render to become visible.

### B2 — Renders for early-output between prompt and first user keystroke

Given the bootstrap is complete (`PostBootstrapPrecmd`) and a background
job emits an image-protocol sequence after the first prompt is shown but
before the user submits a command (the `is_early_output()` window), the
image renders in the same scrollback position as text early-output
already does today.

### B3 — Renders for output during execution (existing behavior preserved)

The existing post-prompt user-typed-command path (the case that already
works today) continues to render images identically. No regression in
the working code path.

### B4 — Renders identically across protocols and across feature-flag
combinations

For every (protocol, feature-flag-state, session-phase) tuple where the
feature flag for that protocol is enabled, the image either always
renders or always drops with the same observable reason. No combination
silently drops only because of session phase.

### B5 — No render path is gated on `is_bootstrapping_precmd_done()`

The bootstrap-stage gate exists for typeahead disambiguation and for
height/layout updates that are unsafe before the first real block. It
must not gate the image-protocol completion handlers. Any future
addition of new "completed-protocol" handlers must not inherit this
gate by accident.

### B6 — Telemetry for silent drops

If an image-protocol completion is reached for any reason that prevents
placement (zero dimensions, feature flag off, decode failure, missing
target grid), the event is logged at `warn` level with the protocol
name, the session phase, and the failure reason. Today the silent drop
in the early-session path is not logged at all, which is what made this
bug invisible for months. This is the smallest invariant that prevents
this class of bug from regressing silently again.

### B7 — Feature-flag-off path unchanged

If the feature flag for the relevant protocol is disabled, behavior is
identical to today: the bytes are consumed by the parser and the image
is not rendered. No telemetry noise from the feature-flag-off path
(only from unexpected drops with the flag on).

## Acceptance criteria

A1. With Kitty `--logo-type kitty-direct` fastfetch in `~/.zshrc`, the
    image renders in the bootstrap output area on a new tab.

A2. With iTerm2 `--logo-type iterm` fastfetch in `~/.zshrc`, same as A1.

A3. With `( sleep 0.2 && fastfetch --logo-type kitty-direct ... ) &!` at
    the end of `~/.zshrc`, the image renders after the first prompt
    appears.

A4. Manually typing the same fastfetch invocation after the prompt
    renders the image (no regression). Pixel-equivalent output to today.

A5. With `FeatureFlag::ITermImages` disabled, no image renders in any of
    the above cases (no regression in the off-path).

A6. An automated integration test asserts B1, B2, B3, and B5 by feeding
    a synthetic byte stream through the parser at each session phase
    and asserting `Event::ImageReceived` is dispatched and the image is
    placed in the correct grid.

## Risks and decisions to make in tech.md

1. **Where to fix.** Two candidate sites:
   (a) `BlockList::handle_completed_iterm_image` /
       `handle_completed_kitty_action` (blocks.rs:3794, 3798) — route to
       the visible bootstrap block instead of dropping when the active
       block is a hidden bootstrap block.
   (b) `Block::delegate!` macro (block.rs:2882) — route image
       completions to `output_grid` even when `state ==
       BeforeExecution`, since `header_grid` placement is not rendered
       in the same way.
   The TECH spec must pick one and justify against the other.

2. **Background-subshell case (B2).** The `is_early_output()` path
   (blocks.rs:3088) routes most output to `EarlyOutput::handler`, but
   image completions skip the early-output check (they use
   `delegate_to_block!` directly, not `delegate!`). The fix may need to
   either teach `EarlyOutput` to forward image completions, or change
   the early-output check to exclude image completions from the
   typeahead split.

3. **Alt-screen interaction.** A program that switches to the alt
   screen during shell init (rare but possible — `tput smcup` in a
   `.profile` is legal) emits images that go to `AltScreen::handle_*`.
   The fix must not change alt-screen behavior; that path is correct
   today.

4. **Hidden vs visible bootstrap blocks.** `BootstrapStage::WarpInput`
   is hidden (`bootstrap.rs::is_hidden`); `ScriptExecution` is not.
   Images emitted while the active block is the hidden WarpInput block
   should not render (those bytes belong to Warp's own injected
   bootstrap script and are not user content). Images emitted while
   the active block is the ScriptExecution block must render. The TECH
   spec must make this distinction explicit.

## Reporter-supplied diagnostic facts (preserved)

From the issue, verified by the reporter via stdout capture:

- `\e_Ga=T,f=100,t=f,c=22,r=22;<base64>\e\` (Kitty graphics) is emitted
  by fastfetch during shell init, identical bytes to the post-prompt
  case.
- `--pipe false` forces emission regardless of TTY auto-detection;
  drops occur even with this flag.
- `precmd` zsh hook deferral: output is suppressed by Warp's block UI
  (different failure mode — out of scope for this spec).
- Pre-rendering with `chafa --format symbols` (text + ANSI color) works
  in all phases, confirming the parser-state-dependent drop is specific
  to the inline image-protocol path.

These should be cited verbatim in the integration test fixture so the
test fails for the same observable reason the user reported.
