# Profile: partial download — server sends truncated tarball.
# tar extraction should fail because the file is corrupted/incomplete.
PROFILE_USER="harness_partial"
PROFILE_EXPECTED="tar extraction failure on corrupt/truncated tarball"
PROFILE_SERVER_BEHAVIOR="partial"

profile_setup() {
    :
}

profile_teardown() {
    :
}
