# Profile: neither curl nor wget available on the remote host.
# The install script should exit with NO_HTTP_CLIENT_EXIT_CODE (3).
PROFILE_USER="harness_nohttp"
PROFILE_EXPECTED="Exit code 3 (no HTTP client); Rust side triggers SCP fallback"

profile_setup() {
    local home="/home/$PROFILE_USER"
    mkdir -p "$home/.harness_bin"
    for bin in /usr/bin/*; do
        local name
        name="$(basename "$bin")"
        [ "$name" = "curl" ] && continue
        [ "$name" = "wget" ] && continue
        ln -sf "$bin" "$home/.harness_bin/$name" 2>/dev/null || true
    done
    for bin in /bin/* /usr/sbin/*; do
        [ -f "$bin" ] || continue
        local name
        name="$(basename "$bin")"
        [ "$name" = "curl" ] && continue
        [ "$name" = "wget" ] && continue
        ln -sf "$bin" "$home/.harness_bin/$name" 2>/dev/null || true
    done
    chown -R "$PROFILE_USER:$PROFILE_USER" "$home/.harness_bin"
}

profile_run() {
    local script="$1"
    local home="/home/$PROFILE_USER"
    ssh_run_script "$PROFILE_USER" "export PATH=$home/.harness_bin; $script"
}

profile_teardown() {
    rm -rf "/home/$PROFILE_USER/.harness_bin"
}
