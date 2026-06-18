# PRODUCT.md — CODE-1779: Drag-and-drop file paths in WSL (and Git Bash)

Linear: https://linear.app/warpdotdev/issue/CODE-1779/windows-drag-and-drop-file-paths-in-wsl
Upstream: https://github.com/warpdotdev/warp/issues/6191

Figma: none provided (no visual design — the change is purely in which string lands in the input buffer)

## Summary
When a Warp tab is attached to a Unix-like shell on Windows — WSL, or MSYS2 / Git Bash — dragging a file or folder from Windows Explorer onto Warp should insert a path in that shell's native form, not a Windows-native path. For WSL that's `/mnt/c/Users/andy/Downloads`; for Git Bash that's `/c/Users/andy/Downloads`. WSL already works correctly on the terminal grid when a long-running command is active; this spec covers the input editor (broken for both WSL and Git Bash today) and adds matching behavior for Git Bash.

## Behavior

1. Dropping one or more files or folders from Windows Explorer onto the Warp input editor inserts each path in the active session's native form:
   - **WSL session** — drive-letter paths are mapped under `/mnt/<drive>/…` with forward slashes.
   - **MSYS2 / Git Bash session** — drive-letter paths are mapped under `/<drive>/…` with forward slashes (MSYS2's POSIX-style path convention, which Git Bash and the MSYS2 runtime translate automatically when invoking native Windows binaries).
   - **All other sessions** — paths are inserted exactly as dropped (see invariant 5).

2. Conversion rules for a single dropped path:
   - WSL session:
     - `C:\Users\andy\file.txt` → `/mnt/c/Users/andy/file.txt`.
     - `D:\Pictures\Screenshot 2025-05-14 155816.png` → `/mnt/d/Pictures/Screenshot 2025-05-14 155816.png` (spaces preserved; shell escaping then applies on top).
     - `C:\` and `C:` both → `/mnt/c`.
     - Uppercase drive letters are lowercased (`E:\foo` → `/mnt/e/foo`).
     - UNC-style paths with no drive letter (`\\server\share\file`) have backslashes converted to forward slashes (`//server/share/file`); no further remapping is attempted.
   - MSYS2 / Git Bash session:
     - `C:\Users\andy\file.txt` → `/c/Users/andy/file.txt`.
     - `D:\Pictures\Screenshot 2025-05-14 155816.png` → `/d/Pictures/Screenshot 2025-05-14 155816.png`.
     - `C:\` and `C:` both → `/c`.
     - Uppercase drive letters are lowercased (`E:\foo` → `/e/foo`).
     - UNC-style paths with no drive letter (`\\server\share\file`) have backslashes converted to forward slashes (`//server/share/file`); no further remapping is attempted.

3. Dropping multiple paths in a single drop inserts each one individually, separated by a single space, with each path transformed per (2). A trailing space is appended to the inserted text so back-to-back drops don't concatenate tokens (unchanged from today).

4. Shell-specific escaping (quoting spaces, special characters) applies on top of the transformed path using the active session's shell family — identical to the non-WSL/non-MSYS2 behavior today.

5. When the active session is neither WSL nor MSYS2/Git Bash (local PowerShell, cmd, SSH into a remote host, Warpified remote, etc.), dropped paths are inserted exactly as they are today. No transformation happens.

6. Image auto-attachment (dragging an image file into Agent Mode / an empty buffer, which attaches it as AI image context) continues to use the original Windows-native path for filesystem reads, regardless of WSL / MSYS2 state. Transformed paths would not be readable from the Windows host.

7. If an image auto-attach fails (e.g. per-query or per-conversation image limit is exceeded) and the path is inserted as text as a fallback, that inserted text is the transformed path (per invariant 2) when the session is WSL or MSYS2 — matching invariant (1).

8. Parity with the terminal grid:
   - WSL: dropping onto the grid already inserts WSL-style paths during a long-running command. The input editor must match, so the user sees the same text regardless of whether the drop target was the grid or the input editor.
   - MSYS2 / Git Bash: the grid's long-running behavior (drop a path, get the Windows-native path written to the PTY without shell escaping, so native Windows executables receive the form they expect) is **intentional and unchanged** by this ticket. The input editor is a different context — the shell itself processes the text next, so MSYS2-style paths are the right default there. Grid and input-editor behavior therefore differ on MSYS2 by design; this is noted so it isn't flagged as a bug in review.

9. The transformation is driven by the *currently active block's session*. Switching blocks (and therefore sessions) between drops updates which rule applies on the next drop.

10. Non-regressions:
    - Dropping into any non-terminal editor (notebooks, settings, themes, etc.) is unchanged — no path transformation is applied.
    - Dropping into a non-WSL, non-MSYS2 terminal session (PowerShell, cmd, SSH, remote Warpified) is unchanged.
    - The terminal-grid long-running-command code paths for both WSL and MSYS2 are unchanged.
    - Dropping image-only content into the input in Agent Mode still attaches the images; nothing about attachment behavior changes.
    - Pasting paths via clipboard is out of scope for this ticket and remains unchanged.
