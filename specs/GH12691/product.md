# Product Spec: Render command block output as Markdown
Issue: https://github.com/warpdotdev/warp/issues/12691
## Summary
Warp users can opt into rendering the output of an individual command block as Markdown. The default terminal-grid rendering remains unchanged, and the toggle affects only the selected block's output display, not the command, raw output data, copy/share contents, or other blocks.
## Problem
Many modern CLIs emit Markdown-formatted stdout, especially LLM CLIs used in one-shot mode. In a normal Warp command block, that output appears as raw Markdown source with visible heading markers, bullets, code fences, emphasis markers, and table syntax. Warp already renders Markdown in AI and notebook surfaces, but plain command output has no way to use that richer presentation.
## Goals / Non-goals
Goals:
- Add an explicit, per-block, opt-in way to switch a command block's output between raw terminal-grid rendering and rendered Markdown.
- Keep raw terminal behavior as the default for all new, existing, restored, shared, and background command blocks.
- Preserve the original terminal output exactly for copy, share, persistence, AI context, search indexing, and any other data-consuming workflow.
- Render Markdown using Warp's existing theme, typography, selection, link, code, and table conventions where practical.
- Keep block height, scrolling, resizing, and context menus correct when rendered Markdown is taller or shorter than the raw grid output.
Non-goals:
- Automatically detecting Markdown output or changing default command-block rendering.
- Rendering the command text, prompt, shell decorations, stderr/stdout boundaries, or input editor content as Markdown.
- Mutating command output into Markdown, stripping ANSI output from stored history, or changing copy/share formats.
- Providing a global setting that makes every command block render Markdown by default.
- Replacing Warp notebooks, AI blocks, or code panes with this command-block display mode.
## Figma
Figma: none provided.
## Behavior
1. Every command block initially renders exactly as it does today: prompt/command and output are displayed in the terminal grid, with ANSI colors, wrapping, links, selection, filters, and scrolling unchanged.
2. When the user opens the context menu for a single command block whose output is not empty, the menu includes `Render output as Markdown` if that block is currently showing raw output.
3. Choosing `Render output as Markdown` changes only that block's output area into a rendered Markdown view. The prompt, command, block header/footer, bookmark, failure stripe, agent-context stripe, and surrounding block chrome stay in their existing positions and styles.
4. A block in Markdown mode shows `Show raw output` in the same context-menu location. Choosing it restores the exact normal terminal-grid output view for that block.
5. The toggle is per-block. Switching one block does not affect any other existing block, the active block, future blocks, split panes, restored conversations, or new sessions.
6. The toggle state remains stable while the block exists in the current terminal model: scrolling away and back, resizing the pane, opening/closing the context menu, changing focus, and switching between terminal view and agent view do not reset the selected block's display mode.
7. Restored blocks and blocks loaded from older persisted data default to raw output. Persisting the user's per-block Markdown display choice across app restarts is not required for the initial version unless implementation review deliberately adds schema support.
8. The context-menu entry is disabled or omitted for block selections where the action would be ambiguous: multi-block selections, non-command rich-content blocks, empty-output blocks, and context menus opened only for selected text.
9. For a running command with non-empty output, the user may enable Markdown mode. New output continues to append to the same block and the rendered Markdown view refreshes as output changes. Incomplete Markdown constructs, such as an unclosed code fence while the command is still streaming, render best-effort and settle into the final rendering when the output completes.
10. Markdown rendering supports, at minimum, headings, paragraphs, hard and soft line breaks, ordered and unordered lists, task-list markers when already supported by the shared parser, inline code, fenced code blocks, emphasis, strong text, strikethrough when already supported, links, horizontal rules, and GitHub-flavored Markdown tables when the corresponding Warp Markdown table feature is enabled.
11. Unsupported Markdown or malformed Markdown never crashes or hides output. If a construct cannot be parsed or rendered safely, that construct falls back to visible text within the rendered output, and the user can always choose `Show raw output` to inspect the original terminal grid.
12. ANSI escape sequences are not interpreted as Markdown syntax. The rendered view is based on the textual contents of the output; raw terminal ANSI color/layout fidelity remains available in raw mode. If ANSI-heavy output produces noisy Markdown rendering, the user can switch back to raw.
13. Rendered Markdown uses Warp theme tokens and terminal-aware colors. Text remains readable in light and dark themes, inline code and code blocks use monospace styling, links use the existing link styling, and tables use the existing themed table treatment rather than hard-coded colors.
14. Long rendered content contributes its measured height to the block list. Scrolling, scroll-to-top/bottom, selection outlines, block borders, failure backgrounds, bookmarks, and viewport positioning remain correct when Markdown output is taller or shorter than the raw output grid.
15. Horizontal overflow is handled safely. Wide tables and long code lines use the same horizontal-scrolling or wrapping behavior as the reused Markdown renderer; they do not force the entire terminal pane into an unusable width.
16. Pane resize reflows rendered Markdown to the new available width and updates the block height. The user does not see stale clipped content, overlapping blocks, or incorrect scroll positions after resize.
17. Copying a whole block, copying output from the context menu, sharing a block/session, saving as a workflow, attaching the block as AI context, and persistence continue to use the original raw command output. The rendered Markdown view is a presentation mode, not a data transformation.
18. Selecting text inside Markdown mode selects the visible rendered text when supported by the rendered components. Copying a rendered selection copies readable plain text for that selection. If selection crosses unsupported visual regions, Warp should still avoid corrupting the raw block selection model or crashing.
19. `Find within block` remains usable in both modes. At minimum, find searches the original output text and navigates to the block. When rendered Markdown exposes line/cell text to the find model, visible matches are highlighted in the rendered view; otherwise raw mode remains the source of exact terminal-grid match highlighting.
20. Links in rendered Markdown are clickable using the same safety and routing behavior as existing Markdown links in AI/notebook surfaces. Raw terminal OSC 8/file-link handling remains unchanged in raw mode.
21. Block filters continue to operate on the raw terminal output. If a filter is active, Markdown mode renders the filtered output text, and clearing or changing the filter refreshes the rendered view for that block.
22. Secret obfuscation remains at least as protective as raw mode. If the raw block would visually obfuscate a secret, the rendered Markdown view must not reveal it through parsing, copying visible selection, link text, table cells, code blocks, or image/diagram metadata.
23. Markdown mode must not interfere with long-running command input, terminal mouse mode, alt-screen behavior, or commands that expect the terminal grid. The toggle is available only for block-list command output, not for alt-screen contents.
24. The feature is discoverable from both the block overflow menu and the right-click block context menu when those menus currently expose single-block actions.
25. The user can recover from any bad rendering by choosing `Show raw output`; there is no destructive state, confirmation dialog, or migration.
## Success criteria
- A user can run an LLM CLI command that prints Markdown, right-click the completed block, choose `Render output as Markdown`, and read headings, lists, code fences, links, and tables without raw Markdown markers dominating the output.
- The same block can be switched back to raw output with no loss of ANSI-styled terminal output.
- Multiple blocks can be independently raw or rendered at the same time.
- Block layout remains stable during scrolling and resize.
- Existing copy/share/persistence workflows keep producing raw command output.
## Validation
- Manual validation should cover a representative Markdown-producing command, a non-Markdown command, an ANSI-colored command, a long table, a long code block, a running command that streams Markdown, an active block filter, and light/dark themes.
- Automated validation should cover per-block state transitions, context-menu labels/disabled states, block height recalculation, parser fallback behavior, raw-copy invariants, and session-restore defaults.
## Open questions
- Should a follow-up version persist a user's per-block display mode across app restart by extending command-block persistence, or is current-session durability sufficient?
- Should command output Markdown mode render local/remote images and Mermaid diagrams in the first version, or should those constructs fall back to links/code until command-block-specific asset resolution is designed?
