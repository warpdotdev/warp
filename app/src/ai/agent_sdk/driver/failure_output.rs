use crate::server::telemetry::secret_redaction::redact_secrets_in_string;

const FAILURE_OUTPUT_MAX_BYTES: usize = 4 * 1024;

fn truncate_failure_output(output: &str, truncation_marker: &str) -> String {
    if output.len() <= FAILURE_OUTPUT_MAX_BYTES {
        return output.to_owned();
    }

    let retained_bytes = FAILURE_OUTPUT_MAX_BYTES - truncation_marker.len();
    let prefix_budget = retained_bytes / 2;
    let suffix_budget = retained_bytes - prefix_budget;

    let mut prefix_end = prefix_budget;
    while !output.is_char_boundary(prefix_end) {
        prefix_end -= 1;
    }

    let mut suffix_start = output.len() - suffix_budget;
    while !output.is_char_boundary(suffix_start) {
        suffix_start += 1;
    }

    format!(
        "{}{}{}",
        &output[..prefix_end],
        truncation_marker,
        &output[suffix_start..]
    )
}

pub(super) fn prepare_failure_output(output: &str, truncation_marker: &str) -> String {
    let mut output = output.trim().to_owned();
    // Redact before truncation so splitting a credential cannot hide it from detection.
    redact_secrets_in_string(&mut output);
    truncate_failure_output(&output, truncation_marker)
}

#[cfg(test)]
#[path = "failure_output_tests.rs"]
mod tests;
