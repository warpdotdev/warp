# Session Restore: Preserve PWD for WSL and Git Bash

## Summary
When Warp restores a session that was running in WSL or an MSYS2-based shell (Git Bash, MSYS2), it re-opens that terminal in the same working directory the user was in when the session was saved.

## Behavior

1. When a Warp session is snapshotted (e.g. on app quit, window close, or session save), the current working directory is persisted as part of the session state.

2. When a WSL session is restored, the new terminal opens with its working directory set to the same path the user was in before. For example, if the user was in `/home/user/projects`, the restored terminal starts there.

3. When an MSYS2 or Git Bash session is restored, the new terminal opens with its working directory set to the same path the user was in before. For example, if the user was in `/c/Users/user/projects`, the restored MSYS2 session starts in `/c/Users/user/projects`.

4. If the working directory stored in the snapshot no longer exists, the terminal opens with no startup directory override — falling back to the shell's default.

5. Session restore behavior for plain Windows shells (PowerShell, Cmd) and native-Unix shells is unaffected.
