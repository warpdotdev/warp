# Profile: missing wget — only curl is available on the remote host.
# The install script should use curl and succeed.
PROFILE_USER="harness_nowget"
PROFILE_EXPECTED="Script uses curl; should succeed"

profile_setup() {
    local home="/home/$PROFILE_USER"
    mkdir -p "$home/.harness_bin"
    for bin in /usr/bin/*; do
        local name
        name="$(basename "$bin")"
        [ "$name" = "wget" ] && continue
        ln -sf "$bin" "$home/.harness_bin/$name" 2>/dev/null || true
    done
    for bin in /bin/* /usr/sbin/*; do
        [ -f "$bin" ] || continue
        local name
        name="$(basename "$bin")"
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
