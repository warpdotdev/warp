# Integration Test Recording Overlays

## Summary
Add an API to the integration test video recording framework that, when enabled, renders visual event metadata in an overlay layer within the recorded video. The goal is to make recorded test videos self-explanatory so a viewer can understand which mouse and keyboard actions were fired during the test without reading the test source.

## Problem
Current integration test recordings show the UI state changing over time, but they do not clearly communicate which input events caused those changes. This makes recordings harder to use for debugging, regression review, demos, and collaboration because a viewer has to infer whether a state transition came from a click, a drag, a keypress, or some combination of inputs.

## Goals
- Make integration test recordings easier to interpret without needing external narration or source-code context.
- Show the user inputs that occurred at the moment they occurred in the recording.
- Follow familiar conventions from other screen recording and presentation tools that use animated annotations for clicks, drags, and keyboard shortcuts.
- Keep the feature opt-in through an API on the video recording framework.

## Non-Goals
- Changing how tests dispatch events.
- Building a full analytics or event-inspection UI outside the recorded video.
- Capturing every possible event type in the first version beyond clicks, click-and-drag, and keyboard events.

## Primary Use Cases
- A developer watches a failed integration test video and can immediately see which click triggered the incorrect UI response.
- A reviewer watches a video attached to a change and can understand a drag gesture or keyboard shortcut without pausing to inspect test code.
- A developer records a repro video for a UI bug and wants the visible annotations to explain the interaction sequence clearly.

## Required Recorded Events

### Clicks
When a click occurs, the video should show a transient animated annotation at the pointer location. This should follow the common pattern used by screen recording apps, such as a pulse, ring, or highlight that makes the click easy to notice without obscuring the UI underneath.

### Click and Drag
When a click-and-drag gesture occurs, the video should show:
- The drag start location
- The drag path or motion between points
- The drag end state

The visualization should make it obvious that the pointer movement was part of a drag gesture rather than ordinary cursor motion.

### Keyboard Events
When keyboard input occurs, the video should show the keys being fired in a compact overlay. The annotation must include:
- Modifier keys, if any
- The primary key being fired

Examples include combinations such as `Cmd+K`, `Shift+Enter`, or `Ctrl+C`, as well as non-modified keys when they are relevant.

## Product Requirements
- The overlay feature must be enabled through an explicit API in the integration test video recording framework.
- When disabled, recording behavior should remain unchanged.
- Overlay rendering should be synchronized with the recorded event timeline so annotations appear at the correct moment in the video.
- Overlays should be visually understandable at normal playback speed without requiring frame-by-frame inspection.
- Animations should be clear but lightweight, avoiding excessive distraction or covering important parts of the UI for too long.
- The system should support multiple keyboard events in sequence and render them in a way that remains readable.
- Mouse and keyboard overlays should be visually consistent so the recording feels like a single coherent annotation system.

## Proposed Experience

### Mouse Annotations
- Clicks use a brief animated pulse or ring centered on the pointer.
- Drag gestures use a visible start indicator and a short-lived path or trail that communicates direction and distance.
- The styling should feel similar to established screen recording tools that visualize cursor interactions for demos and tutorials.

### Keyboard Annotations
- Key events appear as a compact on-screen overlay, likely near the bottom of the video or another consistent location that does not interfere with important UI.
- Modifier keys and the main key should be shown together as a single composed event.
- The overlay should be readable, transient, and consistent across repeated actions.

## API Expectations
The recording framework should expose an API that enables these annotations for a given test recording session. The API should be simple enough that a test author can opt into annotated videos without changing how the test itself expresses input events.

At a minimum, the API should:
- Turn overlay capture on for a recording
- Observe or receive input events already emitted by the test framework
- Render those events into a visual overlay layer in the final video output

## Design Principles
- Prefer familiar annotation patterns over novel visuals.
- Optimize for clarity to someone watching the video for the first time.
- Keep the overlay declarative and easy to enable in tests.
- Ensure the feature improves debugging value without materially changing test authoring ergonomics.

## Success Criteria
- A viewer can correctly identify when a click, drag, or keyboard action occurred by watching the recording alone.
- Keyboard shortcuts are understandable from the video, including modifier keys.
- The overlay feels polished and familiar, matching expectations set by other screen recording applications with animated interaction annotations.
