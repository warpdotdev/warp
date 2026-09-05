# Profile: read-only home directory.
# mkdir -p for the install dir should fail.
PROFILE_USER="harness_rohome"
PROFILE_EXPECTED="mkdir failure; 'Read-only file system' or 'Permission denied'"

profile_setup() {
    local home="/home/$PROFILE_USER"
    # Create the .warp-test dir first, then make it read-only
    mkdir -p "$home/.warp-test"
    chown "$PROFILE_USER:$PROFILE_USER" "$home/.warp-test"
    chmod 555 "$home/.warp-test"
    # Also make home read-only to prevent mkdir -p from creating .warp-test/remote-server
    # We keep .ssh writable for auth
}

profile_teardown() {
    local home="/home/$PROFILE_USER"
    chmod 755 "$home/.warp-test" 2>/dev/null || true
    rm -rf "$home/.warp-test" 2>/dev/null || true
}
