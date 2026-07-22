//! Action overlay model and `.ass` subtitle generation for burned-in
//! recording annotations. The types are used by the app layer (to collect a
//! per-recording action log) on every platform; `.ass` generation is only built
//! where the burn-in re-encode runs (Linux) or under test.

use std::time::Duration;

use crate::{Action, Key, MouseButton, ScrollDirection, TargetedAction, Vector2I};

/// A group of semantic actions dispatched in one `UseComputer` call.
///
/// One entry represents one *successful* `UseComputer` call: `offset` is when
/// the client began executing the call's action sequence, and `finish_offset`
/// is when that complete sequence (including any explicit waits and the
/// requested post-action screenshot) returned. Failed or cancelled calls never
/// become entries.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ActionLogEntry {
    /// Time from when capture went live to when this group's `UseComputer` call
    /// began executing on the client.
    pub offset: Duration,
    /// Time from when capture went live to when this group's complete action
    /// sequence (and any post-action screenshot) finished.
    pub finish_offset: Duration,
    pub labels: Vec<String>,
    /// Resolved pointer events dispatched during this group, in capture-space
    /// pixels, used to burn in click ripples and drag trails. Empty on paths
    /// that record no pointer geometry.
    pub pointer_events: Vec<PointerEvent>,
}

/// A single resolved pointer event captured at dispatch time.
///
/// `point` is a capture-space pixel (full-screen capture: physical root/screen
/// pixels; window capture: window-local pixels) and `offset` is measured on the
/// same source/1x timeline as [`ActionLogEntry::offset`].
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PointerEvent {
    pub offset: Duration,
    pub kind: PointerEventKind,
    /// The button for a press/release; `None` for a move.
    pub button: Option<MouseButton>,
    pub point: Vector2I,
}

/// Which pointer primitive a [`PointerEvent`] represents.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum PointerEventKind {
    Down,
    Move,
    Up,
}

/// Returns true if a `UseComputer` action batch contains at least one real
/// interaction — any non-`Wait` action (keyboard, typing, pointer, or scroll).
/// A wait-only or zero-duration no-op batch (for example a screenshot-only
/// call, which emits a single `Wait(0)`) is not a qualifying action group and is
/// not committed to the recording timeline. A pointer-only batch still
/// qualifies (with empty labels) so its on-screen effects are retained.
pub fn is_meaningful_action_group(actions: &[TargetedAction]) -> bool {
    actions
        .iter()
        .any(|targeted| !matches!(targeted.action, Action::Wait(_)))
}

enum LabelCandidate {
    Key(Vec<Key>),
    Label(String),
}
/// Converts one `UseComputer` call into ordered, redaction-safe overlay labels.
///
/// Key down/up primitives are grouped until all pressed keys are released. Text
/// and scroll actions become semantic labels; pointer and meta actions are
/// omitted. The call-level summary preserves provider naming for a lone key
/// group, but structured actions reconstruct multi-action calls and always
/// determine printable-key redaction.
pub fn overlay_labels_for(actions: &[TargetedAction], action_summary: &str) -> Vec<String> {
    let candidates = collect_label_candidates(actions);
    let use_action_summary = matches!(candidates.as_slice(), [LabelCandidate::Key(_)]);

    candidates
        .into_iter()
        .map(|candidate| match candidate {
            LabelCandidate::Key(keys) => {
                key_label(&keys, use_action_summary.then_some(action_summary))
            }
            LabelCandidate::Label(label) => label,
        })
        .collect()
}

fn collect_label_candidates(actions: &[TargetedAction]) -> Vec<LabelCandidate> {
    let mut candidates = Vec::new();
    let mut current_keys = Vec::new();
    let mut pressed_keys = Vec::new();
    for targeted in actions {
        match &targeted.action {
            Action::KeyDown { key } => {
                if pressed_keys.is_empty() && !current_keys.is_empty() {
                    candidates.push(LabelCandidate::Key(std::mem::take(&mut current_keys)));
                }
                if !current_keys.contains(key) {
                    current_keys.push(key.clone());
                }
                pressed_keys.push(key.clone());
            }
            Action::KeyUp { key } => {
                if let Some(index) = pressed_keys.iter().position(|pressed| pressed == key) {
                    pressed_keys.remove(index);
                }
            }
            Action::TypeText { .. } => {
                flush_keys(&mut candidates, &mut current_keys, &mut pressed_keys);
                candidates.push(LabelCandidate::Label("typing\u{2026}".to_string()));
            }
            Action::MouseWheel { direction, .. } => {
                flush_keys(&mut candidates, &mut current_keys, &mut pressed_keys);
                candidates.push(LabelCandidate::Label(scroll_label(*direction).to_string()));
            }
            Action::Wait(_)
            | Action::MouseDown { .. }
            | Action::MouseUp { .. }
            | Action::MouseMove { .. } => {
                flush_keys(&mut candidates, &mut current_keys, &mut pressed_keys);
            }
        }
    }
    flush_keys(&mut candidates, &mut current_keys, &mut pressed_keys);
    candidates
}

