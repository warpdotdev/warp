---
name: factory-deferred-repositories
description: Clone a Factory's deferred repository on demand when a task needs it, without overwriting existing checkouts or exposing credentials
---

# Deferred Factory repositories

The run's initial repository inventory names the repositories attached to this Factory but not cloned yet. Use that inventory, not the current working directory or a guessed forge, to identify the repository.

1. Clone only when the task needs the repository. Use its inventory HTTPS clone URL and the run's preconfigured Git credential helper. Never put an access token in a prompt, command, URL, log, or file.
2. Choose an absolute target under the run's workspace root. The preferred target is `<workspace-root>/<repo-name>`, alongside eager checkouts, even when the shell has automatically changed into the sole eager repository.
3. Check that the chosen target does not exist before cloning. If the inventory marks a target-name collision, choose a different unused target explicitly. Never replace or write inside an existing checkout.
4. Quote the HTTPS URL and absolute target as shell arguments, then run `test ! -e '<absolute-target>' && test ! -L '<absolute-target>' && git clone --filter=tree:0 '<https-clone-url>' '<absolute-target>'`. Do not repeat an unsuccessful clone into a partially created target; inspect it first.

An on-demand clone creates a Git working tree only. It does not automatically load repository skills, discover file-based MCP servers, run environment setup commands, initialize the build cache, or register the repository with Oz codebase indexing.
