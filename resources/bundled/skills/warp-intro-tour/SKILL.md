---
name: warp-intro-tour
description: Run a guided, narrated intro tour of configuring Warp by driving the running Warp app with the warpctrl CLI. Use when the user asks for a tour, walkthrough, demo, or onboarding of how to configure Warp — settings, split panes, keybindings, Warp Drive, themes — or asks to "show me around Warp" or "give me the tour".
---

# Warp intro tour

Give the user a live, narrated walkthrough of configuring Warp by driving their
**running** Warp app with `{{warpctrl_binary_name}}`. Warp moves while you
explain: settings open, panes split, keybindings and Warp Drive appear on
screen. Nothing is described in the abstract — every stop is shown.

Use this skill when the user asks for an intro tour, a walkthrough, a demo, or
onboarding for configuring Warp. For one-off control requests ("split my pane",
"open Warp Drive"), use the `warpctrl` skill directly instead — this skill is
the curated multi-stop tour.

## Relationship to the `warpctrl` skill

This skill **composes with** the bundled `warpctrl` skill; it does not replace
or fork it. `warpctrl` owns the mechanics — how to make the command available,
targeting selectors, serial-execution discipline, and troubleshooting. Read it
for anything this skill does not spell out, and follow its rules everywhere:

- Run commands **serially**, never in parallel shell calls.
- Discover the exact syntax the running build supports with
  `{{warpctrl_binary_name}} <group> help` rather than guessing.
- **Verify after every mutation** with the matching `list`, `get`, or
  `app active` command.
- Invoke close actions **only** when the user explicitly asks.

## Before you start (preflight)

Run these three checks, one at a time, and stop with a clear explanation if any
of them fails.

1. **The command exists.** `command -v {{warpctrl_binary_name}}`. If it is not
   on `PATH`, use the bundled wrapper `{{warpctrl_wrapper_path}}` or follow the
   confirmation-gated setup flow in the `warpctrl` skill. Do not install a
   symlink without asking.
2. **A controllable Warp is running.**

   ```sh
   {{warpctrl_binary_name}} instance list
   ```

   An empty list means no reachable same-channel Warp: the app is not running,
   this build does not enable Warp Control, or **Settings > Scripting** is off.
   Ask the user to open **Settings > Scripting** and enable it, then re-run.
   Local control is enabled by default on internal dogfood builds and off by
   default on Stable, Preview, and OSS until the user opts in.
   If more than one instance is listed, ask which one to tour and pass
   `--instance <instance_id>` on **every** command for the rest of the tour.
3. **Know the starting state**, so you can describe what changes and leave the
   workspace tidy:

   ```sh
   {{warpctrl_binary_name}} app active
   {{warpctrl_binary_name}} surface list
   ```

   `surface list` reports which destinations are available and why any are not.
   **Skip any stop whose surface is unavailable** and say so out loud instead of
   retrying a destination the build cannot open.

Then ask **one** setup question before the first stop: *"Want me to tidy up
(close the tour tab and extra pane) when we're done, or leave everything open?"*
Their answer decides the finale — never close anything without it.

## How to run the tour

- **Narrate first, then act.** Before each command, say in one or two sentences
  what the user is about to see and why it matters. After it runs, point at what
  changed on screen.
- **One command per step.** Chain sequentially in a single shell call or issue
  separate calls; never fan out in parallel. Warp's active target moves as the
  tour progresses.
- **Verify, then narrate the proof.** Follow each mutation with its `list`/`get`
  check so the tour never claims something that did not happen.
- **Prefer idempotent `open` over `toggle`** so a surface that is already open
  stays open instead of flipping shut mid-tour.
- **Stay read-only by default.** Inspect settings, keybindings, and themes
  freely; ask before changing any persisted preference, and offer to change it
  back.
- **Keep it moving.** The full tour is seven short stops. If the user asks for a
  specific subset ("just panes and Warp Drive"), run only those stops in order.

## The tour

### Stop 1 — A dedicated stage

Give the tour its own tab so the user's real work is untouched.

```sh
{{warpctrl_binary_name}} tab create
{{warpctrl_binary_name}} tab rename "Warp tour"
{{warpctrl_binary_name}} tab list
```

Say that tabs are cheap and nameable, and that everything from here happens in
this tab.

### Stop 2 — Settings, three ways

Settings is the main configuration surface, and Warp can jump straight to the
page or search you want.

```sh
{{warpctrl_binary_name}} surface settings open
{{warpctrl_binary_name}} surface settings open --page "Appearance"
{{warpctrl_binary_name}} surface settings open --query "font size"
```

`--page` takes the page's display name as shown in the Settings sidebar — for
example `Account`, `Appearance`, `Features`, `Privacy`, `Keyboard shortcuts`, or
`Scripting`. An unknown page name fails with a clear error, so pause and read
the sidebar rather than guessing. Mention **Scripting** by name: it is the
toggle that made this whole tour possible.

Settings also has a machine-readable side, which is how an agent configures Warp
without clicking:

```sh
{{warpctrl_binary_name}} setting list
{{warpctrl_binary_name}} --output-format json setting list --namespace appearance
```

