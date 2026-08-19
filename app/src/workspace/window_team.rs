use std::sync::{Arc, OnceLock};

use crate::server::ids::ServerId;

#[derive(Clone, Default)]
pub struct WindowTeam {
    team_uid: Arc<OnceLock<Option<ServerId>>>,
}

impl WindowTeam {
    pub fn pending() -> Self {
        Self::default()
    }

    pub fn assigned(team_uid: Option<ServerId>) -> Self {
        Self {
            team_uid: Arc::new(OnceLock::from(team_uid)),
        }
    }

    pub fn initialize(&self, team_uid: Option<ServerId>) {
        let _ = self.team_uid.set(team_uid);
    }

    pub fn uid(&self) -> Option<ServerId> {
        self.team_uid.get().copied().flatten()
    }

    pub fn is_initialized(&self) -> bool {
        self.team_uid.get().is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn team_assignment_is_immutable() {
        let window_team = WindowTeam::pending();
        let first_team_uid = 123.into();
        let second_team_uid = 456.into();

        window_team.initialize(Some(first_team_uid));
        window_team.initialize(Some(second_team_uid));

        assert_eq!(window_team.uid(), Some(first_team_uid));
    }

    #[test]
    fn personal_assignment_is_immutable() {
        let window_team = WindowTeam::pending();

        window_team.initialize(None);
        window_team.initialize(Some(123.into()));

        assert!(window_team.is_initialized());
        assert_eq!(window_team.uid(), None);
    }
}
