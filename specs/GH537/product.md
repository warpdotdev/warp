# PRODUCT.md — Honor user-defined shell bindkeys in Warp's input editor

Issue: https://github.com/warpdotdev/warp/issues/537

Figma: none provided.

## Summary

Warp's input editor currently ignores user-defined keybindings declared in the
user's shell — `bindkey` in zsh, `bind` / `~/.inputrc` in bash readline, and
`bind` in fish. When a user types in a Warp prompt, those customizations have
no effect, even though the same keys work in any other terminal running the
same shell. This spec covers honoring those user bindings inside the Warp
input editor (the prompt where shell commands are typed) for zsh, bash, and
fish, sourced from the user's actual running shell so that whatever the shell
reports is what Warp respects.

## Goals / Non-goals

In scope:

- Honoring user-defined keybindings in Warp's shell command input editor for
  zsh, bash, and fish sessions.
- Discovery via the user's live shell session (querying the shell for its
  current binding table), so dynamic and conditionally-declared bindings are
  picked up — not by parsing rc files.
- Best-effort coverage of the action set: any shell widget / readline function
  / fish input function that has a Warp-input equivalent is honored. Widgets
  with no clean Warp equivalent degrade gracefully (see below) rather than
  silently stealing the keystroke.
- Keymap modes: emacs vs vi (insert/command/visual) for the shells that
  expose them. Mode switches initiated by the user (e.g. `bindkey -v`,
  `set -o vi`, vi-mode plugins, fish bind modes) take effect without restart.
- Conflict policy with Warp's own keybindings (see Behavior #14).

Out of scope for this spec:

