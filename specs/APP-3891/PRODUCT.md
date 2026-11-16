# CLI Agent Image Paste

## Summary

Allow users to paste screenshots into the CLI agent rich input (e.g. when composing prompts for Claude Code, Gemini CLI, Codex) and have them delivered to the CLI agent as images. Images appear as removable chips in the rich input before submission, matching the existing agent mode UX.

## Problem

When using CLI agents like Claude Code through Warp's rich input, users cannot share visual context (screenshots, UI mockups, error dialogs) with the agent. The only workaround is to save the image to disk, note the file path, and manually type a reference to it — which breaks the conversational flow.

Warp's agent mode already supports pasting images as attachment chips, but the CLI agent rich input did not render chips or deliver images on submission.

## Goals

- Users can paste screenshots into the CLI agent rich input and see them as removable image chips.
- On submission, attached images are delivered to the CLI agent so it can actually see them.
- The chip UX (add, remove, limits) matches existing agent mode behavior with no new UI to learn.
- Works with any CLI agent that supports clipboard image paste out of the box.

## Non-goals

- Drag-and-drop image files into the CLI agent rich input (should work for free via existing infrastructure, but not explicitly targeted or tested).
- Supporting CLI agents that cannot read images from the clipboard (e.g. agents with no Ctrl+V image support).
- Inline image preview/thumbnails within the rich input — chips show filename only, matching agent mode.
- Sending images to Warp's own agent mode backend through this path (that uses a separate server-side flow).

## Figma

Figma: none provided. The UI reuses existing attachment chip rendering from agent mode — no new visual design needed.

## User experience

### Attaching images

1. User opens the CLI agent rich input (Ctrl+G or footer button) while a CLI agent is running.
2. User takes a screenshot or copies an image to the clipboard.
3. User pastes (Cmd+V / Ctrl+V) into the rich input.
4. An image chip appears above the editor, showing the filename (e.g. `pasted-image-1713121234.png`) with an × button to remove it.
5. Multiple images can be pasted. Each appears as a separate chip. The same per-query and per-conversation limits from agent mode apply.

### Removing images

- Clicking the × on a chip removes that image. This uses the existing `DeleteAttachment` action — identical to agent mode.

### Submitting with images

- When the user submits the prompt (Enter), images are delivered to the CLI agent first, followed by the text prompt.
- The image chips disappear after submission.
- If no images are attached, submission behaves exactly as before.

### Delivery mechanism

Images are delivered by simulating what a user would do manually: for each attached image, Warp writes the image data to the system clipboard and sends Ctrl+V (`0x16`) to the PTY. The CLI agent (e.g. Claude Code) reads the image from the clipboard natively.

- A 500ms delay is inserted between each image paste to give the CLI agent time to read from the clipboard before it's overwritten with the next image. This was tested empirically in prototype - we need a relatively significant delay here for the CLI agent to pick up the paste correctly.
- After all images are pasted, the text prompt is sent using the agent-specific submission strategy (inline, bracketed paste, or delayed enter).

## Alternate approaches considered

### 1. Save images to temp files and include file paths in the prompt text

The first approach implemented was to decode each attached image from base64, write it to a temp file on disk (e.g. `/var/folders/.../warp-cli-image-1776199745040956000.png`), and prepend a `[Attached images: /path/to/file]` block to the prompt text.

**Why we didn't choose this:**
- The temp file paths are ugly and OS-specific (`/var/folders/...` on macOS).
- CLI agent then has to go read the files from given paths.
- Requires cleanup logic (tracking temp files, deleting on session end).
- Not how a user would naturally share an image with a CLI agent.

### 2. Don't intercept image paste at all — let raw Ctrl+V pass through to the PTY

Instead of showing chips, let the paste keypress go directly to the CLI agent's PTY so its own image handling takes over.

**Why we didn't choose this:**
- Loses the chip UI entirely — no visual feedback before submission, no ability to remove an accidentally pasted image, no multi-image staging.

### 3. Encode images inline in the prompt (base64 or data URI)

Embed the image data directly in the prompt text sent to the PTY.

**Not considered seriously because:**
- No CLI agent parses inline base64 image data from stdin.
- Would produce enormous, unreadable prompt text.

## Success criteria

1. Pasting a screenshot (Cmd+V) into the CLI agent rich input produces an image chip above the editor.
2. The chip shows a filename and an × close button.
3. Clicking × removes the chip and the underlying pending attachment.
4. Submitting with one attached image: Claude Code shows `[Image #1]` in its prompt and can describe the image content.
5. Submitting with two attached images: Claude Code shows `[Image #1] [Image #2]` and can distinguish between them.
6. Submitting with no attached images behaves identically to the previous behavior (no regression).
7. Image attachment limits (per-query and per-conversation) are enforced, with toast messages for excess images.

## Validation

- **Manual test — single image**: Paste a screenshot, type a prompt referencing the image, submit. Verify Claude Code sees and describes the image correctly.
- **Manual test — multiple images**: Paste two different screenshots, submit. Verify Claude Code sees both as distinct images (`[Image #1]` and `[Image #2]`).
- **Manual test — remove chip**: Paste an image, click ×, submit. Verify no image is sent to the CLI agent.
- **Manual test — no images**: Submit a text-only prompt. Verify behavior is unchanged.
- **Manual test — limits**: Paste more images than the per-query limit. Verify a toast appears and excess images are not attached.
- **Build verification**: `cargo fmt` and `cargo clippy` pass with no warnings.

## Open questions

- **Delay tuning**: The 500ms delay between image pastes is sufficient for Claude Code but may need adjustment for other CLI agents. Need to explore.
- **Non-image-paste agents**: For CLI agents that don't support Ctrl+V image paste, should we fall back to the file path approach or simply not send images?
Need to check if this applies to any CLI agents.
