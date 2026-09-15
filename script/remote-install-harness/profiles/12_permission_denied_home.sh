# Profile: permission denied on install directory.
# The install dir parent is owned by root and not writable by the user.
PROFILE_USER="harness_noperm"
PROFILE_EXPECTED="mkdir -p fails with 'Permission denied'"

profile_setup() {
    local home="/home/$PROFILE_USER"
    # Create .warp-test owned by root, not writable by user
    mkdir -p "$home/.warp-test"
    chown root:root "$home/.warp-test"
    chmod 755 "$home/.warp-test"
    # The user can read but not write, so mkdir -p .warp-test/remote-server fails
}

profile_teardown() {
    local home="/home/$PROFILE_USER"
    chown "$PROFILE_USER:$PROFILE_USER" "$home/.warp-test" 2>/dev/null || true
    rm -rf "$home/.warp-test" 2>/dev/null || true
}
