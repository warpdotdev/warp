#[derive(
    Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
pub struct RequestTeamScope(Option<String>);

impl RequestTeamScope {
    pub fn new(team_uid: Option<String>) -> Self {
        Self(team_uid)
    }

    pub fn is_unscoped(&self) -> bool {
        self.0.is_none()
    }

    pub fn team_uid(&self) -> Option<&str> {
        self.0.as_deref()
    }
}

impl settings_value::SettingsValue for RequestTeamScope {}

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
