/// Check that a CLI command contains a sequence of tokens in order.
/// Useful to ignore flags that may be present between tokens.
pub fn command_contains_sequence(cmd: &str, sequence: &[&str]) -> bool {
    let mut required_index = 0;

    for token in cmd.split_whitespace() {
        if token == sequence[required_index] {
            required_index += 1;
            if required_index == sequence.len() {
                return true;
            }
        }
    }

    false
}

pub fn is_running_in_docker() -> bool {
    std::path::Path::new("/.dockerenv").exists()
}

pub fn get_base_server_url() -> String {
    if is_running_in_docker() {
        "http://host.docker.internal:8080".to_string()
    } else {
        "http://localhost:8080".to_string()
    }
}
