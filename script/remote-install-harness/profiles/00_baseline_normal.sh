# Profile: baseline normal install — everything works.
# This profile verifies the harness itself: a clean user with all tools
# and a working download server should complete the install successfully.
PROFILE_USER="harness_baseline"
PROFILE_EXPECTED="Successful install; exit 0"

profile_setup() {
    :  # No special setup needed
}

profile_teardown() {
    rm -rf "/home/$PROFILE_USER/.warp-test" 2>/dev/null || true
}
