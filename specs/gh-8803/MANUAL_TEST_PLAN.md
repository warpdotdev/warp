# Manual Test Plan — User-configurable LSP servers (gh-8803)

Step-by-step scenario for capturing video evidence of the
`[[editor.language_servers]]` feature on the PR. Covers the
user-visible invariants from `product.md` without dragging on:
new-language custom, override-of-built-in, persistence across
restart, and settings hot-reload.

Expected total runtime narrating each step: ~4–6 minutes.

## Prerequisites

- `~/src/gradle-test` — minimal Gradle Java project, git-init'd. `gradle init --type java-application --dsl groovy --test-framework junit-jupiter --project-name demo --package com.example` generates one.
- `~/src/go-test` — minimal Go module, git-init'd. `go mod init example.com/gotest` plus a `main.go` with a `Greeting` function.
- `jdtls` on `PATH` (e.g., `brew install jdtls`).
- `gopls` at `~/go/bin/gopls` (`go install golang.org/x/tools/gopls@latest`).
- JDK 21+ available via SDKMAN at `~/.sdkman/candidates/java/21.0.11-amzn` (or adjust the `JAVA_HOME` line in step 2.1 to match what's installed).

## Pre-flight (off-camera state reset)

1. Quit any running Warp.
2. Clear persisted custom enablement and the jdtls project cache so every flow starts fresh:
   ```sh
   sqlite3 "$HOME/Library/Group Containers/2BBY89MBSN.dev.warp/Library/Application Support/dev.warp.WarpOss/warp.sqlite" \
     "DELETE FROM workspace_language_server WHERE kind='Custom';"
   rm -rf "$HOME/Library/Application Support/dev.warp.WarpOss/lsp/jdtls/jdtls-data"
   ```
3. In `~/.warp-oss/settings.toml`, remove any `[[editor.language_servers]]` entries so Part 1 starts at the no-customs baseline.
4. Launch Warp:
   ```sh
   cargo run --bin warp-oss
   ```

## Recording

### Part 1 — Baseline: no custom servers configured (~30s)

1.1 Open `~/src/gradle-test/app/src/main/java/com/example/App.java`.

**Expected:** Footer shows *"Language support is unavailable for this file type"*. No Enable button — there is no built-in Java server and no custom is configured.

1.2 Close the pane.

### Part 2 — Configure a custom LSP for a new language (jdtls / Java) (~90s)

2.1 Open `~/.warp-oss/settings.toml` in Warp's editor and append:

```toml
[[editor.language_servers]]
name = "jdtls"
command = "jdtls"
args = ["-data", "{{cache_dir}}/jdtls-data"]
filetypes = [{ pattern = "*.java" }]
env = { JAVA_HOME = "{{env_HOME}}/.sdkman/candidates/java/21.0.11-amzn" }
```

Save the file.

**Expected:** Settings hot-reloads silently. No visible change yet.

2.2 Open `App.java` again.

**Expected:** Footer now shows *"Language support is not currently enabled for gradle-test"* with an **"Enable jdtls"** button. The button text reflects the descriptor's `name` field.

2.3 Click **Enable jdtls**.

**Expected:** jdtls spawns. Activity in the footer (progress / indexing). Eventually settles.

2.4 Hover over `getGreeting` (or any method) in the editor.

**Expected:** Hover tooltip appears with type information. Proves jdtls is serving the file.

2.5 Cmd-click on a symbol.

**Expected:** Jumps to the definition.

### Part 3 — Custom override of a built-in (gopls-custom / Go) (~90s)

> This is the case the override regression test in `crates/integration/src/test/custom_lsp.rs` locks. The built-in `LSPServerType::GoPls` claims `*.go`; the custom should win, and the footer should reflect the custom's `name`.

3.1 In `~/.warp-oss/settings.toml`, append:

```toml
[[editor.language_servers]]
name = "gopls-custom"
command = "{{env_HOME}}/go/bin/gopls"
filetypes = [{ pattern = "*.go" }]
```

Save.

**Expected:** Hot-reloads.

3.2 Open `~/src/go-test/main.go`.

**Expected:** Footer shows **"Enable gopls-custom"** — *not* "Enable gopls". Narrate: the built-in `GoPls` server's binary name is "gopls", but the custom descriptor's `name` wins the footer label because the custom won the resolve.

3.3 Click **Enable gopls-custom**.

**Expected:** gopls spawns under the custom configuration (absolute path `~/go/bin/gopls`, not the built-in's PATH-based resolution).

3.4 Hover over `Greeting` or `fmt.Println`.

**Expected:** Hover tooltip appears with type information.

### Part 4 — Persistence across restart (~30s)

4.1 Quit Warp completely.

4.2 Relaunch Warp and open `App.java` from `gradle-test`.

**Expected:** jdtls **auto-spawns** without any Enable prompt. Hover works immediately. Narrate: enable state persisted in SQLite (`workspace_language_server.kind = 'Custom'`).

4.3 Open `main.go` from `go-test`.

**Expected:** gopls-custom **auto-spawns**, no prompt.

### Part 5 — Settings hot-reload semantics (~30s, optional)

5.1 In `settings.toml`, remove the `gopls-custom` block. Save.

**Expected:** Per product.md invariant 19, the *running* gopls keeps running — settings edits do not restart already-running servers.

5.2 Open a fresh `.go` file from a different repo (one that wasn't open when gopls-custom was configured).

**Expected:** No custom matches `*.go`, so the file falls back to "Language support is unavailable for this file type".

## What this video should make obvious to a reviewer

- Custom LSPs configured via `[[editor.language_servers]]` work for files with no built-in (jdtls/Java) **and** override built-ins for shared filetypes (gopls-custom/Go).
- The footer shows the **custom descriptor's `name`** when a custom wins resolution — the override-regression bug fixed in this PR.
- Enable state persists across restarts via the new SQLite `kind` column.
- Settings hot-reload picks up new descriptors live, but does not restart already-running servers (consistent with invariant 19).

## Recording tips

- Record at 1080p+ so the footer text is legible.
- Slow down on the diagnostic frames: 2.2 (Enable label = custom name), 3.2 (override label distinct from built-in's binary name), 4.2 (auto-spawn after restart).
- Narrate the user-visible state at each step; reviewers shouldn't need to pause to read the footer.
