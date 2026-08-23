use clap::Args;

/// Common args for scoping objects to team or personal drives.
#[derive(Args, Debug, Clone)]
#[group(required = false, multiple = false)]
pub struct ObjectScope {
    /// Create at the team level.
    #[arg(long, group = "scope")]
    pub team: bool,
    /// Create as private to your account.
    #[arg(long, conflicts_with = "team", group = "scope")]
    pub personal: bool,
}

/// Scoping args for headless team-inclusive collection queries and mutations (e.g. `oz secret`,
/// `oz api-key list`): a UID-valued `--team` naming a specific team, or `--personal`. Distinct
/// from [`ObjectScope`]'s boolean `--team`, which selects a team by inference rather than by
/// naming one explicitly.
#[derive(Args, Debug, Clone)]
#[group(required = false, multiple = false)]
pub struct TeamScopeArgs {
    /// Scope the operation to this team. Required when you belong to more than one team;
    /// optional (and must name a team you belong to) otherwise.
    #[arg(long, value_name = "TEAM_UID")]
    pub team: Option<String>,
    /// Scope the operation to your personal resources, ignoring team membership.
    #[arg(long, conflicts_with = "team")]
    pub personal: bool,
}
