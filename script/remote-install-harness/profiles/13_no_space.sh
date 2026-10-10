# Profile: no space left on device / disk quota exceeded.
# Simulated by mounting a tiny tmpfs for the install directory.
PROFILE_USER="harness_nospace"
PROFILE_EXPECTED="'No space left on device' during download or extraction"

profile_setup() {
    local home="/home/$PROFILE_USER"
    # Create the install dir and mount a tiny (64K) tmpfs on it
    mkdir -p "$home/.warp-test/remote-server"
    chown -R "$PROFILE_USER:$PROFILE_USER" "$home/.warp-test"
    # Use a 1-block tmpfs so even mktemp will trigger ENOSPC.
    # Fill it with a single file to exhaust the remaining space.
    mount -t tmpfs -o size=4k,uid=$(id -u "$PROFILE_USER"),gid=$(id -g "$PROFILE_USER") tmpfs "$home/.warp-test/remote-server"
    dd if=/dev/zero of="$home/.warp-test/remote-server/.filler" bs=4096 count=1 2>/dev/null || true
}

profile_teardown() {
    local home="/home/$PROFILE_USER"
    umount "$home/.warp-test/remote-server" 2>/dev/null || true
    rm -rf "$home/.warp-test" 2>/dev/null || true
}