fn key_label(keys: &[Key], action_summary: Option<&str>) -> String {
    if matches!(keys, [Key::Char(ch)] if !ch.is_control()) {
        return "typing\u{2026}".to_string();
    }

    let label = action_summary
        .map(key_label_from_summary)
        .unwrap_or_else(|| key_label_from_keys(keys));
    redact_printable_key(label)
}

fn flush_keys(
    candidates: &mut Vec<LabelCandidate>,
    current_keys: &mut Vec<Key>,
    pressed_keys: &mut Vec<Key>,
) {
    if !current_keys.is_empty() {
        candidates.push(LabelCandidate::Key(std::mem::take(current_keys)));
    }
    pressed_keys.clear();
}

fn redact_printable_key(label: String) -> String {
    let mut chars = label.chars();
    if chars.next().is_some_and(|ch| !ch.is_control()) && chars.next().is_none()
        || label.eq_ignore_ascii_case("space")
    {
        "typing\u{2026}".to_string()
    } else {
        label
    }
}

fn key_label_from_summary(summary: &str) -> String {
    summary
        .find('"')
        .zip(summary.rfind('"'))
        .filter(|(first, last)| last > first)
        .map(|(first, last)| summary[first + 1..last].to_string())
        .unwrap_or_else(|| {
            let trimmed = summary.trim();
            if trimmed.is_empty() {
                "key".to_string()
            } else {
                trimmed.to_string()
            }
        })
}

fn key_label_from_keys(keys: &[Key]) -> String {
    keys.iter()
        .map(|key| match key {
            Key::Char(ch) => ch.to_string(),
            Key::Keycode(keycode) => match *keycode as u32 {
                0xFF09 => "Tab",
                0xFF0D => "Return",
                0xFF1B => "Escape",
                0xFF51 => "Left",
                0xFF52 => "Up",
                0xFF53 => "Right",
                0xFF54 => "Down",
                0xFFE1 | 0xFFE2 => "shift",
                0xFFE3 | 0xFFE4 => "ctrl",
                0xFFE9 | 0xFFEA => "alt",
                0xFFEB | 0xFFEC => "super",
                _ => "key",
            }
            .to_string(),
        })
        .collect::<Vec<_>>()
        .join("+")
}

fn scroll_label(direction: ScrollDirection) -> &'static str {
    match direction {
        ScrollDirection::Up => "scroll \u{2191}",
        ScrollDirection::Down => "scroll \u{2193}",
        ScrollDirection::Left => "scroll \u{2190}",
        ScrollDirection::Right => "scroll \u{2192}",
    }
}

#[cfg(any(linux, test))]
const PILL_FONT_SIZE: i32 = 48;
#[cfg(any(linux, test))]
const APPROX_GLYPH_WIDTH: i32 = 29;
#[cfg(any(linux, test))]
const PILL_HORIZONTAL_PADDING: i32 = 32;
#[cfg(any(linux, test))]
const PILL_GAP: i32 = 24;
#[cfg(any(linux, test))]
const PILL_BOTTOM_MARGIN: i32 = 90;
/// Context margins retained around a real action window before cutting. The
/// pre-action lead-in is short because those frames are mostly the thinking/
/// blocked gap the cut removes; the post-action window is longer so the action's
/// on-screen effect (and its overlay pill) stays visible. Neither is a
/// transition and neither changes the 1x playback rate inside a segment.
#[cfg(any(linux, test))]
const SEGMENT_MARGIN_PRE: Duration = Duration::from_millis(250);
#[cfg(any(linux, test))]
const SEGMENT_MARGIN_POST: Duration = Duration::from_millis(1000);

/// One retained source segment of the cut recording.
///
/// `source_start`/`source_end` describe the half-open interval `[start, end)`
/// of the 1x source master that is kept; `output_start` is where that interval
/// begins on the compacted output timeline (segments are concatenated in source
/// order with all gaps removed).
#[cfg(any(linux, test))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct KeepSegment {
    pub(crate) source_start: Duration,
    pub(crate) source_end: Duration,
    pub(crate) output_start: Duration,
}

#[cfg(any(linux, test))]
fn frame_duration(frame_rate: u32) -> Duration {
    Duration::from_secs_f64(1.0 / frame_rate.max(1) as f64)
}

