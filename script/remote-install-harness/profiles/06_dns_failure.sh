# Profile: DNS resolution failure.
# Override the download URL to a non-existent domain so curl/wget fails DNS.
PROFILE_USER="harness_dns"
PROFILE_EXPECTED="curl/wget DNS failure; 'Could not resolve host' or similar"

profile_setup() {
    :  # No special setup needed — we override the URL in the script
}

profile_run() {
    local script="$1"
    # Replace the download URL with a non-resolvable host
    local modified_script
    modified_script=$(echo "$script" | sed 's|http://127.0.0.1:[0-9]*/download/cli|http://nonexistent.invalid.warp.test/download/cli|g')
    ssh_run_script "$PROFILE_USER" "$modified_script"
}

profile_teardown() {
    :
}
