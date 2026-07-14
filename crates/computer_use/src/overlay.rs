//! Action overlay model and `.ass` subtitle generation for burned-in
//! recording annotations. The types are used by the app layer (to collect a
//! per-recording action log) on every platform; `.ass` generation is only built
//! where the burn-in re-encode runs (Linux) or under test.

use std::time::Duration;

use crate::{Action, Key, ScrollDirection, TargetedAction};

/// A group of semantic actions dispatched in one `UseComputer` call.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ActionLogEntry {
    /// Time from when capture went live to when this group was dispatched.
    pub offset: Duration,
    pub labels: Vec<String>,
    /// How long the group stays on screen before expiring.
    pub show_duration: Duration,
}

fn is_unmodified_printable_key(keys: &[Key]) -> bool {
    matches!(keys, [Key::Char(ch)] if !ch.is_control())
}

/// Default on-screen lifetime of an action group.
pub const DEFAULT_PILL_DURATION: Duration = Duration::from_millis(1500);

enum LabelCandidate {
    Key(Vec<Key>),
    Label(String),
}

pub fn overlay_labels_for(actions: &[TargetedAction], action_summary: &str) -> Vec<String> {
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

    let candidate_count = candidates.len();
    let key_count = candidates
        .iter()
        .filter(|candidate| matches!(candidate, LabelCandidate::Key(_)))
        .count();
    candidates
        .into_iter()
        .map(|candidate| match candidate {
            LabelCandidate::Key(keys) => {
                let label = if is_unmodified_printable_key(&keys) {
                    "typing\u{2026}".to_string()
                } else if key_count == 1 && candidate_count == 1 {
                    key_label_from_summary(action_summary)
                } else {
                    key_label_from_keys(&keys)
                };
                redact_printable_key(label)
            }
            LabelCandidate::Label(label) => label,
        })
        .collect()
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

/// Builds an ASS subtitle document that renders each entry as a bottom-center
/// row. Entries are ordered by timecode and each group's end is clamped to the
/// next group's start.
#[cfg(any(linux, test))]
pub(crate) fn build_overlay_ass(entries: &[ActionLogEntry], dimensions: (u32, u32)) -> String {
    let (width, height) = dimensions;
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
        let start = entry.offset;
        let mut end = entry.offset + entry.show_duration;
        if let Some(next) = ordered.get(index + 1)
            && next.offset < end
        {
            end = next.offset;
        }
        if end <= start {
            continue;
        }

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
                format_ass_timecode(start),
                format_ass_timecode(end),
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
