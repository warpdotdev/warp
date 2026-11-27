# Profile: HTTP 403 Forbidden from the download server.
# curl -f should fail with exit 22; wget with a non-zero exit.
PROFILE_USER="harness_403"
PROFILE_EXPECTED="HTTP 403; curl exit 22 or wget error"
PROFILE_SERVER_BEHAVIOR="403"

profile_setup() {
    :
}

profile_teardown() {
    :
}
