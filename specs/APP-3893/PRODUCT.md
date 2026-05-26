# File-based global rules — Product Spec
Linear: APP-3893
Figma: none provided

## Summary
Let users define agent rules that apply across every project by dropping a Markdown file at a well-known location under `$HOME` (initially `~/.agents/AGENTS.md`). Warp picks the file up automatically, sends its contents with every agent query, and surfaces it in the existing Rules settings view alongside cloud rules.

## Problem
Today, rules that should apply to every project must be copied into a `WARP.md`/`AGENTS.md` inside each repo. Cloud rules (added via Settings → Rules) work across projects but require an account, an in-app editor, and aren't editable from external tools or version control. Users want a file-based path: drop a single Markdown file under their home directory, edit it with any tool, and have it applied to every Warp agent query.

## Non-goals
- A unified "disable all rules" toggle that covers file-based globals (today only cloud rules respect the `MemoryEnabled` setting).
- Editing the file from within the Rules view — clicking the row opens it in the editor; we never write to it.
- A separate Add button for file-based globals; the user creates the file themselves.
- New proto/server changes — globals reuse the existing `ProjectRules` request context entry.

## Behavior

### File detection and indexing

1. On startup, Warp checks for a global rule file at each registered location. The initial registry has one entry: `~/.agents/AGENTS.md`. Adding a new well-known location is a code-side change.

2. If the file exists at the registered path when Warp launches, its contents are read into memory and used as agent context for subsequent queries. The read is asynchronous; it does not block startup.

3. If the file does not exist at startup, Warp watches the parent directory (`~/.agents`) so the rule is picked up when the file is created later in the session. No restart is required.

4. If the user deletes the file, Warp drops it from memory within the directory-watcher's debounce window (~500 ms). Subsequent agent queries no longer include the deleted rule. The same applies if the file becomes unreadable for any other reason between FS events (e.g. the user revokes read permission, or replaces the file with a directory) — Warp must not keep stale rule contents active once the source goes away.

5. If the user edits the file, Warp re-reads its contents on the next directory-watcher tick. The next agent query reflects the new contents.

6. If the registered home subdir (e.g. `~/.agents`) does not exist at all, Warp does nothing visible. Once the user `mkdir`s that subdir and creates the rule file inside, Warp registers a watcher and picks the rule up — no restart required.

7. If the user's home directory cannot be determined (a degenerate environment), global-rule indexing silently does nothing — the agent continues to work with project rules only.

### Agent context

8. When an agent query is sent, the contents of every present global rule file are included in the request, alongside any project-scoped rules discovered for the working directory. The agent treats both layers as "rules to follow."

9. Precedence is `global > project WARP.md > project AGENTS.md`. Within a single project directory, an existing `WARP.md` continues to shadow a sibling `AGENTS.md`; the global rule is appended in addition to whichever project rule wins.

10. If no project rules exist for the working directory and no global rule file is present, agent queries behave exactly as they did before this feature — no rule context is attached.

11. If the user is offline or unauthenticated, file-based global rules still work. They are read locally and shipped with the request body; no server-side rule store is consulted.

12. The very first query immediately after launch may fire before the global rule has finished loading (the read is backgrounded). The next query will include the rule. This is the same race the existing project-rules indexing accepts.

13. "Is this project initialized?" surfaces — the `/init` flow's pending-rules step and the code-review empty state's "Repo is initialized with a {file_name} file." hint — answer based on **project-level** rule files only. Dropping a `~/.agents/AGENTS.md` does not by itself mark every repo as initialized; those surfaces still want to set up `WARP.md` / `AGENTS.md` *inside* the repo. Globals are still applied to agent queries; they just do not impersonate per-project initialization.

### Settings → Rules surface

14. In Settings → AI → Rules → **Global** tab, every detected file-based global rule appears as its own row alongside cloud rules. The row shows the absolute file path and an "Open file" button.

15. Clicking "Open file" opens the file in the configured editor. Warp never writes to the file from this surface.

16. When the file is created, edited, or deleted, the row appears, persists, or disappears in the Global tab live — without restarting Warp or re-opening Settings.

17. The Global tab's search bar matches both cloud-rule names/contents and file-based global paths. Search results from both sources are mixed in the result list.

18. The Global tab's "Add" button continues to create a cloud rule. There is no separate "Add" affordance for file-based globals; the user creates the file directly.

19. When neither cloud rules nor file-based globals exist, the Global tab's zero-state copy mentions both ways to add a rule, including dropping a file at `~/.agents/AGENTS.md`.

20. The "rules disabled" banner — shown when `Settings → AI → Memory` is off — continues to render on the Global tab. **Open question:** that toggle today only governs cloud rules; file-based rules currently bypass it. We accept the asymmetry for now and may revisit in a follow-up.

21. The Project-based tab is unchanged.
