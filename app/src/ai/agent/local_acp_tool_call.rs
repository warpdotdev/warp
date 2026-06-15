use std::time::Duration;

use crate::ai::agent::{AIAgentText, AIAgentTextSection};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LocalAcpToolCallStatus {
    #[default]
    Pending,
    InProgress,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LocalAcpToolKind {
    Read,
    Edit,
    Delete,
    Move,
    Search,
    Execute,
    Think,
    Fetch,
    SwitchMode,
    #[default]
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalAcpToolCallMessage {
    pub tool_call_id: String,
    pub title: String,
    pub kind: LocalAcpToolKind,
    pub status: LocalAcpToolCallStatus,
    pub body: AIAgentText,
    pub locations: Vec<String>,
}

impl LocalAcpToolCallMessage {
    pub fn body_plain_text(&self) -> String {
        self.body
            .sections
            .iter()
            .filter_map(|section| match section {
                AIAgentTextSection::PlainText { text } => Some(text.text().to_string()),
                _ => None,
            })
            .collect()
    }

    pub fn has_visible_body(&self) -> bool {
        !self.body_plain_text().trim().is_empty()
    }

    pub fn header_text(&self, finished_duration: Option<Duration>) -> String {
        if let Some(duration) = finished_duration {
            return format!("{} ({})", self.title, format_tool_duration(duration));
        }

        match self.status {
            LocalAcpToolCallStatus::Pending | LocalAcpToolCallStatus::InProgress => {
                self.title.clone()
            }
            LocalAcpToolCallStatus::Completed => self.title.clone(),
            LocalAcpToolCallStatus::Failed => format!("{} (failed)", self.title),
        }
    }
}

fn format_tool_duration(duration: Duration) -> String {
    let seconds = duration.as_secs();
    if seconds < 60 {
        format!("{seconds}s")
    } else {
        format!("{}m {}s", seconds / 60, seconds % 60)
    }
}
