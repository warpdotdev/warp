# gh-8803: User-Configurable Custom Language Servers

> Filed against GitHub issue [warpdotdev/warp#8803](https://github.com/warpdotdev/warp/issues/8803).

## Summary

Let users register custom Language Server Protocol (LSP) servers in their Warp settings so the editor can offer code intelligence (diagnostics, hover, go-to-definition, completions) for languages Warp does not ship support for out of the box. Configuration mirrors the shape of Neovim's `vim.lsp.config`: a server name, binary command and arguments, and the filetypes it serves. The workspace root is the directory currently open in the Warp window — the same root used by built-in servers today.

The five built-in language servers (rust-analyzer, gopls, pyright-langserver, typescript-language-server, clangd) continue to ship and work as they do today. Users can extend coverage to additional languages by configuring custom servers through `[[editor.language_servers]]`, and can override any of the five built-ins by writing an entry whose `filetypes` overlap a built-in language.

## Problem

Warp's editor today only attaches an LSP client for five built-in languages (Rust, Go, Python, TypeScript/JavaScript, C/C++). Anyone working in Ruby, Zig, Lua, Terraform, OCaml, Haskell, Elixir, Swift, Kotlin, Bash, etc. opens a file and gets nothing — no diagnostics, no hover, no completions — even if the appropriate language server is already installed on their machine. Users have no in-app path to fix that.

## Behavior

### Defining a custom server

1. Users can declare one or more custom language servers in their Warp settings file under a new `[[editor.language_servers]]` array-of-tables. Each entry has these fields:
   - `name` (string, required) — A unique identifier for this server within the user's settings, e.g. `"ruby-lsp"`. Used in UI surfaces (e.g. the footer "Enable {name}" button) and log output. The per-server cache directory's path component is derived from a hash of `name` rather than from `name` directly (see invariant 5's `{{cache_dir}}`), which decouples the cache-dir path from any platform-specific path-segment constraints. **Constraints:** 1–64 characters, drawn from `[A-Za-z0-9._-]` (ASCII letters, digits, dot, underscore, hyphen). Must not be `.` or `..`, must not start with `.` or `-`, and must not be empty. Uniqueness and reserved-name checks are **case-insensitive** (ASCII fold). Reserved names are the five built-in binary display names: `rust-analyzer`, `gopls`, `pyright-langserver`, `typescript-language-server`, `clangd` — the strings the footer "Enable {name}" button shows for built-in servers. The reservation exists solely to prevent footer-label ambiguity (two "Enable rust-analyzer" buttons for two different servers in the same workspace, which can happen when a custom targets a filetype the built-in doesn't claim). SQLite key collisions are not at issue because the table has a `kind` discriminator column. Cache-dir collisions are not at issue because the path is hashed. The reservation list is sourced from `LSPServerType::binary_name()` so adding a built-in automatically extends it. Names violating any of these constraints are settings errors per invariant 23.
   - `command` (string, required) — Path to the server binary. Must be **either** an absolute path (after `~`/`~/` home expansion per invariant 5) **or** a bare name with no path separators (`/` or `\`), which will be resolved against the user's `PATH`. "Absolute" is platform-specific: on Unix, starts with `/`. On Windows, starts with a drive letter followed by `\` or `/` (e.g. `C:\bin\server`, `C:/bin/server`) **or** is a UNC path (`\\server\share\path` or `//server/share/path`). Windows root-relative paths (`\path`, `/path` without a drive) are **not** considered absolute — they depend on the current drive at exec time, which is exactly the cwd-dependent resolution this rule is preventing. **Relative paths containing separators (e.g. `./server`, `bin/server`, `..\\server`, Windows `\server`) are rejected** as settings errors per invariant 23 — these forms would otherwise resolve directly against the spawned process's cwd (the workspace root, per invariant 16). For the PATH-based residual case (`$PATH` containing relative entries), see invariant 16.
   - `args` (array of strings, optional, defaults to `[]`) — Arguments passed to `command` on launch.
   - `filetypes` (array of inline tables, required, non-empty) — Patterns that claim files for this server. Each array entry is an inline table `{ pattern = "...", language_id = "..." }` where `pattern` is required and `language_id` is optional. The LSP `languageId` Warp sends for matched files is the entry's `language_id` when provided; otherwise it defaults to the matched file's lowercase extension, or to the file's literal basename when there is no extension. Use an explicit `language_id` to override the default, both for servers that expect the LSP-standard identifier (e.g. `{ pattern = "*.rb", language_id = "ruby" }`, `{ pattern = "*.sh", language_id = "shellscript" }`) and for servers that speak multiple languageIds (e.g. `{ pattern = "*.ts", language_id = "typescript" }` and `{ pattern = "*.tsx", language_id = "typescriptreact" }` in the same entry). The `pattern` field takes one of two syntactic forms:
     - **Glob** — contains any of `*`, `?`, or `[` (e.g. `"*.rb"`, `"*.rake"`, `"Dockerfile.*"`). Matched against the file's basename only (not the full path) using POSIX-style glob semantics — the syntax accepted by Rust's [`glob` crate `Pattern`](https://docs.rs/glob/latest/glob/struct.Pattern.html), which is a strict subset of POSIX.1-2017 §2.13 Pattern Matching Notation. Supported metacharacters are `*` (any sequence of characters except path separators), `?` (any single character), `[abc]` / `[!abc]` (character class / negated class), and `[a-z]` (ranges). Glob matching is case-insensitive — `"*.rb"` matches both `foo.rb` and `FOO.RB`. Brace alternation (`{a,b}`) and double-star recursion (`**`) are **not** supported in v1, since matching is basename-only.
     - **Literal basename** — any pattern that contains none of `*`, `?`, or `[` (e.g. `"Gemfile"`, `"Rakefile"`, `".bashrc"`). Matches files whose basename equals it exactly, case-sensitively. To match files by extension, write a glob (`"*.rb"`, `"*.ts"`); a bare token like `"rb"` is **not** treated as an extension match — it is a literal basename match against a file literally named `rb`.
   - `env` (table of string → string, optional, defaults to `{}`) — Extra environment variables merged into the server process's environment on launch.
   - `initialization_options` (arbitrary TOML value, optional) — Sent as the `initializationOptions` field of the LSP `initialize` request. Warp does not restructure the value's shape; per invariant 5, string leaves go through placeholder substitution before send, while non-string values pass through unchanged.

2. `name` must be unique across all entries in `[[editor.language_servers]]`. Two entries with the same `name` are a settings error; see invariant 23. Uniqueness is **case-insensitive**: `ruby-lsp` and `Ruby-LSP` are duplicates and produce a settings error. This is a UX rule: distinct-but-case-confusable names produce indistinguishable footer labels ("Enable ruby-lsp" vs. "Enable Ruby-LSP") and log entries, and case-insensitive uniqueness eliminates that ambiguity at validation time. Cache-dir collisions are not at issue because the cache-dir path component is hashed (see invariant 5's `{{cache_dir}}`).

3. Custom server entries override built-in servers when their `filetypes` overlap with a built-in language. For example, an entry with `filetypes = [{ pattern = "*.rs" }]` replaces the built-in `rust-analyzer` mapping for `.rs` files for that user. Removing the custom entry restores the built-in mapping with no further action.

4. When multiple custom entries' `filetypes` patterns could match the same opened file, the first entry in source order in the settings file wins. Built-in language→server mappings are only consulted if no custom entry matches. Overlap between entries is not a settings error — order is the disambiguator.

### Placeholder substitution

5. The string values of `command`, each entry of `args`, each value of `env`, and every string leaf inside `initialization_options` undergo template substitution at launch time. Non-string values (numbers, booleans, arrays, tables) inside `initialization_options` pass through unchanged; only their string children are substituted. Substitution uses the same `{{name}}` template syntax as Warp's tab configs and MCP server rendering, so the convention is consistent across Warp settings files. The following placeholders are recognized:
   - `{{workspace_root}}` — Absolute path to the resolved workspace root (see invariant 12).
   - `{{workspace_slug}}` — A short stable identifier derived from `{{workspace_root}}`, safe to use as a directory name. The same workspace root produces the same slug across launches. The slug is a deterministic truncated hash of the workspace root; in practice two different workspace roots produce different slugs, but collisions are theoretically possible (see tech.md for the implementation and the accepted collision bound).
   - `{{cache_dir}}` — A per-server, per-user cache directory owned by Warp (under the OS cache dir). The path component identifying this server is a deterministic hash of the entry's `name` — the same SHA-256-prefix shape used for `workspace_slug` — so that the cache-dir path is always a safe segment on every supported platform regardless of what characters `name` contains. The hash is stable across launches; the same `name` always produces the same path. Warp creates the directory before launch. Suitable as a parent for server scratch state.
   - `{{env_VAR}}` — The value of environment variable `VAR` in Warp's process environment at launch time. The `env_` prefix is used because the template parser only accepts alphanumeric characters, `-`, and `_` in placeholder names; `{{env_HOME}}` expands to the value of `$HOME`. An undefined variable expands to the empty string and is logged.

6. Substitution is single-pass within a string: a substituted value containing `{{...}}` syntax is not re-expanded. Unknown placeholders (`{{...}}` patterns that do not match any name above) expand to themselves verbatim and are logged once per launch. Whitespace inside the braces invalidates the placeholder, so `{{ workspace_root }}` is not expanded. A single `{` or `}` is ordinary text. Escaping uses the same shared template engine as Warp's tab configs and MCP rendering, so it behaves consistently across those surfaces: a name wrapped in a third pair of braces (`{{{workspace_root}}}`) is treated as escaped — it is not substituted and is passed through verbatim, braces included, so `{{{workspace_root}}}` reaches the spawned process as the literal `{{{workspace_root}}}`. (This differs from standard Handlebars, where `{{{...}}}` denotes unescaped substitution; Warp's engine treats it as a do-not-substitute marker.)

   In addition to `{{...}}` placeholders, a leading `~` or `~/` at the start of any substituted string expands to the current user's home directory. `~` is expanded only at the very beginning of a value; embedded `~` characters (e.g. `/opt/~/bin`) are passed through unchanged. Other-user home expansion (`~someuser/...`) is not supported. `~` expansion is needed because Warp spawns the server with a direct OS `exec`, not through a shell — without it, `command = "~/bin/lsp-server"` would fail with "no such file or directory."

7. Substitution applies before the process is spawned, after settings validation. A custom entry whose post-substitution `command` resolves to a non-existent path follows the same error path as any other failed launch (see invariant 18).

8. Reordering, adding, or removing entries in the settings file takes effect on the next file open for that filetype. Already-running servers are not restarted by an edit to their entry; users can stop and reopen a file to pick up the new config (see invariant 19).

### Opening a file

9. When the user opens a file in the Warp editor, server resolution proceeds as:
   - If the file matches a built-in language and no custom entry overrides it, behavior is unchanged from today.
   - If the file matches exactly one custom entry's `filetypes` (or matches multiple, with first-in-source-order winning per invariant 4), that custom server is the candidate for this file.
   - If the file matches no entry (built-in or custom), the footer surfaces the same "Language support is unavailable for this file type" state it shows today. No new footer affordance, link, or affordance text is introduced by this feature.

10. The footer's visible behavior, copy, and interaction model are unchanged from today. Custom servers participate in every existing footer surface — status indicator, install progress, the per-workspace Enable button, error messages — via the same code paths built-in servers use. The only difference is that the server's display name and status come from a custom entry instead of a built-in `LSPServerType`.

### Enabling a server per workspace

11. The first time a server (built-in or custom) is a candidate for a workspace root, the footer surfaces the existing "Language support is not currently enabled for `<codebase>`" affordance with its Enable button, unchanged from today. Accepting attaches and persists per-workspace state exactly as it does today for built-ins; custom servers reuse the same persistence and Enable flow.

12. The workspace root used for the enable prompt is the directory currently open in the Warp window — the same root used by built-in servers today.

13. Accepting the prompt persists the choice: that server is automatically enabled for that workspace root on subsequent file opens in the same session and across restarts.

14. Declining the prompt persists for the session: the server is not launched and the prompt does not reappear for that workspace root until the next app launch. v1 has no in-app surface to re-enable a declined server before the next app launch; restart the app to be re-prompted (see non-goal 31).

15. If the user has multiple Warp windows open on the same workspace root, accepting or declining the prompt in one window applies to all of them. A single server process is shared across windows on the same root, consistent with today's built-in behavior.

### Server lifecycle

16. When a server is enabled for a workspace and a matching file is opened, Warp launches the post-substitution `command` with the post-substitution `args`, the merged environment from the user's shell environment and the entry's `env` (also post-substitution), and the working directory set to the resolved workspace root. The server is launched once per workspace root; subsequent file opens reuse it.

    **PATH-based residual risk.** When `command` is a bare name, resolution against `$PATH` is performed by the OS process loader (via `Command::new(...)`, which on Unix uses `execvp` and on Windows uses `CreateProcess` wrapped in `cmd.exe /c` so `.cmd`/`.bat` scripts resolve correctly). If the user's `$PATH` contains relative entries (`.`, `..`, `bin`, or empty entries from a stray `::`), those entries are evaluated against the spawned process's cwd at exec time — the workspace root — so a workspace-controlled binary could still satisfy a bare command. This is the **same PATH-resolution behavior used by Warp's built-in language servers today** (`crates/lsp/src/command_builder.rs`, `crates/lsp/src/config.rs::command_and_params`): Warp does not sanitize `$PATH` at LSP spawn for either built-in or custom servers. Users who do not trust the contents of the workspaces they open should set `command` to an absolute path; that bypasses PATH resolution entirely. A future Warp-wide change to `CommandBuilder` could sanitize relative `$PATH` entries for all LSP spawns, but is out of scope for this spec.

    **The post-substitution `command` is not revalidated against the literal-form absolute-or-bare-name rule.** Placeholder substitution (invariant 5) can produce any string the user's environment dictates — including paths the literal-time validator would have rejected, such as a `$LSP=./server` env value substituted into `command = "{{env_LSP}}"`. The user is responsible for the contents of their environment, just as they are for the literal text of `settings.toml`. This matches how Warp treats user-controlled inputs everywhere else: `$PATH` for built-in LSPs is not sanitized, the MCP-server `command` field is not revalidated after env interpolation, and the terminal runs literally what the user types. The literal-text validator's job is to catch obvious settings-file errors (a hand-typed `./server`), not to defend against the user's own environment.

17. The LSP `initialize` request sends the resolved workspace root as a `workspaceFolders` entry and passes the **substituted** `initialization_options` (per invariant 5 — string leaves go through placeholder substitution; non-string values pass through unchanged) when provided. Warp does not restructure the value's shape.

18. If `command` cannot be found on `PATH` (and is not an absolute path), or if the launch fails (non-zero exit before initialization, missing executable bit, etc.), the failure surfaces through the existing footer error path — the same inline error rendering used today for built-in server failures, with the server's `name` and a one-line description of the failure. The editor continues to function without LSP support for that file.

19. Editing or removing an `[[editor.language_servers]]` entry in the settings file does not affect an already-running server for that entry — neither restarting it with new values, nor stopping it on removal. The running server keeps reflecting the configuration from its most recent launch, and continues to serve in-flight requests from open files. New values (or the absence of the entry) take effect only on the next launch, which the user triggers by closing the workspace's editor panes for that filetype and reopening a file, or via an explicit restart action (out of scope to design here; the requirement is that subsequent launches honor the current settings).

### Filetype matching details

20. Filetype matching uses the two forms defined in invariant 1's `filetypes` field: case-insensitive shell-style glob against the basename, and case-sensitive literal basename match. The file's basename is computed from the opened file's path; no other metadata is consulted.

21. Content sniffing (e.g., inspecting a shebang line, parsing file contents to detect language) is out of scope. A bash script named `deploy` with `#!/usr/bin/env bash` at the top is only claimed by a custom entry if `"deploy"` appears in some entry's `filetypes`, or a glob like `"deploy*"` matches. Users who want arbitrary extensionless shell scripts to be claimed must enumerate them.

22. A file whose extension or basename is claimed by a custom entry but whose contents look like a different language (e.g. a `.ts` file that's actually JSON) is still routed by the matched entry. Content sniffing is out of scope per invariant 21.

### Settings validation and errors

23. The following are settings errors and surfaced on settings load:
   - Duplicate `name` across entries.
   - An entry with empty `filetypes`.
   - An entry missing `name` or `command`.
   - An entry whose `name` violates the constraints in invariant 1.
   - An entry whose `name` matches a reserved built-in binary display name, case-insensitive: `rust-analyzer`, `gopls`, `pyright-langserver`, `typescript-language-server`, `clangd`.
   - An entry whose `command`, after `~`/`~/` home-directory expansion, is neither absolute nor a bare name (per invariant 1's `command` rule — Unix `/`-rooted, Windows drive-letter or UNC; root-relative `\path` rejected).
   - An inline-table entry in `filetypes` missing `pattern`.
   - An entry in `filetypes` whose `pattern` field fails to compile as a valid shell-style glob.

   When any entry is invalid, the entire `[[editor.language_servers]]` setting fails to load — no **new** custom servers are launched until the file is fixed (already-running servers keep running per invariant 19) — and the existing settings-error banner surfaces `editor.language_servers` as an invalid value. This matches how every other array-valued setting in Warp behaves (e.g. `agents.profiles.agent_mode_command_execution_allowlist`). Per-entry reasons (which entry, which field) are written to the log so users can investigate when the banner alone isn't enough. The settings file itself is not auto-edited.

24. Unknown fields on an `[[editor.language_servers]]` entry are ignored with a warning logged but no in-app notification. This leaves room to add fields without breaking existing settings files.

25. Warp generates a JSON Schema for the `[[editor.language_servers]]` array as part of the existing build-time settings schema artifact. The schema is consumed by **external editors that support TOML schema validation** (e.g. editing `settings.toml` in another editor via a TOML language server) and by Warp's in-app docs page. The schema describes every field above with descriptions and required/optional markers, and enumerates the recognized `{{...}}` placeholders (`{{workspace_root}}`, `{{workspace_slug}}`, `{{cache_dir}}`, `{{env_VAR}}`), the leading-`~`/`~/` home-directory expansion, and the `{{{...}}}` escape (a placeholder wrapped in a third pair of braces is passed through verbatim). Warp's in-app `settings.toml` text view is not schema-aware in v1; schema-driven autocomplete inside Warp is a separate follow-up.

### Non-goals

26. v1 does not install language server binaries on the user's behalf. Users are responsible for making `command` resolvable. Install instructions may be documented on the public docs page that accompanies this feature (see invariant 25's external-tooling JSON Schema, which the docs page builds on), but Warp itself does not run package managers, and per invariant 9 no new footer link, banner, or affordance is introduced.

27. v1 does not support per-workspace `.warp/settings.toml` overrides of `[[editor.language_servers]]`. Custom servers are defined at the user level only. Per-workspace overrides are a likely follow-up but not part of this feature.

28. v1 does not support multiple language servers attached to the same file simultaneously (e.g. a linter LSP and a navigation LSP on the same `.rb` file). Neovim allows this; Warp's existing one-server-per-language model is preserved for v1.

29. v1 only supports the stdio transport. The server is launched as a child process and communicates over its stdin/stdout. TCP, named pipes, and other LSP transports are out of scope.

30. v1 does not support merging a custom entry's `initialization_options` or other fields into a built-in server's configuration. A custom entry whose `filetypes` overlap a built-in language fully replaces the built-in server for those filetypes (see invariant 3). Users who want to tune a built-in server's behavior must define a complete custom entry that supplies its own `command`, `args`, and other fields.

31. v1 does not ship a dedicated inspection or management surface (command-palette action, settings sub-page, or status-dropdown extension) for listing which servers are configured and running, resetting per-workspace enable/decline state, or restarting a server for a workspace root. The footer's existing per-workspace Enable button and status dropdown carry over from today's built-in flow and apply to custom servers too; a richer inspection/management surface is deferred to a future release.

### Logging and redaction

32. **Log redaction.** When Warp logs custom-server launch information (substituted `command`, `args`, `env` values, and string leaves of `initialization_options`), values that match Warp's existing secret-redaction patterns (`app/src/settings/privacy.rs::CustomSecretRegex`) are redacted before being written. This applies to both the substituted output and the raw descriptor values, and applies at every log level. Users who reference `{{env_VAR}}` placeholders get the same redaction protection as elsewhere in Warp; raw `env` *values* in `settings.toml` are also redacted on launch logging.

   The following are **logged verbatim** (not passed through secret redaction): descriptor `name`, `env` *keys*, `workspace_slug` (a SHA-256-derived hex string with no PII), the resolved `workspace_root` absolute path, and the per-server `cache_dir` absolute path. `workspace_root` and `cache_dir` are absolute filesystem paths and may contain usernames, company names, or private repository names — for example, `/Users/alice/work/acme-internal-repo` — and Warp does **not** treat these as secret-redaction targets. This matches Warp's existing log behavior for built-in language servers (`crates/lsp/src/config.rs::command_and_params` and the LSP log filenames in `app/src/code/lsp_logs.rs`, both of which include workspace-derived paths verbatim today). `CustomSecretRegex` matches token-shaped secrets, not arbitrary PII; users who consider workspace paths sensitive in their environment must filter or redact their log files before sharing them, just as they would for any other Warp log output.

## Worked example: Eclipse JDT Language Server (Java)

This example confirms that the fields and placeholder set above are sufficient to launch a non-trivial language server. JDTLS is not a single binary — it ships as a directory tree containing a versioned launcher jar and platform-specific configuration dirs, requires a JDK 21+ runtime, and demands a unique `-data` directory per workspace. A user who has installed JDTLS at `/opt/jdtls` and has `java` on their `PATH` can register it as:

```toml
[[editor.language_servers]]
name = "jdtls"
command = "java"
args = [
  "-Declipse.application=org.eclipse.jdt.ls.core.id1",
  "-Dosgi.bundles.defaultStartLevel=4",
  "-Declipse.product=org.eclipse.jdt.ls.core.product",
  "-Xmx1G",
  "--add-modules=ALL-SYSTEM",
  "--add-opens", "java.base/java.util=ALL-UNNAMED",
  "--add-opens", "java.base/java.lang=ALL-UNNAMED",
  "-jar", "/opt/jdtls/plugins/org.eclipse.equinox.launcher_1.6.500.v20230717-2134.jar",
  "-configuration", "/opt/jdtls/config_mac_arm",
  "-data", "{{cache_dir}}/workspaces/{{workspace_slug}}",
]
filetypes = [{ pattern = "*.java" }]
initialization_options.settings.java.import.gradle.enabled = true
initialization_options.settings.java.import.maven.enabled = true
initialization_options.workspaceFolders = ["file://{{workspace_root}}"]
```

All of `name`, `command`, `args`, `filetypes`, and `initialization_options` are fields of the same `[[editor.language_servers]]` entry — they are not shared with any other custom server. Two custom entries each have their own `initialization_options`; nothing leaks across rows.

Key observations:

- `{{workspace_slug}}` makes each workspace's `-data` directory unique, which JDTLS requires; without per-workspace substitution, opening a second Java workspace would fail with a lock error.
- The platform-specific `-configuration` path is hardcoded by the user; cross-platform settings sync is out of scope for v1 (see invariant 20's neighborhood — no `{{os}}`/`{{arch}}` placeholders are part of v1's minimum set).
- The launcher jar filename is timestamp-versioned. Upgrading JDTLS requires the user to update the path in `args`. This stays the user's responsibility — Warp will not add JDTLS-specific auto-discovery; v1 of this feature does not grow the built-in server list, and out-of-the-box install support for non-built-in servers is a non-goal (invariant 26).
- `initialization_options.workspaceFolders` hands JDTLS the project root explicitly. JDTLS resolves its project from `rootUri`/`rootPath`/`initializationOptions.workspaceFolders` — **not** from the standard `workspaceFolders` field that invariant 17 sends — so without this line it imports no project and runs in JDTLS's degraded "non-project" mode, where cross-file go-to-definition and hover silently return nothing. `file://{{workspace_root}}` re-supplies (as a `file://` URI) the same root Warp already sends in the standard field, in the form JDTLS reads. Servers like rust-analyzer and gopls read the standard field and need no such duplication.
- The inner `settings` key in `initialization_options.settings.java.import...` is a JDTLS-specific payload convention, not a Warp field. JDTLS reads its Java-language configuration from a nested `settings` object inside the LSP `initializationOptions` payload, which is the shape vscode-java, coc-java, and nvim-jdtls all send. The TOML above produces this JSON on the wire in the `initialize` request:
  ```json
  "initializationOptions": {
    "workspaceFolders": ["file:///abs/path/to/workspace"],
    "settings": {
      "java": { "import": { "gradle": { "enabled": true }, "maven": { "enabled": true } } }
    }
  }
  ```
  Other servers do not use an inner `settings` wrapper — rust-analyzer, for example, reads its `initializationOptions` flat. The shape inside `initialization_options` is defined by each server; Warp does not restructure it. Per invariant 17, string leaves go through placeholder substitution (invariant 5) before the value is sent on the wire; non-string values pass through unchanged.

## Worked examples: overriding a built-in server

A user who wants to override one of the five built-in servers — to change its `args`, point at a different binary, or supply `initialization_options` the built-in does not expose — writes an `[[editor.language_servers]]` entry whose `filetypes` overlap the built-in language. Per invariant 3, the custom entry replaces the built-in for those filetypes; removing the entry restores the built-in.

The five examples below show what an equivalent override looks like for each built-in server. They are not changes Warp ships — Warp continues to launch the built-ins through their existing code paths. The examples exist so a user can copy one as a starting point and modify it. They assume the relevant binary is on `PATH`; a node-wrapped install (`command = "node"`, `args = ["<path>/server.js", "--stdio"]`) is the alternative shape for the Node.js-based servers (pyright, typescript-language-server) and works identically. Each example uses a `my-<binary>` `name` because the binary display names (`rust-analyzer`, `gopls`, `pyright-langserver`, `typescript-language-server`, `clangd`) are reserved per invariant 1; the user can rename it to anything that satisfies the constraints.

### Overriding rust-analyzer

```toml
[[editor.language_servers]]
name = "my-rust-analyzer"
command = "rust-analyzer"
filetypes = [{ pattern = "*.rs", language_id = "rust" }]
```

### Overriding gopls

```toml
[[editor.language_servers]]
name = "my-gopls"
command = "gopls"
filetypes = [{ pattern = "*.go", language_id = "go" }]
```

### Overriding pyright

```toml
[[editor.language_servers]]
name = "my-pyright"
command = "pyright-langserver"
args = ["--stdio"]
filetypes = [{ pattern = "*.py", language_id = "python" }]
```

### Overriding typescript-language-server

```toml
[[editor.language_servers]]
name = "my-typescript-language-server"
command = "typescript-language-server"
args = ["--stdio"]
filetypes = [
  { pattern = "*.ts",  language_id = "typescript" },
  { pattern = "*.tsx", language_id = "typescriptreact" },
  { pattern = "*.js",  language_id = "javascript" },
  { pattern = "*.jsx", language_id = "javascriptreact" },
  { pattern = "*.mjs", language_id = "javascript" },
  { pattern = "*.cjs", language_id = "javascript" },
]
```

### Overriding clangd

```toml
[[editor.language_servers]]
name = "my-clangd"
command = "clangd"
filetypes = [
  { pattern = "*.c",   language_id = "c" },
  { pattern = "*.cc",  language_id = "cpp" },
  { pattern = "*.cpp", language_id = "cpp" },
  { pattern = "*.cxx", language_id = "cpp" },
  { pattern = "*.h",   language_id = "cpp" },
  { pattern = "*.hh",  language_id = "cpp" },
  { pattern = "*.hpp", language_id = "cpp" },
  { pattern = "*.hxx", language_id = "cpp" },
]
```

Notes on these examples:

- Globs match case-insensitively (invariant 1), so `*.c` and `*.h` cover the uppercase variants (`.C`, `.H`) already, no separate entries needed.
- clangd treats `.h` as C++ today (`config.rs:50-54`) because `.h` is genuinely ambiguous; the example above preserves that choice. A user who works in C-heavy codebases can override by editing their entry to map `h` → `c`.
- The TypeScript LSP entry shows the multi-extension/multi-languageId case that motivated the inline-table `filetypes` shape. The same server process handles all six extensions, with each one announced to the server with the correct `languageId`.
- None of the built-in servers need `initialization_options`, `env`, or placeholder substitution to function. Users who want to tune behavior (e.g. `cargo.features = "all"` for rust-analyzer) add fields as needed to their override entry.
