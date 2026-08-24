use clap::Args;

/// Common args for scoping objects to team or personal drives.
///
/// `--team` is optional-valued so it answers both "team or personal?" and "which team?":
/// absent, bare `--team` (your sole team), or `--team=<UID>` when you are on several.
///
/// The uid must be attached with `=`. A detached `--team <UID>` would otherwise swallow the
/// next positional, silently rewriting invocations like `secret delete --team <NAME>` that
/// predate the uid.
#[derive(Args, Debug, Clone)]
#[group(required = false, multiple = false)]
pub struct ObjectScope {
    /// Create at the team level. Pass `--team=<UID>` to choose when you are on more than one.
    #[arg(
        long,
        group = "scope",
        num_args(0..=1),
        require_equals = true,
        value_name = "UID"
    )]
    pub team: Option<Option<String>>,
    /// Create as private to your account.
    #[arg(long, conflicts_with = "team", group = "scope")]
    pub personal: bool,
}

impl ObjectScope {
    /// Whether `--team` was passed, with or without a uid.
    pub fn is_team(&self) -> bool {
        self.team.is_some()
    }

    /// The uid given as `--team=<UID>`, if one was.
    pub fn requested_team_uid(&self) -> Option<&str> {
        self.team.as_ref().and_then(|uid| uid.as_deref())
    }
}
