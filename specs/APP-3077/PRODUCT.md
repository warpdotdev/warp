# Mermaid diagram rendering in notebooks
Mermaid diagrams should be automatically recognized and rendered when they appear in GitHub Flavored Markdown documents within notebooks.

## Raw and rendered views
In raw view, Mermaid code blocks should remain unaltered and visible exactly as authored in the notebook markdown.

In rendered view, those same Mermaid blocks should appear as rendered images rather than raw source text.

## Rendering lifecycle
Rendering a Mermaid diagram may take time, so the UI should show a loading placeholder while the image is being generated.

Diagram generation must not block the UI. Rendering work should happen asynchronously, likely on a background thread.

## Clipboard and selection behavior
Selection across rendered Mermaid diagrams should preserve the authored markdown text when copied.
When rich-text/HTML clipboard output is available, Mermaid selections may also include HTML that represents the rendered diagram for paste targets that understand HTML.
This iteration does not place diagram image bytes on the clipboard, and direct image-only copy affordances for rendered Mermaid diagrams are out of scope.

## Scrolling and layout behavior
When the outer notebook scrolls, the Mermaid image should scroll naturally with the notebook content.

In rendered view, Mermaid diagrams should behave like responsive block content rather than like fixed-size thumbnails.
By default, a rendered Mermaid diagram should match the sizing behavior users expect from other markdown renderers: it should render at its natural width when that width fits comfortably within the notebook, and scale down to fit the available notebook content width when the diagram would otherwise overflow.
We are explicitly not stretching smaller diagrams to fill the full available notebook width by default.

The rendered height should be derived from the diagram's aspect ratio at the chosen width. We should not impose a small fixed default height that causes the diagram to be scaled down inside a larger box.

The rendered diagram should remain fully visible within its block without cropping or letterboxing. The notebook layout must reserve the rendered image height so the diagram never overlaps content below it. If scaling the diagram down to the notebook width still makes it tall, the notebook should simply scroll normally.

This default behavior should optimize for readability of diagram text and labels in the notebook reading experience.

For very large or dense diagrams, future iterations may add dedicated zoom or expand affordances, but the baseline sizing behavior should still be natural-width-or-fit-width with flexible height.

## Theming
We don't need to make mermaid diagram themes match the terminal theme to start but may want this in the future.

## Export
When exporting markdown we should export the raw markdown that was used to generate the diagram.
