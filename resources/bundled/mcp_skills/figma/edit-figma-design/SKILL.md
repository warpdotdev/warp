---
name: edit-figma-design
description: Create or update Figma designs directly from a written product or UI description using the Figma MCP authoring tools. Use when the user wants a mockup, wireframe, screen, component, flow, or concept designed in Figma from text, or wants to iterate on an existing Figma file from textual feedback. Despite the name, this skill can start from a new blank file or edit an existing one. Do not use for capture-based workflows that turn a running page into Figma; use `figma-generate-design` for those, and use `implement-design` for code implementation requests. Requires Figma MCP server connection.
metadata:
  mcp-server: figma
---

# Edit Figma Design

## Overview

This skill creates or updates Figma designs directly from a natural-language description. It combines Figma library search with direct file authoring, and uses Warp's broader agent capabilities only when they are needed to make the design more product-aware or codebase-aware.

## When to use this skill

Use this skill when the user wants you to:

- design a new screen, flow, component, or mockup in Figma from a written description
- refine or extend an existing Figma file from text feedback
- create a first-pass wireframe or higher-fidelity design directly in Figma
- align a Figma design to an existing design system or product vocabulary

Do not use this skill when:

- the user wants production code from a design — use `implement-design`
- the user wants to capture a running page or app into Figma — use `figma-generate-design`
- the user only wants to inspect or pull existing Figma context — use `pull-figma-content`

## Prerequisites

- Figma MCP server must be connected and accessible.
  - Verify that `search_design_system`, `create_new_file`, and `use_figma` are available.
- Gather the minimum information needed to proceed:
  - what should be designed
  - whether to use an existing Figma file or create a new one
  - whether the result should align to an existing design system or codebase
- Ask clarifying questions only when the user has not already given enough detail to start. Keep them short and batch them into one message when possible.

## Required Workflow

**Follow these steps in order. Do not skip steps.**

### Step 1: Confirm this is a Figma-authoring request

If the user is actually asking for implementation, stop and consult `implement-design`.

If the user wants a screenshot-to-Figma or webpage capture flow, stop and consult `figma-generate-design`. That skill is for capture-based workflows; this skill is for text-to-design authoring.

### Step 2: Resolve the destination file first

Both `search_design_system` and `use_figma` need a `fileKey`, so determine the destination before searching or editing.

**If the user provided an existing Figma URL or file key:**

- Extract and use that `fileKey`.
- Reuse the provided URL when you respond.

**If the user wants a new file:**

1. Decide on a clear file name from the request.
2. If the user already provided a `planKey`, use it.
3. Otherwise call the Figma MCP `whoami` tool to inspect the authenticated Figma user and available plans. This is not the shell `whoami` command.
4. If there is exactly one plan, use its `key`.
5. If there are multiple plans, ask the user which team or organization to use.
6. Call `create_new_file(editorType="design", fileName=..., planKey=...)`.
7. Save the returned `fileKey` and URL. Share the URL once the first usable draft is ready.

### Step 3: Gather the right context, but only when it is needed

Decide how much non-Figma context is actually necessary.

**Stay inside Figma MCP only** when the user wants an exploratory concept, wireframe, or mockup and does not ask for codebase alignment.

**Use Warp agent context selectively** when the user wants the design to match an existing product or design system:

- read project rules from `AGENTS.md` and/or `WARP.md` if they exist
- use semantic codebase search, grep, and file reads to find relevant components, product vocabulary, layout patterns, and design-token sources
- use other MCP sources or web search only when the prompt directly depends on them, such as product requirements in another system or explicit inspiration requests
- do not edit code, run REPL commands, or use computer use as part of this skill's normal workflow

### Step 4: Search the design system before authoring

Call `search_design_system` with the resolved `fileKey` before creating new components or styles.

Search for the most reusable assets first:

- components and component sets
- variables and token-like values
- styles for color, typography, spacing, or effects

Start with the user's domain terms and any names discovered from project rules or codebase search.

If needed, narrow follow-up searches with returned library keys rather than immediately broadening the search.

