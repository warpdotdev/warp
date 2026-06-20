# Manual Test Plan — User-configurable LSP servers (gh-8803)

Recording plan for the `[[editor.language_servers]]` feature on the PR, broken
into **short, separate clips** — GitHub caps PR video uploads at **10 MB each**.

Each clip below is self-contained:
- **Setup** — off-camera state to put in place *before* recording the clip.
- **Record** — the on-camera actions (keep each ≤ ~20s).
- **Expected** — what the viewer should see (the frame that matters).
- **Description (post with the clip)** — the text you post in the PR next to
  the clip. Each is written to stand alone, so a reviewer reading "Clip 3" +
  its description understands it without watching the others.

You post each clip as **"Clip N"** + its **Description**. Clips are ordered;
each assumes the previous clip's setup is in place unless its own Setup says
otherwise. The Setup / Record / Expected fields are your recording guide — only
the Description is posted.

## Prerequisites

- `~/src/gradle-test` — minimal Gradle Java project, git-init'd. `gradle init --type java-application --dsl groovy --test-framework junit-jupiter --project-name demo --package com.example` generates one.
- `~/src/go-test` — minimal Go module, git-init'd. `go mod init example.com/gotest` plus a `main.go` with a `Greeting` function.
- `~/src/go-test2` — a *second* Go module (git-init'd), used by Clip 6 to show a new `.go` file falling back to the built-in gopls after the custom is removed.
- `jdtls` on `PATH` (e.g. `brew install jdtls`).
- `gopls` at `~/go/bin/gopls` (`go install golang.org/x/tools/gopls@latest`).
- JDK 21+ via SDKMAN at `~/.sdkman/candidates/java/21.0.11-amzn` (or adjust the `JAVA_HOME` line in Clip 2's snippet).

## Pre-flight (off-camera, before Clip 1)

1. Quit any running Warp.
2. Clear persisted custom enablement and the per-server LSP caches so every flow starts fresh:
   ```sh
   sqlite3 ~/Library/Group\ Containers/2BBY89MBSN.dev.warp/Library/Application\ Support/dev.warp.WarpOss/warp.sqlite "DELETE FROM workspace_language_server WHERE kind='Custom';"
   rm -rf ~/Library/Application\ Support/dev.warp.WarpOss/lsp
   ```
3. In `~/.warp-oss/settings.toml`, remove any `[[editor.language_servers]]` entries so Clip 1 starts at the no-customs baseline.
4. Launch Warp via the run script — it builds the `.app` bundle (cargo bundle + resources + codesign) and opens it. The macOS app must run as a bundle, not via a bare `cargo run`:
   ```sh
   ./script/run
   ```
   Relaunches (Clip 5) can just run `./script/run` again — it's incremental, so fast when nothing changed — or reopen the built bundle with `open <cargo-target-dir>/debug/bundle/osx/WarpOss.app`.

---

## Clip 1 — Baseline: no custom server

**Setup:** none (pre-flight left `settings.toml` with no customs).

**Record:** open `~/src/gradle-test/app/src/main/java/com/example/App.java`.

**Expected:** footer reads *"Language support is unavailable for this file type"* — no Enable button. There is no built-in Java server and no custom is configured.

**Description (post with the clip):** *Baseline: no built-in Java LSP and no custom configured → the footer reports the file type is unsupported.*

## Clip 2 — Enable a custom server (jdtls / Java)

**Setup (off-camera):** append to `~/.warp-oss/settings.toml` and save (hot-reload loads the descriptor live):
```toml
[[editor.language_servers]]
name = "jdtls"
command = "jdtls"
args = ["-data", "{{cache_dir}}/jdtls-data/{{workspace_slug}}"]
filetypes = [{ pattern = "*.java" }]
env = { JAVA_HOME = "{{env_HOME}}/.sdkman/candidates/java/21.0.11-amzn" }
```

**Record:** open `App.java` **fresh** (close the Clip 1 pane first so the footer re-resolves) → footer shows an **"Enable jdtls"** button → click it → jdtls spawns (footer progress/indexing).

**Expected:** the button label is the descriptor's `name` ("jdtls"); clicking it launches the server.

**Description (post with the clip):** *A `[[editor.language_servers]]` entry surfaces an **Enable jdtls** button (label = descriptor `name`); clicking it launches the server.* (Paste the 6-line TOML so reviewers see the config.)

## Clip 3 — Hover + go-to-definition (the `didOpen` fix)

**Setup:** none — jdtls is the running server from Clip 2. **Wait for indexing to settle** (footer activity stops) before recording, or hover won't respond.

**Record:** hover over a method (e.g. `getGreeting`) → type-info tooltip; then Cmd-click a symbol → jumps to its definition.

**Expected:** hover and go-to-definition both work.

**Description (post with the clip):** *Hover and go-to-definition work — a custom-only Java filetype (no built-in language id) now receives `textDocument/didOpen` with the language id derived from the matched descriptor. This is the `didOpen` fix.*

## Clip 4 — Custom override of a built-in (gopls-custom / Go)

**Setup (off-camera):** append to `settings.toml` and save:
```toml
[[editor.language_servers]]
name = "gopls-custom"
command = "~/go/bin/gopls"
filetypes = [{ pattern = "*.go" }]
```
(The command is home-rooted `~/...`, not `{{env_HOME}}/...`: the command trust boundary accepts an absolute path, a leading `~`, or a bare PATH name — a placeholder-built path with separators would be rejected.)

**Record:** open `~/src/go-test/main.go` → footer shows **"Enable gopls-custom"** (not "Enable gopls") → click it → spawns → hover over `Greeting` or `fmt.Println`.

**Expected:** the footer label is the custom `name`, not the built-in binary name "gopls"; the custom config serves the file.

**Description (post with the clip):** *A custom entry overrides the built-in `gopls` for `*.go`. The footer shows the custom descriptor's `name` ("gopls-custom"), not the built-in's binary name, because the custom won the resolve.*

## Clip 5 — Persistence across restart

**Setup:** jdtls and gopls-custom enabled (Clips 2 and 4).

**Record:** quit Warp completely, relaunch (`./script/run` or reopen the bundle), then open `App.java` from `gradle-test` (and `main.go` from `go-test`).

**Expected:** both servers **auto-spawn with no Enable prompt**; hover works immediately.

**Description (post with the clip):** *Enable state persists across restart via the SQLite `workspace_language_server.kind = 'Custom'` column — servers auto-spawn with no re-prompt.*

## Clip 6 — Hot-reload semantics (optional)

**Setup:** gopls-custom running in go-test (from Clip 4/5).

**Record:** delete the `gopls-custom` block from `settings.toml` and save, then open `~/src/go-test2/main.go` (a different repo, never served by gopls-custom).

**Expected:**
- The gopls **already running in go-test keeps running** — a settings edit does not restart or stop a running server (invariant 19).
- go-test2's `.go` now resolves to the **built-in gopls**: the footer shows a built-in "gopls" CTA (Enable / Install gopls), *not* "gopls-custom". Removing the custom handed `*.go` back to the built-in — the reverse of Clip 4's override. (Go has a built-in server, so this is **not** the "unavailable" state; that only appears for filetypes with no built-in, like Java before jdtls.)

**Description (post with the clip):** *Hot-reload applies the edit live: the removed custom no longer claims `*.go` for new files (the built-in gopls takes back over), while the gopls already running in go-test is not restarted (invariant 19).*

## Clip 7 — Launch failure → toast

**Setup (off-camera):** append a descriptor whose command passes validation but isn't installed, save, and create an empty target file:
```toml
[[editor.language_servers]]
name = "ghost-lsp"
command = "definitely-not-installed-xyz"
filetypes = [{ pattern = "*.ghost" }]
```
```sh
touch ~/src/go-test/sample.ghost
```

**Record:** open `~/src/go-test/sample.ghost` → click **Enable ghost-lsp**.

**Expected:** toast — *"Failed to start LSP server "ghost-lsp" for /…/src/go-test with error …"* — it names the descriptor (not a generic message); the workspace root is shown as its full path.

**Description (post with the clip):** *A server that fails to launch raises a toast that names the descriptor (`ghost-lsp`), so the user knows which server failed.*

## Clip 8 — Failure recovery: menu reachable after a fresh failure

**Setup:** continues from Clip 7 — `ghost-lsp` has just failed to launch in this session; do **not** reopen the file.

**Record:** click the LSP status indicator (the lightning-bolt) in the footer to open its menu → click **Restart server**.

**Expected:** the menu opens immediately and offers **Restart server** / **Remove server**, even though the server just failed; **Restart server** re-attempts the launch (fails again, since the binary is still missing — but the retry is real). Before this PR's footer-reconnect fix, the menu was unreachable after a fresh enable→fail until the file was reopened. Clean up afterward with **Remove server** (or delete the `ghost-lsp` entry).

**Description (post with the clip):** *After a fresh enable→fail, the footer's LSP status menu (Restart / Remove) is reachable immediately — no need to reopen the file. The PR's footer-reconnect fix.*

## Off-camera check — secret redaction in launch logs

Not a clip (it inspects the log): secret-shaped values in a custom `command`/`args` are masked before reaching the log.

1. Add a custom secret-redaction pattern in your privacy settings (the same `CustomSecretRegex` mechanism Warp uses app-wide), e.g. `SENTINEL_[A-Z0-9]+`.
2. Put that sentinel in a descriptor arg: `args = ["--token=SENTINEL_ABC123", "-data", "{{cache_dir}}/data"]`.
3. Enable the server and find the spawn line in Warp's logs:
   `Custom LSP "<name>" starting with command: <command> ["--token=***************", …]`

**Expected:** the matched sentinel is replaced character-for-character with `*`, not printed in cleartext. `env` values and `initialization_options` are never logged at all.

---

## Keeping clips under 10 MB

- **Capture the Warp window, not the full Retina desktop.** `Cmd-Shift-5` → *Record Selected Window* (or a tight region). Full-screen Retina capture is the main reason clips balloon; a window capture usually keeps a ≤20s clip in the low single-digit MB.
- **Trim dead air** — start recording just before the key moment; skip long indexing waits.
- **If a clip still exceeds 10 MB, re-encode/downscale:**
  ```sh
  ffmpeg -i clip.mov -vf "scale=1280:-2" -c:v libx264 -crf 30 -an clip.mp4
  ```
  (downscale to ~720p, CRF 30, drop the audio track — you're captioning in text.)

## What reviewers should take away

- Custom LSPs work for files with no built-in (jdtls/Java, Clips 2–3) **and** override built-ins for shared filetypes (gopls-custom/Go, Clip 4).
- The footer shows the **custom descriptor's `name`** when a custom wins resolution — the override-regression fix.
- A custom-only filetype now receives `textDocument/didOpen` with a descriptor-derived language id, so hover/diagnostics work — the `didOpen` fix (Clip 3).
- Enable state persists across restart via the SQLite `kind` column (Clip 5).
- Settings hot-reload applies edits live — a removed custom hands its filetype back to the built-in — without restarting already-running servers (Clip 6).
- Launch failures raise a descriptor-named toast (Clip 7) and the status menu stays reachable for recovery (Clip 8).
- Secret-shaped `command`/`args` are redacted in logs (off-camera check).

## Recording tips

- Record at a resolution where the footer text is legible (a window capture at native size is both legible and small).
- Linger on the frame that matters: Clip 2 (Enable label = custom name), Clip 3 (hover = `didOpen` fix), Clip 4 (override label ≠ built-in binary name), Clip 5 (auto-spawn after restart), Clip 7 (launch toast), Clip 8 (menu reachable after failure).
- Describe each clip in the PR text (the per-clip **Description**) rather than narrating on camera — keeps clips short and lets you drop the audio track (`-an`) to save size.
