use std::io::Write;
use std::path::Path;
use std::time::Duration;

use instant::Instant;

use crate::Event;

/// Well-known key used to store the `ActionLog` inside `StepDataMap`.
pub const ACTION_LOG_KEY: &str = "action_log";

/// A single event recorded in the action log.
pub struct ActionEntry {
    /// Wall-clock instant this entry was recorded.
    recorded_at: Instant,
    /// Human-readable description of the event.
    description: String,
}

/// Returns a concise, human-readable description of an event for the action log.
pub fn event_description(event: &Event) -> String {
    match event {
        Event::KeyDown { chars, .. } => format!("KeyDown '{chars}'"),
        Event::TypedCharacters { chars } => format!("TypedCharacters '{chars}'"),
        Event::LeftMouseDown { .. } => "LeftMouseDown".to_string(),
        Event::LeftMouseUp { .. } => "LeftMouseUp".to_string(),
        Event::LeftMouseDragged { .. } => "LeftMouseDragged".to_string(),
        Event::RightMouseDown { .. } => "RightMouseDown".to_string(),
        Event::MiddleMouseDown { .. } => "MiddleMouseDown".to_string(),
        Event::MouseMoved { .. } => "MouseMoved".to_string(),
        Event::ScrollWheel { .. } => "ScrollWheel".to_string(),
        Event::ModifierStateChanged { .. } => "ModifierStateChanged".to_string(),
        Event::ModifierKeyChanged { .. } => "ModifierKeyChanged".to_string(),
        Event::DragAndDropFiles { .. } => "DragAndDropFiles".to_string(),
        Event::DragFiles { .. } => "DragFiles".to_string(),
        Event::DragFileExit => "DragFileExit".to_string(),
        Event::SetMarkedText { .. } => "SetMarkedText".to_string(),
        Event::ClearMarkedText => "ClearMarkedText".to_string(),
        Event::ForwardMouseDown { .. } => "ForwardMouseDown".to_string(),
        Event::BackMouseDown { .. } => "BackMouseDown".to_string(),
    }
}

/// Accumulates timestamped test events during an integration test run.
///
/// When recording is active, `write_to_file` renders each entry with its
/// offset into the recording (e.g. `[+00:03.142]`). If recording was never
/// started the offset is computed relative to the first entry instead so
/// the log is still useful.
#[derive(Default)]
pub struct ActionLog {
    entries: Vec<ActionEntry>,
    recording_start: Option<Instant>,
}

impl ActionLog {
    pub fn new() -> Self {
        Self::default()
    }

    /// Marks the instant at which recording started.  All log entries
    /// will be displayed with offsets relative to this instant.
    pub fn set_recording_start(&mut self, start: Instant) {
        self.recording_start = Some(start);
    }

    /// Appends an entry with the current wall-clock time.
    pub fn record(&mut self, description: impl Into<String>) {
        self.entries.push(ActionEntry {
            recorded_at: Instant::now(),
            description: description.into(),
        });
    }

    /// Writes the action log to a plain-text file.
    ///
    /// Each line has the form:
    /// ```text
    /// [+MM:SS.mmm] description
    /// ```
    /// The offset is relative to `recording_start` (or to the first entry's
    /// timestamp if recording was never explicitly started).
    pub fn write_to_file(&self, path: &Path) -> anyhow::Result<()> {
        if self.entries.is_empty() {
            return Ok(());
        }

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let base = self.recording_start.unwrap_or(self.entries[0].recorded_at);

        let file = std::fs::File::create(path)?;
        let mut writer = std::io::BufWriter::new(file);

        for entry in &self.entries {
            let offset = entry
                .recorded_at
                .checked_duration_since(base)
                .unwrap_or(Duration::ZERO);
            let total_secs = offset.as_secs();
            let minutes = total_secs / 60;
            let seconds = total_secs % 60;
            let millis = offset.subsec_millis();
            writeln!(
                writer,
                "[+{minutes:02}:{seconds:02}.{millis:03}] {}",
                entry.description
            )?;
        }

        log::info!(
            "ActionLog: wrote {} entries to {}",
            self.entries.len(),
            path.display()
        );
        Ok(())
    }
}

/// Helper to retrieve a mutable reference to the log from a `StepDataMap`.
pub fn get_action_log_mut(step_data_map: &mut super::step::StepDataMap) -> Option<&mut ActionLog> {
    step_data_map.get_mut::<_, ActionLog>(ACTION_LOG_KEY)
}

/// Helper to retrieve a shared reference to the log from a `StepDataMap`.
pub fn get_action_log(step_data_map: &super::step::StepDataMap) -> Option<&ActionLog> {
    step_data_map.get::<_, ActionLog>(ACTION_LOG_KEY)
}
