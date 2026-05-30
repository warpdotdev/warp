# APP-4267: Mermaid render failures show an explicit callout
## Summary
Mermaid diagrams in rendered markdown surfaces should never leave users staring at an indefinite “Rendering Mermaid diagram…” placeholder. When rendering fails or remains pending too long, Warp replaces the loading state with a clear failure callout while preserving the underlying markdown source.
## Problem
The markdown viewer can show a large Mermaid placeholder that remains stuck on “Rendering Mermaid diagram…” indefinitely. Users cannot tell whether the diagram is still rendering, failed due to invalid Mermaid syntax, failed during SVG/image conversion, or hit an internal renderer hang.
## Goals
- Show an explicit, readable failure state for Mermaid diagrams that fail or time out.
- Preserve successful Mermaid rendering behavior.
- Preserve the authored Mermaid markdown for editing, copying, storage, and export.
- Keep the first iteration lightweight by reusing existing markdown/code-block styling.
## Non-goals
- Adding a dedicated Mermaid source editor.
- Adding retry, “copy raw,” or “open raw source” controls in this iteration.
- Changing Mermaid syntax support, diagram theme, layout algorithm, or rendered SVG fidelity.
- Changing ordinary image loading behavior outside the explicit implementation surface needed for Mermaid failures.
## Figma
Figma: none provided (use existing markdown/code block styling).
## Behavior
1. When a rendered markdown surface contains a fenced Mermaid code block and Mermaid rendering is enabled, Warp initially shows the existing pending state: “Rendering Mermaid diagram…”.
2. The pending state is temporary. A Mermaid render attempt enters the failure state if:
   - Mermaid-to-SVG rendering returns an error.
   - The SVG/image conversion pipeline returns an error.
   - The diagram remains unresolved for longer than the Mermaid render timeout, initially 10 seconds.
3. When a Mermaid render attempt enters the failure state, Warp replaces “Rendering Mermaid diagram…” with a visible callout that says “Failed to render Mermaid diagram”.
4. The failure callout appears inside the same diagram/code-block container where the rendered diagram or loading placeholder would have appeared. It uses theme-derived text, border, and background colors consistent with existing rendered code/markdown blocks.
5. The failure callout is compact enough that a failed diagram does not reserve the large default placeholder height when Warp can determine the render has permanently failed. If the failure is only a UI timeout while the underlying render is still unresolved, the callout must still be visible in the existing placeholder area.
6. A successfully rendered Mermaid diagram continues to display as the rendered SVG, not as a callout.
7. If a render attempt times out but later resolves successfully, Warp replaces the timeout callout with the successfully rendered diagram. Warp must not switch back to the loading placeholder for the same unresolved render attempt.
8. If the underlying Mermaid source changes, Warp treats the new source as a new render attempt: the previous success, failure, or timeout state does not permanently carry over to the changed diagram.
9. Multiple Mermaid diagrams in the same markdown document are independent. A failure in one diagram does not change the rendering, loading, or failure state of any other diagram.
10. The authored Mermaid markdown remains the source of truth. Copying, storing, exporting, sharing, undo/redo, and editing behavior continue to operate on the original fenced Mermaid markdown, not on the failure callout text.
11. When Mermaid rendering is disabled or the surface is intentionally showing raw code blocks, existing raw Mermaid code block behavior is unchanged.
12. The callout has no interactive controls in this iteration, so it does not add keyboard focus stops. It must still be readable by text-based accessibility surfaces as ordinary visible text.
13. Selection behavior around the failed diagram remains consistent with rendered Mermaid diagrams today: users should not accidentally select or copy the failure message instead of the authored Mermaid markdown when operating on the block as markdown content.
14. The failure state should not log a user-visible toast or modal. The error is localized to the diagram block so the rest of the document remains readable.
