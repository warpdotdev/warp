# Manual Test Plan — User-configurable LSP servers (gh-8803)

Step-by-step scenario for capturing video evidence of the
`[[editor.language_servers]]` feature on the PR. Covers the
user-visible invariants from `product.md` without dragging on:
new-language custom, override-of-built-in, persistence across
restart, settings hot-reload, and error handling (invalid-settings
banner and launch-failure toast).

Expected total runtime narrating each step: ~6–8 minutes.

## Prerequisites

- `~/src/gradle-test` — minimal Gradle Java project, git-init'd. `gradle init --type java-application --dsl groovy --test-framework junit-jupiter --project-name demo --package com.example` generates one.
- `~/src/go-test` — minimal Go module, git-init'd. `go mod init example.com/gotest` plus a `main.go` with a `Greeting` function.
- `jdtls` on `PATH` (e.g., `brew install jdtls`).
- `gopls` at `~/go/bin/gopls` (`go install golang.org/x/tools/gopls@latest`).
- JDK 21+ available via SDKMAN at `~/.sdkman/candidates/java/21.0.11-amzn` (or adjust the `JAVA_HOME` line in step 2.1 to match what's installed).

## Pre-flight (off-camera state reset)

1. Quit any running Warp.
2. Clear persisted custom enablement and the per-server LSP caches so every flow starts fresh. The cache directory is now namespaced by a hash of the descriptor `name` (not the raw name), so just clear the whole `lsp/` dir:
   ```sh
   sqlite3 "$HOME/Library/Group Containers/2BBY89MBSN.dev.warp/Library/Application Support/dev.warp.WarpOss/warp.sqlite" \
     "DELETE FROM workspace_language_server WHERE kind='Custom';"
   rm -rf "$HOME/Library/Application Support/dev.warp.WarpOss/lsp"
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

**Expected:** Hover tooltip appears with type information. Proves jdtls is serving the file. This is the key check for the `didOpen` fix: Java has no built-in language id in Warp, so a custom-only filetype like this one previously never received a `textDocument/didOpen` and the server saw an empty document. The hover working means the language id is now derived from the descriptor's matched filetype and the document is actually opened on the server.

2.5 Cmd-click on a symbol.

**Expected:** Jumps to the definition.

### Part 3 — Custom override of a built-in (gopls-custom / Go) (~90s)

> This is the case the override regression test in `crates/integration/src/test/custom_lsp.rs` locks. The built-in `LSPServerType::GoPls` claims `*.go`; the custom should win, and the footer should reflect the custom's `name`.

3.1 In `~/.warp-oss/settings.toml`, append:

```toml
[[editor.language_servers]]
name = "gopls-custom"
command = "~/go/bin/gopls"
filetypes = [{ pattern = "*.go" }]
```

Save.

**Expected:** Hot-reloads. Note the command is home-rooted (`~/...`), not `{{env_HOME}}/...`: the command trust boundary (Part 6) only accepts an absolute path, a leading `~`, or a bare PATH name, so a placeholder-built path with separators would be rejected.

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

### Part 6 — Error handling: invalid settings and launch failure (~60s)

Two distinct surfaces: a *validation* error (caught at settings load → banner) and a *launch* failure (caught at spawn → toast).

6.1 With the valid `jdtls` entry from Part 2 still present, append an entry that reuses a built-in server's reserved name:

```toml
[[editor.language_servers]]
name = "gopls"
command = "gopls"
filetypes = [{ pattern = "*.foo" }]
```

Save.

**Expected:** Banner — heading **"Your settings file contains an error."**, body *"Invalid value for 'editor.language_servers'. The default value is being used."* Validation is all-or-nothing at the settings layer, so **every** custom stops loading, not just the bad one: reopen `App.java` and the footer is back to the Part 1 no-custom state (no "Enable jdtls"). The precise reason is in the log: `` editor.language_servers: entry `gopls`: `name` is reserved for a built-in language server ``. (Same banner, other triggers worth mentioning while narrating: a `name` with a space — invalid characters; `command = "./server"` — an unsafe relative path that would resolve against the workspace cwd.)

6.2 Remove the `gopls` entry. Save.

**Expected:** Banner clears; the valid customs load again and the footer returns to its enabled state.

6.3 Append a descriptor whose command is a bare name that passes validation but is not installed:

```toml
[[editor.language_servers]]
name = "ghost-lsp"
command = "definitely-not-installed-xyz"
filetypes = [{ pattern = "*.ghost" }]
```

Save, create an empty `~/src/go-test/sample.ghost`, open it, and click **Enable ghost-lsp**.

**Expected:** Toast — *"Failed to start LSP server "ghost-lsp" for /…/src/go-test with error …"* (the workspace root is shown as its full path). It names the descriptor (`ghost-lsp`) rather than a generic message — the launch path identifies which server failed. Remove the `ghost-lsp` entry afterward.

### Off-camera verification — secret redaction in launch logs

Not a video step (it inspects the log), but worth confirming once: secret-shaped values in a custom `command`/`args` are masked before reaching the log.

1. Add a custom secret-redaction pattern in your privacy settings (the same `CustomSecretRegex` mechanism Warp uses app-wide), e.g. `SENTINEL_[A-Z0-9]+`.
2. Put that sentinel in a descriptor arg: `args = ["--token=SENTINEL_ABC123", "-data", "{{cache_dir}}/data"]`.
3. Enable the server and find the spawn line in Warp's logs:
   `Custom LSP "<name>" starting with command: <command> ["--token=***************", …]`

**Expected:** the matched sentinel is replaced character-for-character with `*`, not printed in cleartext. `env` values and `initialization_options` are never logged at all.

## What this video should make obvious to a reviewer

- Custom LSPs configured via `[[editor.language_servers]]` work for files with no built-in (jdtls/Java) **and** override built-ins for shared filetypes (gopls-custom/Go).
- The footer shows the **custom descriptor's `name`** when a custom wins resolution — the override-regression bug fixed in this PR.
- A custom-only filetype (Java, no built-in language id) now receives `textDocument/didOpen` with a language id derived from the matched descriptor, so hover/diagnostics work — the `didOpen` regression this PR fixes (Part 2.4).
- Enable state persists across restarts via the new SQLite `kind` column.
- Settings hot-reload picks up new descriptors live, but does not restart already-running servers (consistent with invariant 19).
- Invalid descriptors fail safe: a banner names `editor.language_servers`, defaults are used (all-or-nothing), and the precise per-entry reason is logged; a server that fails to *launch* raises a toast that names the descriptor (Part 6).
- Secret-shaped `command`/`args` are redacted in logs (off-camera check).

## Recording tips

- Record at 1080p+ so the footer text is legible.
- Slow down on the diagnostic frames: 2.2 (Enable label = custom name), 2.4 (hover on Java = the `didOpen` fix), 3.2 (override label distinct from built-in's binary name), 4.2 (auto-spawn after restart), 6.1 (invalid-settings banner), 6.3 (launch-failure toast).
- Narrate the user-visible state at each step; reviewers shouldn't need to pause to read the footer.