/// The clamped source interval for one action group: `[offset, finish_offset]`
/// clamped to `[0, source_duration]` with a one-source-frame minimum when start
/// and finish collapse into the same frame (for example an instantaneous call).
/// Returns `None` when the clamped interval is empty (the group falls entirely
/// at or beyond `source_duration`).
#[cfg(any(linux, test))]
fn group_source_interval(
    entry: &ActionLogEntry,
    source_duration: Duration,
    frame: Duration,
) -> Option<(Duration, Duration)> {
    let start = entry.offset.min(source_duration);
    let mut finish = entry.finish_offset.min(source_duration);
    if finish < start {
        finish = start;
    }
    if finish - start < frame {
        finish = start + frame;
    }
    let finish = finish.min(source_duration);
    (finish > start).then_some((start, finish))
}

/// The source interval over which an entry's overlay pill is shown. Unlike
/// [`group_source_interval`] (the action window that drives the cut), this
/// lingers [`SEGMENT_MARGIN_POST`] past `finish_offset` so the pill stays
/// readable instead of flashing for a single frame on an instantaneous action.
/// It is bounded by `source_duration` and by the next group's start so pills
/// never extend past kept frames or overlap. Returns `None` when the interval
/// is empty.
#[cfg(any(linux, test))]
fn overlay_display_interval(
    entry: &ActionLogEntry,
    next_offset: Option<Duration>,
    source_duration: Duration,
    frame: Duration,
) -> Option<(Duration, Duration)> {
    let (start, action_finish) = group_source_interval(entry, source_duration, frame)?;
    // Reuse the post-action margin as the linger so the pill never outlasts the
    // footage the cut retained after the action.
    let mut end = entry
        .finish_offset
        .saturating_add(SEGMENT_MARGIN_POST)
        .min(source_duration)
        .max(action_finish);
    if let Some(next_offset) = next_offset {
        end = end.min(next_offset);
    }
    (end > start).then_some((start, end))
}

/// Builds the ordered retained segments for a post-stop cut.
///
/// Each committed action group contributes a `[start, finish]` window (with a
/// one-frame minimum). Every window is expanded by [`SEGMENT_MARGIN_PRE`] before
/// its start and [`SEGMENT_MARGIN_POST`] after its finish, then clamped to
/// `[0, source_duration]`. The expanded windows are then
/// sorted by source start and merged whenever they overlap or touch (adjacent
/// windows become one contiguous segment), and each merged segment is assigned
/// an `output_start` equal to the cumulative duration of the segments before
/// it. Source gaps between merged segments are removed entirely by the cut.
#[cfg(any(linux, test))]
pub(crate) fn build_keep_segments(
    entries: &[ActionLogEntry],
    source_duration: Duration,
    frame_rate: u32,
) -> Vec<KeepSegment> {
    let frame = frame_duration(frame_rate);

    let mut windows: Vec<(Duration, Duration)> = entries
        .iter()
        .filter_map(|entry| group_source_interval(entry, source_duration, frame))
        .map(|(start, finish)| {
            let expanded_start = start.saturating_sub(SEGMENT_MARGIN_PRE);
            let expanded_end = finish.saturating_add(SEGMENT_MARGIN_POST);
            (
                expanded_start.min(source_duration),
                expanded_end.min(source_duration),
            )
        })
        .collect();
    windows.sort_by_key(|(start, _)| *start);

    let mut merged: Vec<(Duration, Duration)> = Vec::new();
    for (start, end) in windows {
        if let Some(last) = merged.last_mut()
            && start <= last.1
        {
            if end > last.1 {
                last.1 = end;
            }
            continue;
        }
        merged.push((start, end));
    }

    let mut segments = Vec::with_capacity(merged.len());
    let mut output_cursor = Duration::ZERO;
    for (start, end) in merged {
        segments.push(KeepSegment {
            source_start: start,
            source_end: end,
            output_start: output_cursor,
        });
        output_cursor += end - start;
    }
    segments
}

/// Remaps a source-timeline interval `[start, end]` onto the compacted output
/// timeline defined by `segments`.
///
/// Returns `None` when the interval is wholly inside a removed gap (no retained
/// segment overlaps it) or when the remapped interval is empty. Intervals that
/// touch a cut boundary are clamped to the retained boundary: the start is
/// clamped up to the first overlapping segment's start, and the end is clamped
/// down to the last overlapping segment's end. Remapping is done at `Duration`
/// precision before ASS centisecond formatting.
#[cfg(any(linux, test))]
pub(crate) fn remap_source_interval(
    start: Duration,
    end: Duration,
    segments: &[KeepSegment],
) -> Option<(Duration, Duration)> {
    if end <= start {
        return None;
    }
    // A segment [ss, se) overlaps [start, end) iff ss < end && se > start.
    let first = segments
        .iter()
        .find(|seg| seg.source_start < end && seg.source_end > start)?;
    let last = segments
        .iter()
        .rev()
        .find(|seg| seg.source_start < end && seg.source_end > start)?;

    let clamped_start = start.max(first.source_start);
    let clamped_end = end.min(last.source_end);
    if clamped_end <= clamped_start {
        return None;
    }
    let out_start = first.output_start + (clamped_start - first.source_start);
    let out_end = last.output_start + (clamped_end - last.source_start);
    (out_end > out_start).then_some((out_start, out_end))
}

