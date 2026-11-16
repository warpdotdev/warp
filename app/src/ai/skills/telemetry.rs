use ai::skills::{SkillProvider, SkillReference, SkillScope};
use serde::{Deserialize, Serialize};
use serde_json::json;
use strum_macros::{EnumDiscriminants, EnumIter};
use warp_core::telemetry::{EnablementState, TelemetryEvent, TelemetryEventDesc};

use crate::features::FeatureFlag;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum SkillOpenOrigin {
    // 'Open skill' button on ReadSkill tool call result
    ReadSkill,
    // 'Open skill' button on ReadFiles tool call result
    ReadFiles,
    // 'Open skill' button on CodeDiffView
    EditFiles,
    // /open-skill command
    OpenSkillCommand,
}

/// Telemetry events for skills
#[derive(Serialize, Debug, EnumDiscriminants)]
#[strum_discriminants(derive(EnumIter))]
pub enum SkillTelemetryEvent {
    /// Emitted when a skill is invoked via the ReadSkill tool call.
    Read {
        /// Specifies the unique reference to the skill, which is either a path or a bundled skill ID
        reference: SkillReference,
        /// Specifies the parsed skill name from SKILL.md front matter, if the reference resolved
        name: Option<String>,
        /// Specifies the scope of the skill (home or project)
        scope: Option<SkillScope>,
        /// Specifies the provider of the skill (Warp, Claude, Codex, etc.)
        provider: Option<SkillProvider>,
        /// Whether the ReadSkill lookup failed (reference could not be resolved)
        error: bool,
    },
    /// Emitted when a SKILL.md file is opened from an 'open skill' button or /edit-skill command.
    Opened {
        /// Specifies the unique reference to the skill, which is either a path or a bundled skill ID
        reference: SkillReference,
        /// Specifies the parsed skill name from SKILL.md front matter, if the reference resolved
        name: Option<String>,
        /// Specifies the origin of the skill open (button or command)
        origin: SkillOpenOrigin,
    },
}

impl TelemetryEvent for SkillTelemetryEvent {
    fn name(&self) -> &'static str {
        SkillTelemetryEventDiscriminants::from(self).name()
    }

    fn payload(&self) -> Option<serde_json::Value> {
        match self {
            SkillTelemetryEvent::Read {
                reference,
                name,
                scope,
                provider,
                error,
            } => Some(json!({
                "reference": reference,
                "name": name,
                "scope": scope,
                "provider": provider,
                "error": error,
            })),
            SkillTelemetryEvent::Opened {
                reference,
                name,
                origin,
            } => Some(json!({
                "reference": reference,
                "name": name,
                "origin": origin,
            })),
        }
    }

    fn description(&self) -> &'static str {
        SkillTelemetryEventDiscriminants::from(self).description()
    }

    fn enablement_state(&self) -> EnablementState {
        SkillTelemetryEventDiscriminants::from(self).enablement_state()
    }

    fn contains_ugc(&self) -> bool {
        // context: Skills are created by users, and the events we emit contain the `name` field,
        // which is defined by the user.
        true
    }

    fn event_descs() -> impl Iterator<Item = Box<dyn TelemetryEventDesc>> {
        warp_core::telemetry::enum_events::<Self>()
    }
}

impl TelemetryEventDesc for SkillTelemetryEventDiscriminants {
    fn name(&self) -> &'static str {
        match self {
            Self::Read => "Skill.Read",
            Self::Opened => "Skill.Opened",
        }
    }

    fn description(&self) -> &'static str {
        match self {
            Self::Read => "A skill was read via the ReadSkill tool call",
            Self::Opened => "A skill was opened from an 'open skill' button or /edit-skill command",
        }
    }

    fn enablement_state(&self) -> EnablementState {
        EnablementState::Flag(FeatureFlag::ListSkills)
    }
}

warp_core::register_telemetry_event!(SkillTelemetryEvent);
