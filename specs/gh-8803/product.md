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
   - `name` (string, required) — A unique identifier for this server within the user's settings, e.g. `"ruby-lsp"`. Used in UI surfaces (e.g. the footer "Enable {name}" button) and log output. The per-server cache directory's path component is derived from a hash of `name` rather than from `name` directly (see invariant 5's `{{workspace_cache_dir}}`), which decouples the cache-dir path from any platform-specific path-segment constraints. **Constraints:** 1–64 characters, drawn from `[A-Za-z0-9._-]` (ASCII letters, digits, dot, underscore, hyphen). Must not be `.` or `..`, must not start with `.` or `-`, and must not be empty. Uniqueness is **case-insensitive** (ASCII fold). A custom entry **may** reuse a built-in server's name (e.g. `rust-analyzer`) — there are no reserved names. Reusing a built-in's display name is itself an override: a custom entry whose `name` case-insensitively matches a built-in server's display name **replaces that built-in entirely** (see invariant 3). Because the custom always wins that conflict, one workspace never surfaces a duplicate "Enable {name}" affordance. Names violating the character/length constraints are settings errors per invariant 23.
   - `command` (string, required) — Path to the server binary. Must be **either** an absolute path (after `~`/`~/` home expansion per invariant 5) **or** a bare name with no path separators (`/` or `\`), which will be resolved against the user's `PATH`. "Absolute" is platform-specific: on Unix, starts with `/`. On Windows, starts with a drive letter followed by `\` or `/` (e.g. `C:\bin\server`, `C:/bin/server`) **or** is a UNC path (`\\server\share\path` or `//server/share/path`). Windows root-relative paths (`\path`, `/path` without a drive) are **not** considered absolute — they depend on the current drive at exec time, which is exactly the cwd-dependent resolution this rule is preventing. **Relative paths containing separators (e.g. `./server`, `bin/server`, `..\\server`, Windows `\server`) are rejected** as settings errors per invariant 23 — these forms would otherwise resolve directly against the spawned process's cwd (the workspace root, per invariant 16). For the PATH-based residual case (`$PATH` containing relative entries), see invariant 16.

     `command` is substitutable (invariant 5), but this rule is checked on the **literal** value, *before* substitution. So placeholders are usable in `command` in exactly two shapes: as the **whole value** — a single bare token like `command = "{{env_LSP_BIN}}"`, which has no separators and so passes as a bare name, then expands to any path — or **within an absolute path**, like `command = "/opt/{{env_TOOLCHAIN}}/bin/lsp"`, which starts with `/` and so passes as absolute. A value that is *neither* absolute nor a bare token — a placeholder-led relative path such as `{{workspace_root}}/bin/lsp` — is **rejected**, the same cwd-safety guard that rejects `./server`: resolving the binary relative to the opened (possibly untrusted) workspace is precisely what this rule prevents. Use an absolute form or a whole-value placeholder instead.
   - `args` (array of strings, optional, defaults to `[]`) — Arguments passed to `command` on launch.
   - `filetypes` (array of strings, required, non-empty) — Patterns that claim files for this server, e.g. `["*.rb", "*.rake", "Gemfile"]`. The LSP `languageId` Warp sends for a matched file is derived automatically (see invariant 33) — there is no per-entry language-id field — from Warp's built-in filetype → language-id registry when the extension or basename is known, otherwise the file's lowercase extension, otherwise its literal basename. Each entry is a string in one of two syntactic forms:
     - **Glob** — contains any of `*`, `?`, or `[` (e.g. `"*.rb"`, `"*.rake"`, `"Dockerfile.*"`). Matched against the file's basename only (not the full path) using POSIX-style glob semantics — the syntax accepted by Rust's [`glob` crate `Pattern`](https://docs.rs/glob/latest/glob/struct.Pattern.html), which is a strict subset of POSIX.1-2017 §2.13 Pattern Matching Notation. Supported metacharacters are `*` (any sequence of characters except path separators), `?` (any single character), `[abc]` / `[!abc]` (character class / negated class), and `[a-z]` (ranges). Glob matching is case-insensitive — `"*.rb"` matches both `foo.rb` and `FOO.RB`. Brace alternation (`{a,b}`) and double-star recursion (`**`) are **not** supported in v1, since matching is basename-only.
     - **Literal basename** — any pattern that contains none of `*`, `?`, or `[` (e.g. `"Gemfile"`, `"Rakefile"`, `".bashrc"`). Matches files whose basename equals it, case-insensitively — all filetype matching is case-insensitive, the model VS Code uses. To match files by extension, write a glob (`"*.rb"`, `"*.ts"`); a bare token like `"rb"` is **not** treated as an extension match — it is a literal basename match against a file literally named `rb`.
   - `env` (table of string → string, optional, defaults to `{}`) — Extra environment variables merged into the server process's environment on launch.
   - `initialization_options` (arbitrary TOML value, optional) — Sent as the `initializationOptions` field of the LSP `initialize` request. Warp does not restructure the value's shape; per invariant 5, string leaves go through placeholder substitution before send, while non-string values pass through unchanged.

2. `name` must be unique across all entries in `[[editor.language_servers]]`. Two entries with the same `name` are a settings error; see invariant 23. Uniqueness is **case-insensitive**: `ruby-lsp` and `Ruby-LSP` are duplicates and produce a settings error. This is a UX rule: distinct-but-case-confusable names produce indistinguishable footer labels ("Enable ruby-lsp" vs. "Enable Ruby-LSP") and log entries, and case-insensitive uniqueness eliminates that ambiguity at validation time. Cache-dir collisions are not at issue because the cache-dir path component is hashed (see invariant 5's `{{workspace_cache_dir}}`).

3. Custom server entries always take precedence over built-in servers; on any conflict, the custom wins. Override happens two ways:
   - **By filetype** — when a custom entry's `filetypes` overlap a built-in language, the custom handles those files. For example, an entry with `filetypes = ["*.rs"]` replaces the built-in `rust-analyzer` for `.rs` files. A built-in that serves several filetypes still handles the ones the custom does not claim (e.g. a custom claiming only `*.c` leaves the built-in clangd serving `.cpp`, `.h`, and the rest).
   - **By name** — when a custom entry's `name` case-insensitively matches a built-in server's display name (`rust-analyzer`, `gopls`, `pyright-langserver`, `typescript-language-server`, `clangd`), the custom **replaces that built-in server entirely**: the built-in is suppressed for all of its filetypes, and the custom's `filetypes` define what is served. This is the explicit "I am replacing this server" gesture, so the user is responsible for declaring the filetypes they want covered.

   Removing the custom entry restores the built-in with no further action in either case.

4. When multiple custom entries' `filetypes` patterns could match the same opened file, the last matching entry in source order in the settings file wins — a later entry overrides an earlier one, so appending an entry supersedes a previous definition. Built-in language→server mappings are only consulted if no custom entry matches — and a built-in whose display name is taken by a custom entry (invariant 3, by name) is never consulted at all. Overlap between entries is not a settings error — order is the disambiguator.

### Placeholder substitution

5. The string values of `command`, each entry of `args`, each value of `env`, and every string leaf inside `initialization_options` undergo template substitution at launch time. Non-string values (numbers, booleans, arrays, tables) inside `initialization_options` pass through unchanged; only their string children are substituted. Substitution uses the same `{{name}}` template syntax as Warp's tab configs and MCP server rendering, so the convention is consistent across Warp settings files. The following placeholders are recognized:
   - `{{workspace_root}}` — Absolute path to the resolved workspace root (see invariant 12).
   - `{{workspace_cache_dir}}` — A per-server, **per-workspace** cache directory owned by Warp (under the OS cache dir), unique to the combination of this server's `name` and the resolved workspace root. The user does not compose this path — Warp guarantees it is distinct for each (server, workspace) pair, so opening the same server in two workspaces never collides (this is exactly what servers like Eclipse JDT-LS require of their `-data` directory). The path is built from a deterministic hash of the entry's `name` and a deterministic hash of the workspace root (each a SHA-256 prefix), so it is always a safe path segment on every supported platform regardless of what characters `name` contains, and it is stable across launches — the same `name` in the same workspace always resolves to the same directory. Warp creates the directory before launch. Suitable as a parent for, or directly as, the server's per-workspace scratch/data state.
   - `{{env_VAR}}` — The value of environment variable `VAR` in Warp's process environment at launch time. The `env_` prefix is used because the template parser only accepts alphanumeric characters, `-`, and `_` in placeholder names; `{{env_HOME}}` expands to the value of `$HOME`. An undefined variable expands to the empty string and is logged.

6. Substitution is single-pass within a string: a substituted value containing `{{...}}` syntax is not re-expanded. Unknown placeholders (`{{...}}` patterns that do not match any name above) expand to themselves verbatim and are logged once per launch. Whitespace inside the braces invalidates the placeholder, so `{{ workspace_root }}` is not expanded. A single `{` or `}` is ordinary text. There is no in-Warp escape for the recognized placeholder set — if a user needs to emit a literal string that exactly matches `{{workspace_root}}`, `{{workspace_cache_dir}}`, or `{{env_VAR}}` into the spawned process's args, they must produce it via the consuming tool rather than via the settings file.

   In addition to `{{...}}` placeholders, a leading `~` or `~/` at the start of any substituted string expands to the current user's home directory. `~` is expanded only at the very beginning of a value; embedded `~` characters (e.g. `/opt/~/bin`) are passed through unchanged. Other-user home expansion (`~someuser/...`) is not supported. `~` expansion is needed because Warp spawns the server with a direct OS `exec`, not through a shell — without it, `command = "~/bin/lsp-server"` would fail with "no such file or directory."

7. Substitution applies before the process is spawned, after settings validation. A custom entry whose post-substitution `command` resolves to a non-existent path follows the same error path as any other failed launch (see invariant 18).

8. Reordering, adding, or removing entries in the settings file takes effect on the next file open for that filetype. Already-running servers are not restarted by an edit to their entry; users can stop and reopen a file to pick up the new config (see invariant 19).

### Opening a file

9. When the user opens a file in the Warp editor, server resolution proceeds as:
   - If the file matches a built-in language and no custom entry overrides it, behavior is unchanged from today.
   - If the file matches exactly one custom entry's `filetypes` (or matches multiple, with the last-in-source-order entry winning per invariant 4), that custom server is the candidate for this file.
   - If the file matches no entry (built-in or custom), the footer surfaces the same "Language support is unavailable for this file type" state it shows today. No new footer affordance, link, or affordance text is introduced by this feature.

10. The footer's visible behavior, copy, and interaction model are unchanged from today. Custom servers participate in every existing footer surface — status indicator, install progress, the per-workspace Enable button, error messages — via the same code paths built-in servers use. The only difference is that the server's display name and status come from a custom entry instead of a built-in `LSPServerType`.

### Enabling a server per workspace

11. The first time a server (built-in or custom) is a candidate for a workspace root, the footer surfaces the existing "Language support is not currently enabled for `<codebase>`" affordance with its Enable button, unchanged from today. Accepting attaches and persists per-workspace state exactly as it does today for built-ins; custom servers reuse the same persistence and Enable flow.

12. The workspace root used for the enable prompt is the directory currently open in the Warp window — the same root used by built-in servers today.

13. Accepting the prompt persists the choice: that server is automatically enabled for that workspace root on subsequent file opens in the same session and across restarts.

14. There is no decline action on the prompt — it is a passive affordance, matching today's built-in behavior. Not accepting it leaves the server un-enabled for the session only (nothing is persisted); the affordance remains available whenever a matching file is open in that workspace, and reappears after restart. Separately, a user can **explicitly disable** an enabled server via the footer status dropdown or the settings code page, exactly as for built-ins today: that choice persists across restarts and is reversed through those same surfaces.

15. If the user has multiple Warp windows open on the same workspace root, enabling or disabling a server in one window applies to all of them. A single server process is shared across windows on the same root, consistent with today's built-in behavior.

### Server lifecycle

16. When a server is enabled for a workspace and a matching file is opened, Warp launches the post-substitution `command` with the post-substitution `args`, the merged environment from Warp's process environment — with `PATH` set to the user's interactive shell `PATH`, captured the same way built-in servers do today — and the entry's `env` (also post-substitution), and the working directory set to the resolved workspace root. Environment variables exported only in shell rc files (other than `PATH`) are not part of the spawn environment when Warp is launched from the GUI; use the entry's `env` (or `{{env_VAR}}` where Warp's own environment carries the value) to supply them. The server is launched once per workspace root; subsequent file opens reuse it.

    **PATH-based residual risk.** When `command` is a bare name, resolution against `$PATH` is performed by the OS process loader (via `Command::new(...)`, which on Unix uses `execvp` and on Windows uses `CreateProcess` wrapped in `cmd.exe /c` so `.cmd`/`.bat` scripts resolve correctly). If the user's `$PATH` contains relative entries (`.`, `..`, `bin`, or empty entries from a stray `::`), those entries are evaluated against the spawned process's cwd at exec time — the workspace root — so a workspace-controlled binary could still satisfy a bare command. This is the **same PATH-resolution behavior used by Warp's built-in language servers today** (`crates/lsp/src/command_builder.rs`, `crates/lsp/src/config.rs::command_and_params`): Warp does not sanitize `$PATH` at LSP spawn for either built-in or custom servers. Users who do not trust the contents of the workspaces they open should set `command` to an absolute path; that bypasses PATH resolution entirely. A future Warp-wide change to `CommandBuilder` could sanitize relative `$PATH` entries for all LSP spawns, but is out of scope for this spec.

    **The post-substitution `command` is not revalidated against the literal-form absolute-or-bare-name rule.** Placeholder substitution (invariant 5) can produce any string the user's environment dictates — including paths the literal-time validator would have rejected, such as a `$LSP=./server` env value substituted into `command = "{{env_LSP}}"`. The user is responsible for the contents of their environment, just as they are for the literal text of `settings.toml`. This matches how Warp treats user-controlled inputs everywhere else: `$PATH` for built-in LSPs is not sanitized, the MCP-server `command` field is not revalidated after env interpolation, and the terminal runs literally what the user types. The literal-text validator's job is to catch obvious settings-file errors (a hand-typed `./server`), not to defend against the user's own environment.

17. The LSP `initialize` request sends the resolved workspace root as a `workspaceFolders` entry and passes the **substituted** `initialization_options` (per invariant 5 — string leaves go through placeholder substitution; non-string values pass through unchanged) when provided. Warp does not restructure the value's shape.

    `initialization_options` is the **only** configuration channel in v1. Warp does not advertise the `workspace/configuration` client capability and does not send `workspace/didChangeConfiguration`, so custom servers must accept all of their configuration at initialize time; anything a server would normally pull via `workspace/configuration` falls back to that server's own defaults. This matches how Warp's built-in servers are configured today.

18. If `command` cannot be found on `PATH` (and is not an absolute path), or if the launch fails (non-zero exit before initialization, missing executable bit, etc.), the failure surfaces through the existing footer error path — the same inline error rendering used today for built-in server failures, with the server's `name` and a one-line description of the failure. The editor continues to function without LSP support for that file.

19. Editing or removing an `[[editor.language_servers]]` entry in the settings file does not affect an already-running server for that entry — neither restarting it with new values, nor stopping it on removal. The running server keeps reflecting the configuration from its most recent launch, and continues to serve in-flight requests from open files. New values (or the absence of the entry) take effect only on the next launch, which the user triggers by closing the workspace's editor panes for that filetype and reopening a file, or via an explicit restart action (out of scope to design here; the requirement is that subsequent launches honor the current settings).

### Filetype matching details

20. Filetype matching uses the two forms defined in invariant 1's `filetypes` field: shell-style glob against the basename, and literal basename match. Both forms match **case-insensitively** — the same model VS Code's file associations use. The file's basename is computed from the opened file's path; no other metadata is consulted.

21. Content sniffing (e.g., inspecting a shebang line, parsing file contents to detect language) is out of scope. A bash script named `deploy` with `#!/usr/bin/env bash` at the top is only claimed by a custom entry if `"deploy"` appears in some entry's `filetypes`, or a glob like `"deploy*"` matches. Users who want arbitrary extensionless shell scripts to be claimed must enumerate them.

22. A file whose extension or basename is claimed by a custom entry but whose contents look like a different language (e.g. a `.ts` file that's actually JSON) is still routed by the matched entry. Content sniffing is out of scope per invariant 21.

### Settings validation and errors

23. The following are settings errors and surfaced on settings load:
   - Duplicate `name` across entries.
   - An entry with empty `filetypes`.
   - An entry missing `name` or `command`.
   - An entry whose `name` violates the constraints in invariant 1.
   - An entry whose `command`, after `~`/`~/` home-directory expansion, is neither absolute nor a bare name (per invariant 1's `command` rule — Unix `/`-rooted, Windows drive-letter or UNC; root-relative `\path` rejected).
   - A `filetypes` entry that is not a non-empty string.
   - A `filetypes` glob entry that fails to compile as a valid shell-style glob.

   When any entry is invalid, the entire `[[editor.language_servers]]` setting fails to load — no **new** custom servers are launched until the file is fixed (already-running servers keep running per invariant 19) — and the existing settings-error banner surfaces `editor.language_servers` as an invalid value. This matches how every other array-valued setting in Warp behaves (e.g. `agents.profiles.agent_mode_command_execution_allowlist`). Per-entry reasons (which entry, which field) are written to the log so users can investigate when the banner alone isn't enough. The settings file itself is not auto-edited.

24. Unknown fields on an `[[editor.language_servers]]` entry are ignored with a warning logged but no in-app notification. This leaves room to add fields without breaking existing settings files.

25. Warp generates a JSON Schema for the `[[editor.language_servers]]` array as part of the existing build-time settings schema artifact. The schema is consumed by **external editors that support TOML schema validation** (e.g. editing `settings.toml` in another editor via a TOML language server) and by Warp's in-app docs page. The schema describes every field above with descriptions and required/optional markers, and enumerates the recognized `{{...}}` placeholders (`{{workspace_root}}`, `{{workspace_cache_dir}}`, `{{env_VAR}}`) and the leading-`~`/`~/` home-directory expansion. Warp's in-app `settings.toml` text view is not schema-aware in v1; schema-driven autocomplete inside Warp is a separate follow-up.

### Non-goals

26. v1 does not install language server binaries on the user's behalf. Users are responsible for making `command` resolvable. Install instructions may be documented on the public docs page that accompanies this feature (see invariant 25's external-tooling JSON Schema, which the docs page builds on), but Warp itself does not run package managers, and per invariant 9 no new footer link, banner, or affordance is introduced.

27. v1 does not support per-workspace `.warp/settings.toml` overrides of `[[editor.language_servers]]`. Custom servers are defined at the user level only. Per-workspace overrides are a likely follow-up but not part of this feature.

28. v1 does not support multiple language servers attached to the same file simultaneously (e.g. a linter LSP and a navigation LSP on the same `.rb` file). Neovim allows this; Warp's existing one-server-per-language model is preserved for v1.

29. v1 only supports the stdio transport. The server is launched as a child process and communicates over its stdin/stdout. TCP, named pipes, and other LSP transports are out of scope.

30. v1 does not support merging a custom entry's `initialization_options` or other fields into a built-in server's configuration. A custom entry whose `filetypes` overlap a built-in language fully replaces the built-in server for those filetypes (see invariant 3). Users who want to tune a built-in server's behavior must define a complete custom entry that supplies its own `command`, `args`, and other fields.

31. v1 does not ship a dedicated inspection or management surface (command-palette action, settings sub-page, or status-dropdown extension) for listing which servers are configured and running, resetting per-workspace enable/disable state, or restarting a server for a workspace root. The footer's existing per-workspace Enable button and status dropdown carry over from today's built-in flow and apply to custom servers too; a richer inspection/management surface is deferred to a future release.

### Logging and redaction

32. **Log redaction.** When Warp logs custom-server launch information (substituted `command`, `args`, `env` values, and string leaves of `initialization_options`), values that match Warp's existing secret-redaction patterns (`app/src/settings/privacy.rs::CustomSecretRegex`) are redacted before being written. This applies to both the substituted output and the raw descriptor values, and applies at every log level. Users who reference `{{env_VAR}}` placeholders get the same redaction protection as elsewhere in Warp; raw `env` *values* in `settings.toml` are also redacted on launch logging.

   The following are **logged verbatim** (not passed through secret redaction): descriptor `name`, `env` *keys*, the resolved `workspace_root` absolute path, and the per-server, per-workspace `workspace_cache_dir` absolute path. `workspace_root` and `workspace_cache_dir` are absolute filesystem paths and may contain usernames, company names, or private repository names — for example, `/Users/alice/work/acme-internal-repo` — and Warp does **not** treat these as secret-redaction targets. This matches Warp's existing log behavior for built-in language servers (`crates/lsp/src/config.rs::command_and_params` and the LSP log filenames in `app/src/code/lsp_logs.rs`, both of which include workspace-derived paths verbatim today). `CustomSecretRegex` matches token-shaped secrets, not arbitrary PII; users who consider workspace paths sensitive in their environment must filter or redact their log files before sharing them, just as they would for any other Warp log output.

### Built-in filetype → language-id registry

33. Warp ships a **shared filetype → language registry**: a mapping from file extension (and literal basenames like `Dockerfile`, `Makefile`) to a language identity. The facet this feature reads is the LSP `languageId`, using the language identifier set VS Code defines. The registry is Warp-wide, not LSP-private (see Evolution), and is **independent of the five built-in servers** — it is data, not server dispatch.

    **Coverage commitment.** At release, the registry's associations are at parity with the union of:
    - **VS Code's** built-in language contributions — every known language identifier in VS Code's core set, with its default extension and filename associations; and
    - **neovim's** built-in filetype detection tables for **extensions and literal filenames** (`vim.filetype`'s `extension` and `filename` tables). Neovim's content-based (first-line/shebang) and path-pattern detection are out of scope, consistent with invariants 20–21.

    For a language neovim recognizes but VS Code's core set does not, the id is the one that language's dominant VS Code extension registers (e.g. `zig`, `elixir`, `gleam`). A parity test enumerates both sources' association sets and asserts the registry is a superset, so coverage cannot silently regress (see tech.md).

    **Resolution.** The registry is the **sole** source of the `languageId` Warp sends for a file matched by a custom server: the file's lowercased extension (or literal basename) is looked up in the registry; on a hit that id is used, otherwise Warp falls back to the file's lowercase extension, then its literal basename. A registry miss never blocks a server from attaching and is never a settings error — the fallback id is sent, which single-language servers ignore. A custom server has **no** per-entry `language_id` field — the id always comes from the registry.

    **Evolution.** Expanding the registry is a **data-only change**: it does not require adding a built-in server or touching the custom-server pipeline, and it is the single place a missing or wrong `languageId` is fixed. Both custom servers and Warp's five built-in servers resolve their `languageId` through this one registry — there is no separate built-in mapping to keep in sync. The registry is Warp's shared home for filetype→language associations: the editor's syntax-highlighting grammar selection also moves onto it in this feature (an internal refactor with no user-visible change — the same files highlight the same way), and other filetype consumers such as file icons and language display names are candidates for later migration; the migration scope lives in tech.md. The resolver is designed to admit first-line/shebang detection in the future (out of scope now, consistent with invariants 20–21) without changing the custom-server surface. The registry is **not user-extensible in v1**: no settings surface adds or overrides mappings. Per-file-type configuration (overriding the id for one pattern, or other per-pattern options) is intentionally out of scope for this feature; if Warp adds it later, it will be a separate, cross-feature configuration mechanism rather than a field on `[[editor.language_servers]]`.

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
  "-data", "{{workspace_cache_dir}}",
]
filetypes = ["*.java"]
initialization_options.settings.java.import.gradle.enabled = true
initialization_options.settings.java.import.maven.enabled = true
```

All of `name`, `command`, `args`, `filetypes`, and `initialization_options` are fields of the same `[[editor.language_servers]]` entry — they are not shared with any other custom server. Two custom entries each have their own `initialization_options`; nothing leaks across rows.

Key observations:

- `{{workspace_cache_dir}}` is already unique per (server, workspace), which is exactly what JDTLS's `-data` directory requires — opening a second Java workspace gets its own data directory with no extra configuration, so there is no lock conflict.
- The platform-specific `-configuration` path is hardcoded by the user; cross-platform settings sync is out of scope for v1 (see invariant 20's neighborhood — no `{{os}}`/`{{arch}}` placeholders are part of v1's minimum set).
- The launcher jar filename is timestamp-versioned. Upgrading JDTLS requires the user to update the path in `args`. This stays the user's responsibility — Warp will not add JDTLS-specific auto-discovery; v1 of this feature does not grow the built-in server list, and out-of-the-box install support for non-built-in servers is a non-goal (invariant 26).
- The inner `settings` key in `initialization_options.settings.java.import...` is a JDTLS-specific payload convention, not a Warp field. JDTLS reads its Java-language configuration from a nested `settings` object inside the LSP `initializationOptions` payload, which is the shape vscode-java, coc-java, and nvim-jdtls all send. The TOML above produces this JSON on the wire in the `initialize` request:
  ```json
  "initializationOptions": {
    "settings": {
      "java": { "import": { "gradle": { "enabled": true }, "maven": { "enabled": true } } }
    }
  }
  ```
  Other servers do not use an inner `settings` wrapper — rust-analyzer, for example, reads its `initializationOptions` flat. The shape inside `initialization_options` is defined by each server; Warp does not restructure it. Per invariant 17, string leaves go through placeholder substitution (invariant 5) before the value is sent on the wire; non-string values pass through unchanged. This example works under invariant 17's initialize-only configuration channel because JDTLS accepts its full configuration in this payload at startup; configuration it would otherwise refresh at runtime (vscode-java delivers updates via `workspace/didChangeConfiguration`) stays at its launch-time values until the server is relaunched (invariant 19).

## Worked examples: overriding a built-in server

A user who wants to override one of the five built-in servers — to change its `args`, point at a different binary, or supply `initialization_options` the built-in does not expose — writes an `[[editor.language_servers]]` entry that reuses the built-in's name. Per invariant 3 (by name), the custom entry replaces that built-in entirely; removing the entry restores the built-in.

The five examples below show what an equivalent override looks like for each built-in server. They are not changes Warp ships — Warp continues to launch the built-ins through their existing code paths. The examples exist so a user can copy one as a starting point and modify it. They assume the relevant binary is on `PATH`; a node-wrapped install (`command = "node"`, `args = ["<path>/server.js", "--stdio"]`) is the alternative shape for the Node.js-based servers (pyright, typescript-language-server) and works identically. Each example gives the entry the **same `name` as the built-in** so it replaces that built-in entirely (invariant 3, by name) — names are not reserved (invariant 1), and the entry's `filetypes` define what the replacement serves. The `languageId` Warp sends for each extension comes from its built-in filetype → language-id registry (invariant 33); there is no per-entry `language_id` field.

### Overriding rust-analyzer

```toml
[[editor.language_servers]]
name = "rust-analyzer"
command = "rust-analyzer"
filetypes = ["*.rs"]
```

### Overriding gopls

```toml
[[editor.language_servers]]
name = "gopls"
command = "gopls"
filetypes = ["*.go"]
```

### Overriding pyright

```toml
[[editor.language_servers]]
name = "pyright-langserver"
command = "pyright-langserver"
args = ["--stdio"]
filetypes = ["*.py"]
```

### Overriding typescript-language-server

```toml
[[editor.language_servers]]
name = "typescript-language-server"
command = "typescript-language-server"
args = ["--stdio"]
filetypes = ["*.ts", "*.tsx", "*.js", "*.jsx", "*.mjs", "*.cjs"]
```

### Overriding clangd

```toml
[[editor.language_servers]]
name = "clangd"
command = "clangd"
filetypes = ["*.c", "*.cc", "*.cpp", "*.cxx", "*.h", "*.hh", "*.hpp", "*.hxx"]
```

Notes on these examples:

- **Pyright's entry `name` is `pyright-langserver`, not `pyright`.** To override a built-in *by name*, the entry's `name` must match that built-in's display name exactly (invariant 3, by name), and a built-in's display name is its server **binary** name — the string the footer's "Enable {name}" button already shows. For the other four, the product name and the binary name are the same (`rust-analyzer`, `gopls`, `clangd`, `typescript-language-server`), so this distinction is invisible. Pyright is the exception: the product is called "pyright," but its language-server executable is `pyright-langserver`, so that binary name is what Warp's built-in uses and what a custom entry must reuse to replace it *by name*. (Since this example claims `*.py` — pyright's only filetype — an entry named anything else with `filetypes = ["*.py"]` would still take over Python by *filetype* override; reusing the binary name is what makes it a by-*name* replacement and keeps the footer's "Enable" label identical to the built-in's.) Pyright also needs `args = ["--stdio"]` to speak LSP over stdin/stdout, the same as `typescript-language-server`; `rust-analyzer`, `gopls`, and `clangd` default to stdio and need no such flag.
- Globs match case-insensitively (invariant 1), so `*.c` and `*.h` cover the uppercase variants (`.C`, `.H`) already, no separate entries needed.
- The built-in filetype → language-id registry (invariant 33) resolves each extension automatically. `.h` is genuinely ambiguous (C vs. C++); the registry picks one default for it, and changing how `.h` is classified is a data-only change to the registry (invariant 33), applied uniformly for every custom server (including a custom overriding built-in clangd, as here) — there is no per-entry override.
- The TypeScript LSP entry shows a single server process handling six extensions; the registry announces each one to the server with the correct `languageId` (`typescript`, `typescriptreact`, `javascript`, `javascriptreact`) automatically.
- None of the built-in servers need `initialization_options`, `env`, or placeholder substitution to function. Users who want to tune behavior (e.g. `cargo.features = "all"` for rust-analyzer) add fields as needed to their override entry.
