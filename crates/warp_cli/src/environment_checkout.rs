use std::path::PathBuf;

#[derive(Debug, Clone, clap::Args)]
pub struct EnvironmentCheckoutArgs {
    #[arg(long)]
    pub requests_file: PathBuf,
    /// Where to write the per-checkout report describing how the batch went.
    #[arg(long)]
    pub report_file: PathBuf,
    #[arg(long)]
    pub remove_origins_only: bool,
    #[arg(long, conflicts_with = "remove_origins_only")]
    pub fail_if_target_exists: bool,
}

#[cfg(test)]
#[path = "environment_checkout_tests.rs"]
mod tests;
