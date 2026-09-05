# Profile: TLS/CA certificate verification failure.
# Point curl at an HTTPS URL but with an invalid CA bundle so
# certificate verification fails.
PROFILE_USER="harness_tls"
PROFILE_EXPECTED="curl/wget TLS error; 'certificate verify failed' or similar"

profile_setup() {
    local home="/home/$PROFILE_USER"
    # Create an empty CA bundle so TLS verification always fails
    mkdir -p "$home/.harness_ssl"
    touch "$home/.harness_ssl/empty_ca.crt"
    chown -R "$PROFILE_USER:$PROFILE_USER" "$home/.harness_ssl"
}

profile_run() {
    local script="$1"
    local home="/home/$PROFILE_USER"
    # Replace the download URL with an HTTPS endpoint (use a real domain
    # that exists but will fail CA verification with our empty bundle)
    local modified_script
    modified_script=$(echo "$script" | sed 's|http://127.0.0.1:[0-9]*/download/cli|https://app.warp.dev/download/cli|g')
    # Force curl to use our empty CA bundle
    ssh_run_script "$PROFILE_USER" "export CURL_CA_BUNDLE=$home/.harness_ssl/empty_ca.crt; export SSL_CERT_FILE=$home/.harness_ssl/empty_ca.crt; $modified_script"
}

profile_teardown() {
    rm -rf "/home/$PROFILE_USER/.harness_ssl"
}