Read a single value with `setting get <key>`. Changing one (`setting set <key>
<value>`, `setting toggle <key>`) persists a real user preference — only do it
if the user says yes, confirm with `setting get <key>`, and offer to restore the
old value at the end.

### Stop 3 — Splitting panes

Layout is where Warp stops feeling like a plain terminal.

```sh
{{warpctrl_binary_name}} pane split --direction right
{{warpctrl_binary_name}} pane list
{{warpctrl_binary_name}} pane rename "logs"
{{warpctrl_binary_name}} pane navigate --direction left
{{warpctrl_binary_name}} app active
```

Split directions are `left`, `right`, `up`, and `down`; `pane navigate` also
accepts `previous` and `next`. Call out that the split changed the active pane —
that is exactly why the tour re-checks `app active` instead of assuming. If the
user wants to see the layout stretch, `pane resize --direction right` and
`pane maximize` / `pane unmaximize` are safe, reversible extras.

Stage a command in the new pane to show the input buffer is scriptable:

```sh
{{warpctrl_binary_name}} input insert "git status"
```

Be explicit that this **stages text only** — Warp Control never presses Enter or
runs anything for the user.

### Stop 4 — Keybindings

Open the keybindings page and read the live bindings.

```sh
{{warpctrl_binary_name}} surface keybindings open
{{warpctrl_binary_name}} --output-format json keybinding list
{{warpctrl_binary_name}} keybinding get "pane_group:navigate_next"
```

`keybinding list` returns entries like `{"name": "pane_group:navigate_next",
"description": "Activate Next Pane", "keystroke": "Ctrl Shift }"}`. Pass
`keybinding get` a `name` exactly as it appears there — a good live beat is to
look up the shortcut for something the user just watched happen, like moving
between the panes from Stop 3.

Warp Control **inspects** keybindings but does not rewrite them. To actually
remap something, hand off to the bundled `change-keybinding` skill, which edits
the user's `keybindings.yaml`, and offer to do it: *"Want me to remap one now?"*
That hand-off is the point of this stop — the tour shows the surface, the other
skill makes the change.

### Stop 5 — Warp Drive

```sh
{{warpctrl_binary_name}} surface warp-drive open
```

With the panel open, explain what lives there: saved workflows, notebooks,
prompts, and environment variables the user (and their team) can reuse across
sessions. Warp Control opens the surface but intentionally cannot read or mutate
Drive contents, so invite the user to click through the panel themselves while
it is on screen.

### Stop 6 — Making it yours: themes and zoom

```sh
{{warpctrl_binary_name}} theme get
{{warpctrl_binary_name}} surface theme-picker open
{{warpctrl_binary_name}} appearance get
```

`theme list` shows what is installed on this machine. Let the user browse the
picker or that list, and if they name a theme, apply it and confirm — always
using a name `theme list` actually returned:

```sh
{{warpctrl_binary_name}} theme list
{{warpctrl_binary_name}} theme set "Cyber Wave"
{{warpctrl_binary_name}} theme get
```

`theme system-set true` follows the OS light/dark setting (with
`theme light-set` / `theme dark-set` picking each side).
`appearance font-size-increase`, `appearance zoom-increase`, and their `-reset`
counterparts are an easy, instantly visible demo — always reset afterwards
unless the user wants to keep the change.

### Stop 7 — Finding anything

```sh
{{warpctrl_binary_name}} surface command-palette open --query "theme"
```

Close the loop: every surface on this tour is also one palette search away.
Mention the neighbours worth knowing — `surface project-explorer open`,
`surface global-search open`, `surface conversation-list open`, and
`surface agent-management open` — and note that
`{{warpctrl_binary_name}} surface list` is the full, build-accurate menu.

### Finale

Recap the stops in three or four lines, then honour the cleanup answer from
preflight:

- **Tidy up** (only if they said yes): close the extra pane and the tour tab.

  ```sh
  {{warpctrl_binary_name}} pane close
  {{warpctrl_binary_name}} tab close
  ```

  Warp's normal close warnings still apply and may cancel a close — if one does,
  say so and leave it open.
- **Leave it open** (default): re-run `{{warpctrl_binary_name}} app active` and
  tell the user exactly where they are.

Revert anything you changed with their permission but they did not want to keep
(theme, zoom, settings), then point them at the `warpctrl` skill for driving
Warp on their own.

## If a stop fails

Do not silently skip a broken step or invent a workaround command:

- `no_instance` or an empty `instance list` — Warp is not running, the build
  lacks Warp Control, or **Settings > Scripting** is off.
- `ambiguous_instance` — more than one same-channel Warp is running; re-run with
  `--instance <instance_id>`.
- `unsupported_action` / `not_allowlisted` — this build does not offer that
  action. Check `{{warpctrl_binary_name}} surface list` or
  `{{warpctrl_binary_name}} action list`, then move to the next stop.
- `stale_target` — the tab or pane went away mid-tour. Re-run `app active` and
  continue from the current target.

Report the failure in one line, continue the tour with the remaining stops, and
defer to the `warpctrl` skill's troubleshooting section for anything deeper.
