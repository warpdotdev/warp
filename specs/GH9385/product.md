# GH9385: Cmd+click full wrapped links in rich CLI-agent output

## Summary
Warp should open the complete target when a user Cmd+clicks a long URL that appears wrapped across multiple rendered lines in rich AI or CLI-agent output, especially output produced by terminal TUIs such as Claude Code. The intended behavior is that Warp treats a strongly contiguous wrapped URL as one logical link even when the producing tool emitted hard line breaks instead of a terminal soft wrap.

## Problem
Users often receive long URLs from CLI and AI-assisted tools. When those tools render a long URL across multiple lines, Warp's rich CLI-agent output can detect only the first line as the clickable URL. Cmd+click then opens an incomplete target, which usually lands on a broken page or loses important query parameters.

This is confusing because regular terminal-grid soft-wrapped URLs already behave like one URL, and the same Claude Code session reportedly opens the full URL in iTerm2. The failure is most visible in narrow panes, split panes, or embedded viewers that reduce the available width.

## Goals
- Cmd+clicking any segment of a qualifying wrapped URL in rich AI or CLI-agent output opens the full URL.
- Hover highlighting and click targets communicate that all visible wrapped segments belong to the same target.
- Single-line URLs and existing markdown hyperlinks continue to work exactly as they do today.
- Regular terminal-grid link detection does not regress.
- Link detection remains conservative enough to avoid turning unrelated TUI layouts, tables, logs, or prose across adjacent lines into a single unintended link.
- Link detection remains performant for long AI outputs, streaming output, and large restored conversations.

## Non-goals
- Rewriting all URL detection across Warp. This spec is focused on rich AI and CLI-agent output where hard line breaks split a logical URL.
- Changing the existing terminal-grid soft-wrap behavior, which already scans across soft-wrapped rows.
- Treating arbitrary hard line breaks as URL continuations. Continuation should require strong URL-safe evidence.
- Adding a new context-menu item such as "Copy full URL". That is a useful follow-up but not required for the Cmd+click fix.
- Requiring upstream tools to emit OSC 8 hyperlinks. Warp should handle common wrapped plain-text URLs defensively even if upstreams do not emit explicit hyperlink metadata.
- Solving all wrapped local filesystem references in this first URL-focused fix. They are related and should be considered in the technical design, but URL correctness is the required product outcome for this issue.

## Figma / design references
Figma: none provided. This is an interaction correctness fix for existing rich output rendering rather than a new visual design.

## User experience
1. When rich AI or CLI-agent output contains a long URL split across adjacent formatted lines, Cmd+clicking any visible segment of that URL opens the complete reconstructed URL.
2. Hovering any segment of a reconstructed wrapped URL shows the same link affordance users see for normal links, including pointer cursor and link highlight.
3. A reconstructed wrapped URL should include all contiguous URL-safe continuation text needed to form the original URL, including path segments, query parameters, fragments, percent escapes, hyphens, underscores, periods, slashes, ampersands, equals signs, and other characters valid in URLs.
4. A reconstructed wrapped URL should not include surrounding prose, bullets, table separators, code-fence syntax, trailing sentence punctuation, or unrelated text from the next line.
5. Cmd+clicking the first line, a middle continuation line, or the final line of the wrapped URL opens the same full URL.
6. Single-line URLs continue to open the exact same URL as before.
7. Markdown hyperlinks such as `[label](https://example.com/long-target)` continue to use the markdown target rather than reconstructed display text.
8. Regular terminal command output outside rich AI/CLI-agent rendering continues to use the terminal grid's existing URL behavior.
9. If Warp cannot confidently determine that adjacent lines are one URL, it should prefer the current safer behavior of not joining them rather than opening a surprising unrelated URL.
10. If an upstream tool emits explicit hyperlink metadata in the future, Warp should prefer that metadata over heuristic reconstruction when supported.
11. Local filesystem references that hard-wrap in rich CLI-agent output should not regress. Whether they are included in the first implementation or handled as a follow-up should be decided during technical review based on performance and false-positive risk.

## Success criteria
- Given rich CLI-agent output like `https://example.com/foo/bar?param=abc` followed immediately by `defghijklmnopqrstuvwxyz`, Cmd+clicking on either line opens `https://example.com/foo/bar?param=abcdefghijklmnopqrstuvwxyz`.
- Given a URL split across three or more adjacent lines, Cmd+clicking any segment opens the full URL and does not truncate at the first visual line.
- Given two independent URLs on adjacent lines, Warp keeps them as two separate links.
- Given a URL followed by prose on the next line, Warp does not append the prose.
- Given a URL in a markdown hyperlink, Warp opens the markdown hyperlink target.
- Given rich output containing tables, bullets, or columns with URL-looking fragments in different cells, Warp does not join across cells unless the text is clearly a single contiguous URL continuation.
- Detection time remains bounded and does not noticeably affect streaming or rendering large AI outputs.

## Validation
- Add unit coverage for reconstructed rich-output links split across two and three formatted lines.
- Add unit coverage for clicking or resolving any line segment to the same full URL target.
- Add negative unit coverage for adjacent independent URLs, URL plus prose, and table-like adjacent fragments.
- Add regression coverage that existing single-line URLs and markdown hyperlinks still resolve normally.
- Manually validate on macOS with a narrow pane and Claude Code output containing a long URL split across lines.
- Manually validate that regular terminal-grid soft-wrapped URLs still open fully.

## Open questions
- Should wrapped local filesystem references be included in the same implementation, or should they be tracked separately after the URL-specific fix lands?
- Should Warp add a "Copy full URL" action for reconstructed wrapped URLs in a follow-up?
- What exact continuation heuristics are acceptable for TUI table layouts where adjacent rows may contain URL-safe text that is not logically connected?
