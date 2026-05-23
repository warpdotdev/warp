# WARP.md

## Visual Sanity Check for Table Example Screenshots
This project can auto-generate screenshots of the table example demos and then sanity check them using computer vision. The goal is to quickly catch apparent rendering bugs (e.g., empty cells, obvious misalignment, missing headers) before committing or opening a PR.

### How to capture images
- Build and run the example with capture flags:
  - Baseline (reference images): `../../../../target/debug/examples/table-sample --capture-baseline`
  - Current (to compare locally): `../../../../target/debug/examples/table-sample --capture-screenshots`
- Output directories:
  - Baseline: `screenshots/baseline/`
  - Current: `screenshots/current/`

### Sanity-check protocol (Agent/Agent Mode)
- Use the read_file tool to upload all PNGs in the chosen directory (baseline or current).
- For each image, scan for:
  - Completely blank/black/solid-color large areas where UI should be rendered
  - Obvious missing headers, rows, or columns
  - Clearly misaligned row bands or headers vs. body
  - Text clipped mid-line or unreadable due to extreme contrast issues
- Report any images that exhibit the above, with a short note.

Notes:
- This is a quick visual smoke test, not a pixel-perfect comparison.
- If a failure is found, re-run the example for a single demo by navigating with arrow keys or by re-running the full capture and re-checking.
