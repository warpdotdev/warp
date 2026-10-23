# Profile: read-only filesystem mount for the install directory.
# Simulated by mounting a read-only tmpfs.
PROFILE_USER="harness_rofs"
PROFILE_EXPECTED="'Read-only file system' during mkdir or write"

profile_setup() {
    local home="/home/$PROFILE_USER"
    mkdir -p "$home/.warp-test/remote-server"
    chown -R "$PROFILE_USER:$PROFILE_USER" "$home/.warp-test"
    mount -t tmpfs -o ro,uid=$(id -u "$PROFILE_USER"),gid=$(id -g "$PROFILE_USER") tmpfs "$home/.warp-test/remote-server"
}

profile_teardown() {
    local home="/home/$PROFILE_USER"
    umount "$home/.warp-test/remote-server" 2>/dev/null || true
    rm -rf "$home/.warp-test" 2>/dev/null || true
}
