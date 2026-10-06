use std::path::PathBuf;

#[derive(Debug, Clone, clap::Args)]
pub struct EnvironmentCheckoutArgs {
    #[arg(long)]
    pub requests_file: PathBuf,
    #[arg(long)]
    pub failure_report: PathBuf,
    #[arg(long)]
    pub resolved_heads_report: Option<PathBuf>,
    #[arg(long)]
    pub remove_origins_only: bool,
}

#[cfg(test)]
#[path = "environment_checkout_tests.rs"]
mod tests;
