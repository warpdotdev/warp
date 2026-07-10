# Product Spec: Render LaTeX math (`$...$` / `$$...$$`) in AI agent block output

**Issue:** [warpdotdev/warp#9677](https://github.com/warpdotdev/warp/issues/9677)
**Figma:** none provided

## Summary

AI agents (Warp AI, Claude Code, Codex, Gemini CLI harnesses) frequently emit LaTeX math in their responses. Warp currently displays the raw `$...$` / `$$...$$` source, which is hard to read. This spec covers phase 1 of #9677: recognizing math delimiters in the shared markdown parser and typesetting display math (`$$...$$`) in AI agent block output. Typeset inline math and the markdown file viewer surface are explicit follow-ups.

## Problem

A large share of agent conversations involve math: ML derivations, statistics, optimization, numerical methods. Today a response like

```
The update rule is:
$$
\theta_{t+1} = \theta_t - \eta \nabla_\theta \mathcal{L}(\theta_t)
$$
```

renders as literal dollar-sign soup inside an otherwise polished agent block.

## Goals

- Recognize `$...$` (inline) and `$$...$$` (display) math per pandoc's `tex_math_dollars` conventions in the shared markdown parser.
- Typeset display-math blocks in AI agent output as theme-colored, centered visual blocks (KaTeX-level LaTeX coverage).
- Math support must never break existing rendering: on any parse/typesetting failure, show the raw source unchanged.
- Copy/selection of a math block yields the original `$$...$$` LaTeX source.

## Non-goals (follow-ups)

- Typeset **inline** math rendering inside text lines (requires inline-image layout in text runs). Inline spans are recognized by the parser and preserved as raw source with a `math` style so renderers can adopt them incrementally.
- The markdown **file viewer / notebooks** surface (`BlockItem` pipeline).
- Full MathJax coverage; KaTeX-level coverage is sufficient (per issue).
- LaTeX environments outside math mode (`\begin{document}`, etc.).

## Behavior invariants

1. A line containing only `$$`, followed by LaTeX lines, followed by a line containing only `$$`, renders as a centered typeset equation block in agent output.
2. A standalone line of the form `$$<latex>$$` renders the same way.
3. Inline `$...$` spans follow pandoc rules in the parser: the opener `$` must be immediately followed by non-whitespace; the closer must be immediately preceded by non-whitespace and not immediately followed by a digit. Therefore `It costs $20,000 and $30,000 total` is plain text.
4. `\$` is a literal dollar sign and never opens/closes math.
5. `$`/`$$` inside inline code spans and fenced code blocks are literal (code has higher precedence).
6. Unclosed `$` or `$$` renders as literal text. While an agent response is streaming, an unterminated `$$` block therefore shows its raw source and upgrades to a typeset block when the closing `$$` arrives.
7. Empty or whitespace-only math (`$$$$`, `$$ $$`) is not math.
8. On LaTeX the typesetter cannot handle, the block falls back to raw `$$...$$` source; no error UI, no broken layout.
9. Typeset math uses the block's text color (theme-aware in light and dark themes) and scales with the configured font size.
10. Copying a math section yields its `$$...$$` markdown source, byte-for-byte.
11. Find-in-block matches against the raw LaTeX source of math sections.
12. Existing shell-block, table, code-block, image, and Mermaid behavior is unchanged.

## Success criteria

1. Asking an agent a question whose answer contains display math shows typeset equations in place of raw source.
2. All invariants above hold, with unit-test coverage for the parser rules (invariants 3–7) and the section splitter (1, 2, 5, 6, 7).
3. No regression in existing markdown parsing/rendering test suites.
