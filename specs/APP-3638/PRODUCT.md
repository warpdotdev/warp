# CLI Agent Rich Input: /prompts Product Spec

## Summary
Add support for `/prompts` (saved prompts) in the CLI agent rich input composer — the input that appears when composing a prompt to send to a running CLI agent (Claude Code, Codex, Gemini CLI, etc.). When a saved prompt is selected, its content is inserted into the input and submitted as plain text to the PTY.

## Problem
When users compose prompts for CLI agents through Warp's rich input (Ctrl-G or the Compose button), they cannot browse or insert saved prompts. The normal Warp agent input supports prompts, but the CLI agent input does not. Users must manually type or paste prompt content.

## Goals
- Let users browse and select saved prompts in the CLI agent rich input.
- Insert the prompt content into the editor, reusing the existing workflow info box flow (with argument highlighting and shift-tab editing).

## Non-goals
- Changing how prompts work in the normal Warp agent input.

## Figma
https://www.figma.com/design/CsBdBW4YoLgSAbr5eSkwV6/House-of-Agents?node-id=7001-18001&p=f&m=dev

## User Experience

### Trigger
User selects a prompt from the prompts menu, same as in normal Warp input.

### Behavior
When a saved prompt is selected, the existing workflow info box flow handles insertion — the prompt template is inserted into the editor with argument highlighting and the shift-tab UX for editing parameters. This works as-is because the CLI agent rich input reuses the same `Input` view and editor. On submit, the buffer text (with filled-in arguments) is written to the PTY as plain text.

## Success Criteria
- Users can select a saved prompt in the CLI agent rich input and have its content inserted with the standard workflow argument editing UX.
- The prompt content submits correctly as plain text to the PTY.

## Validation
- Open a CLI agent rich input. Open the prompts menu, select a prompt, verify the prompt content is inserted with argument highlighting.
- Edit a prompt argument using shift-tab, submit, verify the full text is written to the PTY.
