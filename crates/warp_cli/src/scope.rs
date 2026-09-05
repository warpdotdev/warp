use clap::Args;

/// Common args for selecting a team.
///
/// `--team` is optional-valued: absent, bare `--team` (your sole team), or
/// `--team=<UID>` when you are on several.
///
/// The uid must be attached with `=`. A detached `--team <UID>` would otherwise swallow the
/// next positional, silently rewriting invocations like `secret delete --team <NAME>` that
/// predate the uid.
#[derive(Args, Debug, Clone)]
pub struct TeamSelection {
    /// Scope the command to a team. Pass `--team=<UID>` to choose when you are on more than one.
    #[arg(
        long,
        group = "scope",
        num_args(0..=1),
        require_equals = true,
        value_name = "UID"
    )]
    pub team: Option<Option<String>>,
}

impl TeamSelection {
    /// Whether `--team` was passed, with or without a uid.
    pub fn is_team(&self) -> bool {
        self.team.is_some()
    }

    /// The uid given as `--team=<UID>`, if one was.
    pub fn requested_team_uid(&self) -> Option<&str> {
        self.team.as_ref().and_then(|uid| uid.as_deref())
    }
}

/// Common args for scoping objects to team or personal drives.
#[derive(Args, Debug, Clone)]
#[group(required = false, multiple = false)]
pub struct ObjectScope {
    #[command(flatten)]
    pub team_selection: TeamSelection,
    /// Create as private to your account.
    #[arg(long, conflicts_with = "team", group = "scope")]
    pub personal: bool,
}

impl ObjectScope {
    /// Whether `--team` was passed, with or without a uid.
    pub fn is_team(&self) -> bool {
        self.team_selection.is_team()
    }

    /// The uid given as `--team=<UID>`, if one was.
    pub fn requested_team_uid(&self) -> Option<&str> {
        self.team_selection.requested_team_uid()
    }
}
