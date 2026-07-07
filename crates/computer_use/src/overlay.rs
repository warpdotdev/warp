//! Keyboard action overlay model and `.ass` subtitle generation for burned-in
//! recording annotations. The types are used by the app layer (to collect a
//! per-recording action log) on every platform; `.ass` generation is only built
//! where the burn-in re-encode runs (Linux) or under test.

use std::time::Duration;

/// The kinds of input annotated in a recording. Only keyboard input is
/// rendered: pointer and scroll actions are visible on screen via the cursor or
/// content motion, so they are never turned into an overlay entry.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum OverlayKind {
    /// A keyboard shortcut / keypress (e.g. `ctrl+a`).
    Key,
    /// Text entry, rendered as a generic `typing…` label (never the payload).
    Type,
}

/// A single keyboard action to annotate, timed relative to capture start.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ActionLogEntry {
    /// Time from when capture went live to when this action was dispatched.
    pub offset: Duration,
    pub kind: OverlayKind,
    /// The text rendered in the pill (e.g. `ctrl+a`, `typing…`).
    pub label: String,
    /// How long the pill stays on screen before expiring (clamped to the next
    /// entry so only one pill shows at a time).
    pub show_duration: Duration,
}

/// Default on-screen lifetime of a pill.
pub const DEFAULT_PILL_DURATION: Duration = Duration::from_millis(1500);

/// Builds an ASS subtitle document that renders each entry as a bottom-center
/// pill. Entries are ordered by timecode and each pill's end is clamped to the
/// next pill's start so only one is visible at a time.
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
    // Alignment 2 = bottom-center; BorderStyle 3 + a semi-transparent BackColour
    // give the rounded dark pill; MarginV lifts it off the bottom edge.
    script.push_str(
        "Style: Pill,DejaVu Sans,48,&H00FFFFFF,&H000000FF,&H00000000,&HB0000000,\
         -1,0,0,0,100,100,0,0,3,16,0,2,40,40,90,1\n\n",
    );
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
        script.push_str(&format!(
            "Dialogue: 0,{},{},Pill,,0,0,0,,{}\n",
            format_ass_timecode(start),
            format_ass_timecode(end),
            escape_ass_text(&entry.label),
        ));
    }
    script
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
