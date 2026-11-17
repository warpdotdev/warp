---
name: oz-browser-verification
description: Verify a web UI change or reproduce a UI bug in the sandbox's bundled browser and capture screenshots or a recording as run artifacts. Use whenever a task needs a running web page exercised or visually checked.
---

# Browser verification

Use this when you need to see or exercise a running web page: checking that a UI change renders and behaves correctly, reproducing a reported UI bug, or producing visual proof for a pull request. The sandbox has a virtual display and a bundled browser; you drive it with `playwright-cli` from the shell and attach captures to the run as artifacts.

## Preconditions

Run `command -v playwright-cli && [ -n "$DISPLAY" ]` first. If either check fails, browser verification is unavailable in this environment: say so in your report and continue without visual proof. Do not install a browser, start a display server, or set or override `DISPLAY`.

## Start the app under test

Start servers in the background with stdin redirected from `/dev/null` so they are not stopped when they read input, then wait until they are ready:

```sh
nohup <start command> </dev/null > /tmp/app.log 2>&1 &
```

## Drive the page

- `playwright-cli open --headed <url>` opens the page in a browser on the display. Keep `--headed` so screenshots and any recording show the same browser. Use `-s=<name>` to keep separate sessions apart.
- `playwright-cli snapshot` (or `playwright-cli find <text>`) lists the page's elements with refs such as `e21`. Act on refs: `click <ref>`, `fill <ref> <text>`, `press <key>`, `hover <ref>`, `select <ref> <value>`.
- `playwright-cli screenshot --hires --filename=<absolute path>` captures the current page. `playwright-cli video-start <absolute path>.webm`, then the interaction, then `playwright-cli video-stop` records a flow.
- Use `playwright-cli --help` and `playwright-cli <command> --help` for anything else instead of guessing flags.
- `playwright-cli close-all` when you are done.
- If the browser fails to launch with a sandbox error, retry as `PLAYWRIGHT_MCP_SANDBOX=0 playwright-cli open --headed <url>`.
- To drive a browser you launched yourself, start it with `chromium --remote-debugging-port=9222 <url>` and attach with `playwright-cli attach --cdp=http://localhost:9222`.

## Capture and report proof

- Write captures outside the repository with ordered, descriptive names: `/tmp/ui-verification/01-initial.png`, `/tmp/ui-verification/02-after-submit.png`, `/tmp/ui-verification/checkout-flow.webm`.
- Report every capture that supports your conclusion to the run, with a description of what it shows. Screenshots become run artifacts that the pull request description embeds inline; recordings become run artifacts with a thumbnail:

```sh
"$OZ_CLI" harness-support report-artifact screenshot --path '/tmp/ui-verification/02-after-submit.png' --description 'Checkout form after submit: success banner visible'
"$OZ_CLI" harness-support report-artifact video --path '/tmp/ui-verification/checkout-flow.webm' --title 'Checkout flow' --description 'Adding an item, submitting the form, and seeing the success banner'
```

- Never commit screenshots, recordings, or `.playwright-cli/` output (snapshots are written to the current directory). Do not add them to git.

## What counts as proof

- Exercise the real path on the actual rendered page. A mockup, a design image, or an adjacent screen is not proof.
- For a bug reproduction, capture the symptom occurring. For a fix, capture the same path with the symptom gone. For a new feature, capture the new surface and its key interactions.
- Prefer a recording for a multi-step flow and a screenshot for a single end state. Capture both when a still adds a detail the flow moves past.
- State the URL, the viewport, and the build you exercised in your report. Never claim a visual check that did not happen; if the page cannot be reached, report the blocker and the next step instead.

## In the pull request

After reporting the captures, run:

```sh
"$OZ_CLI" harness-support pr-media-markdown
```

Insert the printed blocks into the pull request body above the attribution line, videos above screenshots. The blocks carry managed markers, so when you report new captures later, run the command again and replace the whole existing block with the new one rather than adding a second copy. Describe what you verified next to the blocks; never paste local file paths into the pull request.
