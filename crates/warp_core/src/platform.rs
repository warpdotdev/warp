use line_ending::LineEnding;

#[derive(Debug, Clone)]
pub enum SessionPlatform {
    MSYS2,
    WSL,
    Native,
    /// A shell running inside a Linux Docker sandbox container.
    DockerSandbox,
}

impl SessionPlatform {
    #[allow(clippy::disallowed_methods)]
    pub fn default_line_ending(&self) -> LineEnding {
        match self {
            SessionPlatform::MSYS2 | SessionPlatform::WSL | SessionPlatform::DockerSandbox => {
                LineEnding::LF
            }
            SessionPlatform::Native => LineEnding::from_current_platform(),
        }
    }
}
