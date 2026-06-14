# Agent Guidelines for warp

## Visual Evidence

When working in this repository, agents should proactively include visual evidence for any user-visible client change. This reduces the need for users to repeatedly ask for screenshots.

### When to capture visual evidence

**Always include visual evidence for:**
- Layout or spacing changes (padding, margins, sizing, alignment)
- Color, opacity, or theme token changes
- Border, corner radius, or background fill changes
- New UI components or views
- Rendering/display bug fixes

**Visual evidence is not needed for:**
- Internal refactors with no visible output change
- Non-UI logic (data structures, algorithms, event handling)
- Build configuration, CI, or dependency bumps
- Documentation-only changes

### What to do when ambiguous

If it's unclear whether a change produces a user-visible difference, ask the user before opening the PR:

> "This change touches `[file/component]`. Should I capture a screenshot of the result with computer use before opening the PR?"

### How to capture and attach evidence

Use the `upload-screenshot` skill from `common-skills`. It handles:
1. Building and launching Warp via computer use to capture a screenshot
2. Uploading to a stable image host
3. Embedding the image in the PR description's **Screenshots / Videos** section
4. Optionally posting to a Slack thread

See the skill at `common-skills/.agents/skills/upload-screenshot/SKILL.md` for the full workflow.

### PR description format

Include a **Screenshots / Videos** section in the PR description for any UI change:

```markdown
### Screenshots / Videos

![Description of what is shown](https://i.imgur.com/XXXX.png)

<details>
<summary>Full screenshot</summary>

![Full view](https://i.imgur.com/YYYY.png)

</details>
```
