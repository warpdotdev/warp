# Profile: missing curl — only wget is available on the remote host.
# The install script should fall back to wget and succeed.
PROFILE_USER="harness_nocurl"
PROFILE_EXPECTED="Script falls back to wget; should succeed if wget is present"

profile_setup() {
    local home="/home/$PROFILE_USER"
    # Hide curl by creating a restricted PATH wrapper
    mkdir -p "$home/.harness_bin"
    # Symlink everything from /usr/bin except curl
    for bin in /usr/bin/*; do
        local name
        name="$(basename "$bin")"
        [ "$name" = "curl" ] && continue
        ln -sf "$bin" "$home/.harness_bin/$name" 2>/dev/null || true
    done
    # Also link from /bin and /usr/sbin
    for bin in /bin/* /usr/sbin/*; do
        [ -f "$bin" ] || continue
        local name
        name="$(basename "$bin")"
        [ "$name" = "curl" ] && continue
        ln -sf "$bin" "$home/.harness_bin/$name" 2>/dev/null || true
    done
    chown -R "$PROFILE_USER:$PROFILE_USER" "$home/.harness_bin"
}

profile_run() {
    local script="$1"
    # Run with restricted PATH so curl is not found
    local home="/home/$PROFILE_USER"
    ssh_run_script "$PROFILE_USER" "export PATH=$home/.harness_bin; $script"
}

profile_teardown() {
    rm -rf "/home/$PROFILE_USER/.harness_bin"
}
