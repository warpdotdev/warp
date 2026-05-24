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