/// Builds an ASS subtitle document that renders each entry as a bottom-center
/// row on the compacted output timeline. Entries are ordered by source start;
/// each group's overlay display interval (its action window lingered
/// `SEGMENT_MARGIN_POST` past `finish_offset`, bounded by the next group's start) is
/// remapped through the retained segments before timecode formatting, so pills
/// stay aligned with their actions after the cut and remain readable. Groups
/// whose remapped interval is empty (for example wholly inside a removed gap)
/// emit no dialogue. Pointer-only groups with empty labels commit to the
/// timeline (keeping their segment) but render no pill.
#[cfg(any(linux, test))]
pub(crate) fn build_overlay_ass(
    entries: &[ActionLogEntry],
    dimensions: (u32, u32),
    source_duration: Duration,
    frame_rate: u32,
) -> String {
    let (width, height) = dimensions;
    let segments = build_keep_segments(entries, source_duration, frame_rate);
    let frame = frame_duration(frame_rate);

    let mut script = String::new();
    script.push_str("[Script Info]\n");
    script.push_str("ScriptType: v4.00+\n");
    script.push_str(&format!("PlayResX: {width}\n"));
    script.push_str(&format!("PlayResY: {height}\n"));
    script.push_str("ScaledBorderAndShadow: yes\n\n");
    script.push_str("[V4+ Styles]\n");
    script.push_str(
        "Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, \
         BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, \
         BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding\n",
    );
    // Each dialogue is explicitly positioned; BorderStyle 3 gives each one its
    // own dark background.
    script.push_str(&format!(
        "Style: Pill,DejaVu Sans Mono,{PILL_FONT_SIZE},&H00FFFFFF,&H000000FF,&H00000000,&HB0000000,\
         -1,0,0,0,100,100,0,0,3,16,0,2,40,40,90,1\n\n",
    ));
    script.push_str("[Events]\n");
    script.push_str(
        "Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n",
    );

    let mut ordered: Vec<&ActionLogEntry> = entries.iter().collect();
    ordered.sort_by_key(|entry| entry.offset);

    for (index, entry) in ordered.iter().enumerate() {
        let next_offset = ordered.get(index + 1).map(|next| next.offset);
        let (start, finish) =
            match overlay_display_interval(entry, next_offset, source_duration, frame) {
                Some(interval) => interval,
                None => continue,
            };
        let Some((out_start, out_end)) = remap_source_interval(start, finish, &segments) else {
            continue;
        };

        let widths = entry
            .labels
            .iter()
            .map(|label| approximate_pill_width(label))
            .collect::<Vec<_>>();
        let total_width =
            widths.iter().sum::<i32>() + PILL_GAP * widths.len().saturating_sub(1) as i32;
        let mut left = (width as i32 - total_width) / 2;
        let y = height.saturating_sub(PILL_BOTTOM_MARGIN as u32);

        for (label, pill_width) in entry.labels.iter().zip(widths) {
            let x = left + pill_width / 2;
            script.push_str(&format!(
                "Dialogue: 0,{},{},Pill,,0,0,0,,{{\\an2\\pos({x},{y})}}{}\n",
                format_ass_timecode(out_start),
                format_ass_timecode(out_end),
                escape_ass_text(label),
            ));
            left += pill_width + PILL_GAP;
        }
    }
    script
}

#[cfg(any(linux, test))]
fn approximate_pill_width(label: &str) -> i32 {
    label.chars().count() as i32 * APPROX_GLYPH_WIDTH + PILL_HORIZONTAL_PADDING
}

/// Formats a duration as an ASS timecode (`H:MM:SS.cc`, centisecond precision).
#[cfg(any(linux, test))]
fn format_ass_timecode(duration: Duration) -> String {
    let total_cs = (duration.as_millis() / 10) as u64;
    let cs = total_cs % 100;
    let total_secs = total_cs / 100;
    let secs = total_secs % 60;
    let mins = (total_secs / 60) % 60;
    let hours = total_secs / 3600;
    format!("{hours}:{mins:02}:{secs:02}.{cs:02}")
}

/// Neutralizes characters that would be interpreted by the ASS parser so a label
/// renders as plain text.
#[cfg(any(linux, test))]
fn escape_ass_text(text: &str) -> String {
    text.replace('\\', "")
        .replace('{', "(")
        .replace('}', ")")
        .replace(['\n', '\r'], " ")
}

#[cfg(test)]
#[path = "overlay_tests.rs"]
mod tests;