- Bindings inside surfaces other than the shell command input editor — the
  AI prompt input, command palette, search, settings — keep their existing
  Warp keybindings unchanged. (See Open question at #5.)
- Other shells (PowerShell, nushell, xonsh, csh family). Adding more shells
  follows the same shape but is not required for this issue to land.
- A Warp-native keybinding-import config surface where users redeclare their
  bindings inside Warp settings. The intent of this issue is "the bindings I
  already have should just work" — not "give me yet another config".
- Static parsing of `~/.zshrc`, `~/.inputrc`, `~/.config/fish/`, etc. The
  source of truth is the live shell.
- Honoring shell-level abbreviations, aliases, completions, syntax
  highlighting, or autosuggestion plugins. Only key-to-action bindings.

## Behavior

### Motivating cases

Real-world bindkey users in 2025 fall into two overlapping groups, and
the spec must serve both. The driving examples:

**Group 1: single-keystroke external widgets (TUI takeovers).**

- **atuin** binds `Ctrl-R` (history search) and Up arrow to its own
  zsh widget / bash `bind -x` command / fish function. Pressing the
  bound key opens atuin's TUI, the user fuzzy-searches their history,
  selects a command, atuin writes the result to the shell's
  `$BUFFER` / `$READLINE_LINE` / fish `commandline` and exits. The
  user is then back at the prompt with that command in the editor.
- **fzf** binds `Ctrl-R` (history fuzzy-find), `Ctrl-T` (file
  fuzzy-find), and `Alt-C` (directory fuzzy-cd) to similar shell
  widgets that invoke the `fzf` binary as a TUI.
- **Editor-launching macros** like the canonical
  `bindkey '^X^E' edit-command-line` (open `$EDITOR` to edit the
  current command).

**Group 2: continuous inline-rendering plugins** (the line editor
itself is customized — these don't fire on a single keystroke; they
hook every keystroke to paint, suggest, highlight, or expand inline).

- **zsh-autosuggestions** wraps `self-insert` and other widgets to
  paint a dimmed history-suggestion inline as the user types. Right
  arrow / End / Ctrl-E accepts the suggestion via a wrapper widget.
- **zsh-syntax-highlighting** (and **fast-syntax-highlighting**)
  hooks widgets to repaint the prompt line with syntax colors as the
  user types.
- **fish abbreviations** (`abbr`) expand on space / enter — this is
  fish's first-class feature, not a plugin, and many users rely on
  it heavily.
- **zsh-vi-mode** (jeffreytse/zsh-vi-mode) rebinds large parts of the
  keymap, swaps cursor shapes per mode, and adds surround/text-object
  operators.

The spec must honor both groups as primary v1 use cases; "v1 ships
without atuin/fzf" is not acceptable, and "v1 ships but
zsh-autosuggestions silently no longer works" is also not acceptable.
Group 1 is handled by external widget pass-through (#11.5). Group 2
needs a separate mechanism — the shell's line editor needs to be the
authority for the current prompt's display so its plugins can paint
inline. See #11.6.

### Discovery and lifecycle

1. When a Warp tab starts a supported shell (zsh, bash, fish), Warp queries
   that shell for its current keybinding table once the shell is ready to
   accept commands but before the first user keystroke is processed by the
   input editor. Until the table arrives, the input editor uses Warp's
   default keymap; once the table arrives, user bindings take effect on the
   next keystroke.

2. The query mechanism is shell-native and visible only to Warp internals —
   the user does not see the query command echoed in their scrollback, in
   history, or in any block. Equivalents in spirit (not literal):
   - zsh: `bindkey -L` for each keymap (`main`, `emacs`, `viins`, `vicmd`,
     `vivis`, `viopp`, `command`, `isearch`, `menuselect`, plus any
     user-defined keymaps).
   - bash: `bind -p` for the current keymap and `bind -p -m emacs` /
     `-m vi-insert` / `-m vi-command` for the others.
   - fish: `bind` with no args, plus `bind -M insert` / `default` /
     `visual` / etc.

3. If the shell fails to start, exits before the query completes, or returns
   an unparseable response, Warp logs a diagnostic and falls back to its
   default keymap for that tab. The tab remains usable; no user-facing error
   toast is required.

4. When the user changes their bindings inside an existing session
   (`bindkey '^X^E' edit-command-line`, `bind '"\C-x\C-e": edit-and-execute-command'`,
   sourcing a new rc file mid-session, switching emacs/vi mode), Warp picks
   up the change without requiring a restart of the tab. Discovery is
   driven shell-side at every `precmd`, so the change is detected when
   the prompt next redraws. The user-visible invariant: a binding
   declared at the shell prompt is honored starting with the first
   keystroke after Warp has parsed the next `ShellBindings` payload
   from that prompt. Keystrokes pressed during the small async window
   between the prompt firing and the payload being parsed use the
   previous keymap (consistent with the non-blocking guarantee in #26);
   declarations never block typing.

5. Each tab tracks its own bindings independently. Changing bindings in one
   tab does not affect another tab, even if both run the same shell.

6. Closing and reopening a tab re-queries from scratch. Warp does not cache
   bindings across tab restarts; the user's current shell state is always
   the source of truth.

### Honoring bindings in the input editor

7. While the user is typing in the shell command input editor, every key
   press is resolved against the precedence ladder defined in #14
   (reserved infrastructure keys → user-customized Warp keybindings →
   user shell bindings for the active keymap → Warp's default
   keybindings → default character insertion). Shell bindings are
   consulted only after the two higher tiers have been checked and have
   not produced a match. When the matched action is a shell binding and
   the bound widget has a Warp equivalent, Warp performs that action
   and consumes the keystroke. When the bound widget is unsupported
   (#11), the keystroke continues down the ladder to Warp's defaults.

8. Multi-key sequences (`^X^E`, `^[f`, `gg`, fish `\\cx\\ce`) are honored
   as a single action. Resolution rules:

   - **Mid-sequence buffering.** While Warp has received one or more
     keys that match a prefix of a longer binding but not yet a complete
     binding, no action fires and no character is inserted; Warp waits
     for the next key.
   - **Ambiguous bindings (prefix is also a complete binding).** When
     a key sequence matches both a complete binding and a prefix of a
     longer binding (the canonical example: `^[` is `vi-cmd-mode` *and*
     a prefix of `^[f`), Warp uses a 500 ms ambiguity timeout. If
     another key arrives within 500 ms that extends the prefix, the
     longer match wins. If no key arrives within the timeout, the
     complete short binding fires.
   - **Pure-prefix timeout.** When a sequence matches a prefix but no
     complete binding (e.g. partial `^X` of `^X^E`), pending keys are
     held without timeout — readline / ZLE both wait indefinitely on
     pure prefixes. The user pressing any non-extending key abandons
     the prefix immediately (next rule).
   - **Abandonment.** When a non-matching key arrives mid-sequence,
     the prefix is abandoned: Warp replays the buffered keys plus the
     just-received key through normal handling, in arrival order. Any
     of those replayed keys may itself trigger a single-key binding;
     none of them re-enter prefix accumulation until the replay
     finishes. This matches readline / ZLE behavior.
   - **No keystroke is ever silently dropped.** Either a binding fires,
     or the buffered keys are replayed.
   - **Focus loss / window blur** mid-sequence abandons the prefix
     (replay path); on refocus, accumulation starts fresh.

   The 500 ms ambiguity timeout is the standard readline default; it
   may be made configurable in a follow-up but is fixed for v1.

9. "Insert literal string" bindings (e.g. zsh `bindkey -s '^X' 'echo hi\n'`,
   readline `"\C-x": "echo hi\n"`) inject the bound text into the input
   stream as if the user had typed each character — matching shell
   input-queue semantics, not literal text insertion. A newline in the
   bound string therefore submits the line (it triggers `accept-line`),
   `^A` moves the cursor to the start, and so on. The injected
   characters are processed through the same key-resolution chain as
   real keystrokes, including any other bindings they happen to trigger.
   Plain `self-insert` (no string macro) inserts the literal key
   character at the cursor as expected.

10. Bindings fall into three categories that Warp handles differently;
    the user does not need to think about which is which, but the spec
    must be precise about each.

    **Category A — built-in widgets** (the bound action is a well-known
    ZLE / readline / fish input function — `backward-kill-word`,
    `kill-line`, `up-history`, etc.). Warp translates these to its own
    `InputAction` and executes them natively in its block-mode editor.
    Fast, no shell roundtrip.

    **Category B — string macros** (`bindkey -s` / readline string
    bindings). Handled per #9: injected back through the input
    pipeline so newlines submit and control characters trigger their
    actions.

    **Category C — external shell-function widgets** (the bound
    action is a user-defined zsh widget declared via `zle -N`, a bash
    `bind -x` shell command, or a fish function — including atuin's
    `atuin-search`, fzf's `fzf-history-widget`, custom user widgets,
    plugin-provided widgets, and `edit-command-line`). These are
    honored via pass-through: see #11.5 for the user experience.

    The full set of Category A widgets Warp must honor when bound
    includes, at minimum:

    - Cursor motion: `forward-char`, `backward-char`, `forward-word`,
      `backward-word`, `beginning-of-line`, `end-of-line`,
      `beginning-of-buffer-or-history`, `end-of-buffer-or-history`.
    - Deletion: `backward-delete-char`, `delete-char`, `backward-kill-word`,
      `kill-word`, `kill-line`, `backward-kill-line`, `kill-whole-line`,
      `unix-word-rubout`, `unix-line-discard`.
    - Yank / kill ring: `yank`, `yank-pop`, `kill-region`, `copy-region-as-kill`.
    - History: `up-line-or-history`, `down-line-or-history`, `up-history`,
      `down-history`, `history-incremental-search-backward`,
      `history-incremental-search-forward`,
      `history-search-backward` / `-forward`, fish's history-pager bindings.
    - Editing: `transpose-chars`, `transpose-words`, `upcase-word`,
      `downcase-word`, `capitalize-word`, `quoted-insert`, `tab-insert`,
      `overwrite-mode`, `undo`, `redo` (where supported).
    - Submission and abort: `accept-line`, `accept-and-hold`,
      `accept-and-infer-next-history`, `accept-search`, `send-break`
      (`^C`), `eof` / `delete-char-or-list` (`^D` on empty line).
    - Vi mode: `vi-cmd-mode`, `vi-insert`, `vi-replace`, `vi-add-next`,
      `vi-add-eol`, `vi-change`, `vi-delete`, `vi-yank`, `vi-put-after`,
      `vi-put-before`, `vi-find-next-char` / `-prev-char`, `vi-repeat-find`,
      `vi-up-line-or-history`, `vi-down-line-or-history`, `vi-goto-mark`,
      `vi-set-mark`, `vi-replace-chars`, `vi-substitute`,
      `vi-change-whole-line`, plus fish-mode equivalents.
    - Completion: `complete-word`, `expand-or-complete`,
      `expand-or-complete-prefix`, `menu-complete`, `reverse-menu-complete`.
      These trigger Warp's existing completion UI (not the shell's), but
      from whichever key the user has bound them to. Behavior of the
      completion UI itself is unchanged by this spec.

11. Category A widgets that have no clean Warp equivalent (`redisplay`,
    `quoted-insert` in edge cases, etc.) are handled as follows:

    - If the widget has a documented behavior Warp can replicate
      cheaply, Warp replicates it.
    - Otherwise the keystroke falls through to Warp's default handling
      for that key, and Warp emits a one-time-per-session diagnostic
      noting the unsupported widget. The diagnostic uses the same
      redaction policy as telemetry: widget name verbatim only when in
      the documented shell-vocabulary allowlist; user-defined or
      otherwise unknown names are written as `user-defined`. Key
      sequences and binding bodies are never included.
    - This rule does not apply to Category C (external shell-function
      widgets) — those go through pass-through, never the
      "unsupported" path.

11.5. **External widget pass-through (Category C).** When a key is
    bound to an external shell-function widget, pressing that key:

    - Briefly hands input control to the shell. Warp's block-mode
      input editor yields; the shell's line editor (ZLE / readline /
      fish-line-editor) takes over the prompt with the user's
      currently-typed buffer pre-populated as `$BUFFER` /
      `$READLINE_LINE` / `commandline` and the cursor at the same
      position the user had in Warp's editor.
    - The widget runs natively. If it draws a TUI (atuin, fzf,
      `edit-command-line` opening `$EDITOR`, etc.), the alt-screen
      handling Warp already uses for `vim` / `less` / `htop` applies —
      the widget gets full terminal control until it exits.
    - When the widget exits, the new buffer state (whatever the
      widget wrote into `$BUFFER` / `$READLINE_LINE` / `commandline`)
      is synced back into Warp's input editor and the user is
      returned to block-mode editing. The cursor position the widget
      left behind is preserved.
    - If the widget calls `accept-line` (i.e. submits the command
      itself, as some atuin configurations do), Warp treats the
      submission the same as if the user had pressed Enter in
      block mode — the resulting command is run as a Warp block.
    - The widget's stderr / stdout (anything it writes outside its
      alt-screen) renders as terminal output, like any other
      command. It does not appear in Warp's input editor.
    - Cancellation: if the widget exits without writing to the
      buffer (the user presses Esc inside atuin), Warp's editor
      content is restored to whatever it was before the binding
      fired. The user did not lose their in-progress typing.

    **Failure modes.** If the shell errors during widget invocation
    (the widget is undefined, the bound function exits non-zero, the
    shell crashes), Warp restores the user's pre-invocation buffer
    and surfaces a one-time diagnostic naming the widget. The
    keystroke is not silently swallowed and the user is never left
    with a dead prompt.

    **Latency.** Pass-through introduces a small round-trip: typically
    50–150 ms before the widget's TUI appears. This is not a hard
    invariant but the spec calls it out so the implementation budgets
    appropriately. atuin's own latency measured outside Warp is the
    floor.

    **Concurrent input.** Once Warp has yielded for the widget,
    subsequent keystrokes reach the shell directly (this is what
    makes atuin's UI navigable). Warp does not buffer or re-intercept
    keystrokes during pass-through. Returning focus to Warp's
    block-mode editor happens when the widget signals completion.

11.6. **Continuous inline-rendering plugin support.** When the user has
    plugins installed that hook every keystroke to paint, suggest,
    highlight, or expand inline (zsh-autosuggestions, zsh- or
    fast-syntax-highlighting, fish abbreviations, zsh-vi-mode's
    visual mode indicators), Warp honors them. Concretely, while the
    user types in the input editor on a tab where these plugins are
    active:

    - **Inline suggestions appear.** If zsh-autosuggestions or an
      equivalent is loaded and would have suggested a completion at
      the current buffer state, that suggestion is visible in the
      input editor in dimmed text after the cursor, exactly as it
      would render in the user's terminal without Warp.
    - **Suggestion acceptance works.** The keys the plugin binds to
      accept a suggestion (typically Right arrow, End, Ctrl-E) accept
      it the same way they would natively. Word-at-a-time acceptance
      (Alt-F when bound to a `_zsh_autosuggest_accept_word`-style
      widget) also works.
    - **Syntax highlighting renders.** If zsh-syntax-highlighting,
      fast-syntax-highlighting, or an equivalent is loaded, the
      input editor shows the same per-token coloring as the user's
      native terminal does — command vs argument, valid vs invalid
      command, matching/mismatching quote and bracket pairs.
    - **fish abbreviations expand.** Pressing space or enter after a
      typed abbreviation expands it to its full form before the
      command runs, exactly as fish does natively.
    - **vi-mode indicators are correct.** Cursor shape per vi mode
      (block in command mode, beam in insert, underline in replace)
      matches what the active vi-mode plugin would draw. zsh-vi-mode
      surround / text-object operators behave as they would natively.
    - **No double-render or flicker.** The user sees one rendered
      line per prompt — Warp's editor is not separately rendered on
      top of (or under) the shell's view of the buffer.
    - **Block mode UI is preserved.** Everything above the current
      prompt (block list, sidebar, command palette, etc.) renders
      and behaves exactly as it does today. The change is scoped to
      how the active prompt's input area is composed.

    The mechanism by which these plugins drive the input editor's
    rendering is left to the tech spec — multiple shapes are
    plausible (per-keystroke ZLE round-trip, embedding the shell's
    line editor as the rendering authority for the active prompt,
    plugin-specific query API, etc.) and the right choice is
    informed by latency measurements that the implementation must
    do. The behavioral invariants above are the bar regardless of
    how the implementation gets there.

    **Failure mode.** If the plugin emits something Warp's renderer
    can't faithfully display (an obscure ANSI sequence, a 24-bit
    color the active theme rejects, custom terminal-mode toggling),
    the plugin's output renders as plain text (not crashing) and a
    one-time-per-session diagnostic notes the limitation. The user
    is never left with a broken prompt.

    **Detection.** Warp does not need to enumerate plugins by name.
    The same `bindkey -L` / `bind -p` / `bind` query used for
    Category A/B/C bindings already surfaces the plugin's installed
    widgets (e.g., `_zsh_autosuggest_accept` shows up bound to
    Right arrow when zsh-autosuggestions is loaded). The presence
    of these widgets is the signal that the implementation should
    activate the inline-rendering path for that tab.

12. `clear-screen` (typically `^L`) clears Warp's block list to the current
    prompt, matching the user's expectation from a real terminal — even if
    Warp's default `^L` already does this, the binding must continue to
    work when remapped to another key.

### Modes

13. When the shell is in vi mode, the active keymap follows the shell's
    current mode (insert / command / visual / replace). Warp learns about
    mode transitions through the same mechanism it uses for binding
    discovery (see #4) — when the shell signals a mode change (by repaint,
    OSC, prompt update, or whichever signal the implementation lands on),
    Warp's input editor switches keymaps so the next keystroke is matched
    against the new map. Visible mode indicators (cursor shape, vim-mode
    plugin status text in the prompt) remain whatever the shell already
    drew; Warp does not add its own.
    - **Open question:** what's the canonical signal for mode change across
      the three shells? Tech spec must answer this concretely. If no
      reliable signal exists for one shell, document the fallback (poll on
      every prompt redraw, etc.).

### Precedence and conflicts

14. Resolution order for a single keystroke in the shell command input
    editor, highest priority first:
    1. Reserved infrastructure keys (see below).
    2. User-customized Warp keybindings (anything the user has explicitly
       set in Warp settings).
    3. User shell bindkeys for the active keymap.
    4. Warp's default keybindings.
    5. Default character insertion.

    Rationale: a key the user has explicitly bound in Warp settings is the
    strongest signal of intent. Below that, the user's shell bindings
    override Warp's defaults — that is the entire point of this issue. Warp
    defaults are the floor.

    **Reserved infrastructure keys.** A small set of keys is structurally
    needed for Warp ↔ shell communication (input reporting, prompt-mode
    switching, kill-buffer signaling) and cannot be honored as
    user-controlled in v1. User bindings on these keys are imported into
    Warp's debug view tagged `reserved-by-warp` and do not fire;
    bindings on every other key follow the regular precedence above.
    The reserved set per shell:

    - **zsh:** `^P` (Warp uses for `kill-buffer`), `\ei` (input reporting).
    - **bash:** `\C-p` (`kill-whole-line` for clear-buffer), `\ei`
      (input reporting), `\ep` (switch to PS1 prompt), `\ew` (switch to
      Warp prompt).
    - **fish:** `\cP` (clear input buffer), `\ep` (switch to PS1
      prompt), `\ew` (switch to Warp prompt), `\ei` (input reporting).

    These match the keys Warp's existing bootstrap already installs in
    each shell. Lifting the exception (re-implementing each integration
    point without bind-level interception) is a tracked follow-up; the
    integrations exist today and replacing them is out of scope here.

15. When a user shell binding shadows a Warp default, no warning, banner, or
    toast appears. The user already declared this binding in their shell
    config; the override is the desired behavior. Diagnostics for shadowed
    Warp defaults may be available through verbose logging but are not
    user-facing.

16. When a user shell binding cannot be honored because the bound widget is
    unsupported (#11), the keystroke falls through to Warp's default — it
    does not steal the keystroke and produce nothing. The user sees the
    Warp default fire on that key, which may differ from what their shell
    would have done. The diagnostic from #11 is the user's signal that
    something they configured isn't supported yet.

### Multi-tab and multi-shell scenarios

17. A window with multiple tabs running different shells (one zsh, one
    bash, one fish) honors each tab's bindings independently. Switching
    focus between tabs changes the active binding table to that tab's.

18. SSH sessions: when the user SSHes from a Warp tab to a remote host and
    a shell starts on the remote, Warp does not query the remote shell for
    bindings in v1. The local Warp input editor continues to use the
    bindings of the local shell that started the tab, or Warp defaults if
    the local shell wasn't a supported one. Honoring remote bindings
    requires the remote-side Warp agent and is out of scope here.

19. Subshells started inside a session (`bash` typed at a zsh prompt,
    `tmux`, etc.) keep the parent tab's binding table. The user does not
    see a re-query, and bindings the subshell may have configured do not
    take effect in the Warp input editor. Re-querying on every subshell
    transition is feasible but a follow-up.

### Surface boundaries

20. Bindings only apply while the user's input focus is in the shell
    command input editor of a tab whose shell is one of zsh / bash / fish.
    They do not affect:
    - Warp command palette, settings, search, AI prompt input,
      block-level chrome (the keystrokes there continue to use Warp's own
      keymap).
    - Tabs whose shell is not a supported shell — those tabs use Warp
      defaults.
    - Any modal overlay rendered above the input editor (file palette,
      command palette, suggestions popover focus, etc.).

21. Switching focus from the input editor to another surface and back does
    not require re-querying. The binding table from the most recent query
    remains valid for the duration of the tab unless invalidated by #4.

### AI / agent prompt input

22. The AI prompt input editor does not honor shell bindkeys by default —
    it is not a shell, and shell vi/emacs muscle memory there would
    conflict with the AI input's own conventions.
    - **Open question:** add an opt-in setting "Use my shell bindings in AI
      prompts too"? Default off either way. Resolve before implementation.

22.5. **Interaction with the shell-vs-natural-language classifier.**
    Warp's agent conversation input runs a per-keystroke classifier that
    labels the current buffer as "shell command" or "natural language",
    and the label can flicker as the user types (`cd ~/p` initially
    looks shell-y, then `cd ~/please help me` flips to NL). If
    bindkey honoring is gated on this classifier, naive gating
    produces three failure modes the v1 must avoid:

    - **Flickering inline plugins.** Dimmed autosuggestions and
      syntax-highlight colors that appear and disappear as the
      classifier oscillates mid-word. The user sees their command
      lose its highlighting between keystrokes for no visible reason.
    - **Misclassified bindkey loss.** The user presses `Ctrl-R`
      expecting atuin, but the classifier had just flipped to "NL",
      so the binding is not honored and atuin doesn't open. The
      user thinks bindings are broken.
    - **Misclassified bindkey activation.** The user is composing
      a sentence in NL, classifier briefly flips to "shell", an
      autosuggest dimmed-text suggestion appears in the middle of
      their sentence and looks like a rendering bug.

    The v1 rules that resolve these:

    a. **Explicit bound keystrokes are classifier-independent.** When
       the user presses a key bound to an external widget (Category
       C: atuin's `Ctrl-R`, fzf's `Ctrl-T`, etc.) the binding is
       always honored, regardless of the current classifier label.
       Pressing the bound key is an unambiguous intent signal from
       the user that overrides whatever the classifier last said.
       Cost of a stray accidental press is low (the widget opens,
       user dismisses it with Escape). Cost of a missed intentional
       press is high (binding feels broken).

    b. **Inline-plugin rendering is hysteretic and debounced.**
       Continuous inline plugins (autosuggest, syntax-highlighting,
       fish abbreviations) only render when the classifier has held
       "shell command" for at least the last N characters (N small,
       on the order of 3–5) and only stop rendering when the
       classifier has held "NL" for the last N characters. The
       output is debounced over a short window (~80 ms) before the
       transition takes effect. The user sees one stable rendering
       state per logical phrase of typing, not a flicker on every
       keystroke.

    c. **Transitions are clean, not animated.** When the
       hysteretic state flips, inline-plugin output disappears (or
       appears) in a single frame. No fade, no partial paint.

    d. **Explicit user override (lock).** A keyboard shortcut and a
       small affordance in the input editor let the user lock the
       current buffer to "shell mode" or "NL mode", disabling the
       classifier for that buffer. Use case: the user knows what
       they're typing and the classifier keeps getting it wrong.
       The lock resets at the next agent turn (per-buffer, not
       sticky across the conversation).

    e. **Classifier output is observable for debugging.** A
       developer setting exposes the current classifier label and
       hysteresis state in the input editor (subtle indicator).
       Off by default; used to diagnose user reports of "bindings
       are flaky in agent input".

    These rules apply whenever #22 is opted in. If #22 stays off
    (v1 default), the classifier interaction doesn't arise because
    bindkeys aren't honored in the agent input at all.

### Settings, opt-out, and discoverability

23. The feature is on by default for supported shells once it ships.
    - **Open question:** ship behind a feature flag for staged rollout
      (default off → dogfood → preview → stable), or default on from
      release? Tech spec / release plan to decide.

24. A single setting "Honor shell keybindings in input editor" lives under
    the Keybindings section of settings. Toggling it off immediately
    restores Warp's default keymap for all tabs (no restart). Toggling
    it back on resumes matching against each tab's most recently
    received binding table; any drift since the toggle was off is
    picked up on the tab's next `precmd` payload, since re-queries are
    shell-driven (see TECH.md §1). Users who want a fresh re-import
    without waiting for the next prompt can press Enter on an empty
    line, which fires `precmd` immediately.

25. The Keybindings settings page surfaces, somewhere reachable, a
    read-only view of the bindings Warp has imported for the active tab —
    enough that a user debugging "why didn't my binding work" can see what
    Warp received from the shell and which entries Warp marked unsupported.
    Format: a list of `key → action (status)` rows where status is one
    of `honored-builtin` (Category A — translated to a Warp action),
    `honored-macro` (Category B — string macro re-injected per #9),
    `honored-passthrough` (Category C — external widget routed through
    pass-through per #11.5), `shadowed-by-warp-user` (a user-customized
    Warp keybinding wins for this key), `reserved-by-warp` (one of the
    structurally reserved keys from #14), or `unsupported` (Category A
    widget Warp cannot replicate; user-defined-shell-function widgets
    do not appear in this status — they are always
    `honored-passthrough`). The exact UI is left to the tech spec; the
    behavioral requirement is that the information is discoverable
    without enabling debug logging.

### Performance and correctness invariants

26. The initial binding query must not block the user's first keystroke
    perceptibly. If the query has not completed by the time the user types,
    the keystroke is handled with Warp defaults; it is not buffered or
    delayed. Late-arriving bindings apply to subsequent keystrokes.

27. The query must not appear in the user's shell history, in scrollback,
    in the block list, or as visible output. Side effects on the shell's
    own state (kill ring, last-status `$?`, etc.) must be avoided or
    cleaned up.

28. Receiving a malformed or partial response from the shell never causes
    a crash, hang, or stuck input editor. The fallback is always: drop the
    bad data, log a diagnostic, keep using whatever binding table was
    valid before.

29. Existing Warp keybindings that the user has not customized continue to
    work unchanged on tabs running unsupported shells, on tabs where the
    feature is disabled, and on tabs where the query has not yet completed
    or failed.

## Open questions

Collected from inline references above plus a few cross-cutting ones the
tech spec must resolve:

- v1 handling of user-defined named widgets whose body is shell code (#11).
- Canonical signal for vi-mode transitions across zsh, bash, and fish (#13).
- AI prompt input opt-in for shell bindings (#22).
- Default-on vs feature-flagged staged rollout (#23).
- (Resolved) Telemetry redaction policy for widget names — see #11; the
  rule is allowlist-or-bucket, never raw user-defined names.