Prefer reusing and importing matches over recreating them from scratch.

### Step 5: Prepare `use_figma` safely

Before the first `use_figma` call, plan the edit sequence and follow the tool's required Plugin API constraints.

Keep the authoring plan incremental:

1. create page and frame structure
2. establish layout and major sections
3. reuse or import design-system assets
4. apply variables, styles, and typography
5. add content and polish
6. make targeted revisions based on what the file now contains

### Step 6: Edit the design in small `use_figma` steps

Use multiple small `use_figma` calls instead of one giant script.

Good step boundaries:

- create a page and top-level frames
- lay out a header, sidebar, hero, or content region
- import or place one family of reusable components
- bind colors, text styles, or spacing variables
- update copy, states, or alignment for a specific section

After each step, inspect the result and only continue once the previous step succeeded.

When creating anything component-like, prefer imported library assets discovered in Step 4.

### Step 7: Hand back the design and next options

When the first usable draft is ready:

- return the Figma URL if you have it
- summarize what you created or updated at a high level
- ask whether the user wants revisions in Figma

If the user asks to implement the approved design in code, stop using this skill and consult `implement-design`.

## Warp-agent guidance

Use Warp's broader capabilities to reduce manual prompting, not to add unnecessary work.

**Good uses of Warp agent capabilities in this skill:**

- finding existing component names or design tokens in the repo
- reading project rules that constrain layout, naming, or branding
- pulling product requirements from other connected systems when the user explicitly relies on them

**Usually unnecessary for this skill:**

- shell commands or REPL access
- code edits
- computer-use validation
- broad web research without a specific user request

## Examples

### Example 1: New file from a product description

User says: "Design a billing overview screen in Figma for our desktop app. Use our existing design system and create a new file."

**Actions:**

1. Confirm this is Figma authoring, not code implementation.
2. Resolve the destination by calling `whoami` if needed, then `create_new_file`.
3. Read `AGENTS.md` or `WARP.md`, or search the codebase only if needed to understand billing terminology and existing components.
4. Call `search_design_system` with billing-related queries.
5. Build the screen in small `use_figma` steps.
6. Return the new Figma file URL and offer to revise.

### Example 2: Update an existing Figma file

User says: "Add an onboarding checklist to this Figma file: https://figma.com/design/FILEKEY/Product?node-id=1-2"

**Actions:**

1. Extract the `fileKey` from the existing URL.
2. Search the design system for checklist, card, badge, and progress assets before creating anything new.
3. Use incremental `use_figma` calls to add the new section.
4. Return the same Figma URL and summarize the change.

### Example 3: Pure exploratory concept

User says: "Create a first-pass mobile workout planner mockup in Figma. It doesn't need to match my codebase yet."

**Actions:**

1. Create a new file if needed.
2. Skip codebase search and project-rule inspection.
3. Use `search_design_system` only to reuse any relevant Figma library assets.
4. Build the concept directly in Figma with small `use_figma` steps.
5. Share the file link and ask what to refine next.

## Common issues and responses

### Issue: The user hasn't said whether to use an existing file or a new one

Ask one direct question that resolves the destination. Do not start `search_design_system` or `use_figma` until you have a `fileKey`.

### Issue: Multiple Figma plans are available for `create_new_file`

Ask the user which team or organization to use. Do not guess.

### Issue: The user wants the design to match existing product conventions, but the request is vague

Read the project's rules first. Then use targeted codebase search to gather only the components and conventions relevant to the requested surface.

### Issue: The user asks for both a Figma design and implementation

Create or update the Figma design first only if the user's request is primarily about authoring in Figma. If the request is primarily about implementation, consult `implement-design` instead. After the design is approved, implementation can follow in a separate step.

### Issue: `use_figma` fails or the script is getting large

Break the task into smaller `use_figma` calls. Prefer structure first, then styling, then targeted revisions.

## Additional resources

- [Figma MCP Server Documentation](https://developers.figma.com/docs/figma-mcp-server/)
- [Figma MCP Server Tools and Prompts](https://developers.figma.com/docs/figma-mcp-server/tools-and-prompts/)
