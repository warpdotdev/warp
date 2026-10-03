# Profile: HTTP 502 Bad Gateway from the download server.
# curl -f should fail with exit 22; wget with a non-zero exit.
PROFILE_USER="harness_502"
PROFILE_EXPECTED="HTTP 502; curl exit 22 or wget error"
PROFILE_SERVER_BEHAVIOR="502"

profile_setup() {
    :
}

profile_teardown() {
    :
}
