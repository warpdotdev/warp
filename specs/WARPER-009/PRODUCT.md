# WARPER-009: local file-edit corruption port

## Summary

Warper should port upstream repo/file-tool work only when it prevents local file corruption in the OpenRouter editing path. Under the XP bar, the current candidate is the multiline partial-line suffix fix in diff validation. Repo metadata scale, result-size caps, Project Explorer refreshes, watcher load, and Git UI work are deferred until a current Warper workflow fails.

## Why this matters for Warper

OpenRouter exposes `apply_file_diffs`, and the request-file-edits executor applies those diffs to local files. If that path corrupts suffix content around multiline partial lines, it damages user files. That is a survival issue for the local-first agent. Large-repo performance and context fidelity improvements may be useful, but they are not implementation work until backed by a failing acceptance test or current user pain.

## What goes wrong without this

1. An OpenRouter model can propose an `apply_file_diffs` edit whose search block ends with a partial final line. This happens when the model includes enough trailing context to identify the target line but stops before the full line content, for example searching for `// socket_path:` while the actual file line is `// socket_path: ~/.warp[-channel]/remote-server/server.sock`.
2. The diff matcher works on whole-line windows, so the partial final search line can match the entire file line. At that point Warper knows the target line contains an unmatched suffix that was not part of the model's search text.
3. Current Warper only preserves that unmatched final-line suffix when the search block and replacement block have the same number of lines. The upstream regression is the case where the replacement removes one or more earlier lines while keeping the same partial final context line. That is exactly the kind of cleanup edit a model emits when deleting comments, blank lines, or obsolete setup while retaining the next meaningful line.
4. In the broken case, Warper builds a replacement delta that covers the whole matched final file line but inserts only the model's partial replacement line. The untouched suffix is dropped from the insertion.
5. The request-file-edits path then applies that delta to disk through Warper's normal local edit flow. The edit can look valid in the agent conversation because the search matched and the replacement was accepted; the lost suffix is not a failed match, it is silent data loss inside a successful match.
6. The user-visible damage is a local file that compiles or behaves differently for reasons unrelated to the requested edit. A path, argument, expression tail, comment detail, string suffix, or closing syntax that lived after the partial context can disappear.
7. This is worse than rejecting a diff. A rejected diff leaves the file untouched and lets the user or agent retry. A successful corrupt diff changes the user's working tree and can be mistaken for intentional model output during review.
8. The failure is not limited to generated code. Any retained OpenRouter file edit can hit it: Rust source, shell scripts, config files, markdown, TOML, JSON, or project docs. The trigger is the shape of the search/replace block, not the file type.

## Source commits

| Commit | Upstream why | Current Warper evidence | Resolution |
| --- | --- | --- | --- |
| `a1b76c28` | PR `#9623` fixes multiline partial-line suffix preservation in diff validation. | `app/src/ai/agent/api/openrouter.rs:529-530` exposes `apply_file_diffs`; `app/src/ai/blocklist/action_model/execute/request_file_edits.rs:270-271` applies diffs. | Port. |

## Behavior

1. Applying a diff to a file with multiline partial-line content preserves untouched suffix text exactly.
2. Failed or invalid diff validation must fail closed rather than applying a corrupt edit.
3. The port must not import GitHub PR workflow, Git chip UI behavior, remote SSH Project Explorer state, hosted code indexing, or broad repo metadata rewrites.

## Deferred Repo/File Rows

| Commits | Reason |
| --- | --- |
| `802a881e`, `89f61b63`, `48331870`, `5fa22831`, `9f459842`, `43828a6d`, `03ad9ea9`, `e8024b5a`, `0f97ef18`, `3497d184`, `21e70d56`, `5d8507e4`, `bd7202f3` | Useful upstream fixes, but not current Warper survival work without a failing OpenRouter workflow. |
| `5bee7a75`, `59e802ea`, `2fe9d43c`, `1175e82f`, `ffe93a5e`, `1d2775ac`, `cb4fe42a` | Git UI, branch-chip, PR, or watcher API work outside current XP-critical scope. |

## Validation

- Add a regression test for multiline partial-line suffix preservation in diff validation.
- Run the local file-edit/OpenRouter test path that applies a diff to disk.
