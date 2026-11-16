/// The Docker engine creates this file at the root of every container.
const DOCKER_ENV_FILE: &str = "/.dockerenv";

/// Detect whether we are running inside a Docker container.
///
/// The Docker runtime places a `/.dockerenv` marker file in the root filesystem of
/// every container it creates. This is the standard heuristic used to detect Docker.
pub fn is_in_docker() -> bool {
    std::fs::exists(DOCKER_ENV_FILE).unwrap_or(false)
}
