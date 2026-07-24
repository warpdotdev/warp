# Profile: connection refused — nothing listening on the download port.
# curl/wget should fail with "Connection refused".
PROFILE_USER="harness_refused"
PROFILE_EXPECTED="curl/wget connection refused; exit non-zero"

profile_setup() {
    :
}

profile_run() {
    local script="$1"
    # Point at a port where nothing is listening
    local modified_script
    modified_script=$(echo "$script" | sed 's|http://127.0.0.1:[0-9]*/download/cli|http://127.0.0.1:19999/download/cli|g')
    ssh_run_script "$PROFILE_USER" "$modified_script"
}

profile_teardown() {
    :
}
