#!/usr/bin/env bash
# Remote server install failure harness.
#
# Builds local OpenSSH host profiles that reproduce each CSV failure family,
# runs the Warp install script against each, and captures output.
#
# Usage:
#   ./run_harness.sh [--profile PROFILE_NAME] [--install-script PATH]
#
# Without --profile, runs all profiles.  The default install script is
# the one checked in at crates/remote_server/src/install_remote_server.sh,
# with placeholders substituted for local testing.
set -o pipefail

HARNESS_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$HARNESS_DIR/../.." && pwd)"
RESULTS_DIR="$HARNESS_DIR/results"
PROFILES_DIR="$HARNESS_DIR/profiles"
SSH_PORT=2222
SSHD_CONFIG="$HARNESS_DIR/.harness_sshd_config"
HOST_KEY="$HARNESS_DIR/.harness_host_key"
CLIENT_KEY="$HARNESS_DIR/.harness_client_key"
SSHD_PID_FILE="$HARNESS_DIR/.harness_sshd.pid"
FAKE_DOWNLOAD_PORT=18443
FAKE_DOWNLOAD_PID_FILE="$HARNESS_DIR/.harness_httpd.pid"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
NC='\033[0m'

log_info()  { echo -e "${CYAN}[harness]${NC} $*"; }
log_ok()    { echo -e "${GREEN}[  OK  ]${NC} $*"; }
log_fail()  { echo -e "${RED}[ FAIL ]${NC} $*"; }
log_warn()  { echo -e "${YELLOW}[ WARN ]${NC} $*"; }

# ---------------------------------------------------------------------------
# Cleanup
# ---------------------------------------------------------------------------
cleanup() {
    log_info "Cleaning up..."
    if [ -f "$SSHD_PID_FILE" ]; then
        kill "$(cat "$SSHD_PID_FILE")" 2>/dev/null || true
        rm -f "$SSHD_PID_FILE"
    fi
    if [ -f "$FAKE_DOWNLOAD_PID_FILE" ]; then
        kill "$(cat "$FAKE_DOWNLOAD_PID_FILE")" 2>/dev/null || true
        rm -f "$FAKE_DOWNLOAD_PID_FILE"
    fi
}
trap cleanup EXIT

# ---------------------------------------------------------------------------
# Parse args
# ---------------------------------------------------------------------------
SINGLE_PROFILE=""
INSTALL_SCRIPT_PATH=""
while [[ $# -gt 0 ]]; do
    case "$1" in
        --profile)   SINGLE_PROFILE="$2"; shift 2 ;;
        --install-script) INSTALL_SCRIPT_PATH="$2"; shift 2 ;;
        *) echo "Unknown arg: $1" >&2; exit 1 ;;
    esac
done

# ---------------------------------------------------------------------------
# Prepare install script with substituted placeholders
# ---------------------------------------------------------------------------
prepare_install_script() {
    local template="$REPO_ROOT/crates/remote_server/src/install_remote_server.sh"
    if [ -n "$INSTALL_SCRIPT_PATH" ]; then
        template="$INSTALL_SCRIPT_PATH"
    fi
    local install_dir="~/.warp-test/remote-server"
    local binary_name="oz-test"
    local channel="dev"
    local download_base_url="http://127.0.0.1:${FAKE_DOWNLOAD_PORT}/download/cli"

    sed \
        -e "s|{download_base_url}|${download_base_url}|g" \
        -e "s|{channel}|${channel}|g" \
        -e "s|{install_dir}|${install_dir}|g" \
        -e "s|{binary_name}|${binary_name}|g" \
        -e "s|{version_query}||g" \
        -e "s|{version_suffix}||g" \
        -e "s|{no_http_client_exit_code}|3|g" \
        -e "s|{staging_tarball_path}||g" \
        "$template"
}

# ---------------------------------------------------------------------------
# SSH infrastructure
# ---------------------------------------------------------------------------
setup_ssh_keys() {
    if [ ! -f "$HOST_KEY" ]; then
        ssh-keygen -t ed25519 -f "$HOST_KEY" -N "" -q
    fi
    if [ ! -f "$CLIENT_KEY" ]; then
        ssh-keygen -t ed25519 -f "$CLIENT_KEY" -N "" -q
    fi
}

start_sshd() {
    log_info "Starting sshd on port $SSH_PORT..."
    mkdir -p /run/sshd  # Required for privilege separation

    cat > "$SSHD_CONFIG" <<EOF
Port $SSH_PORT
ListenAddress 127.0.0.1
HostKey $HOST_KEY
AuthorizedKeysFile %h/.ssh/authorized_keys
PasswordAuthentication no
ChallengeResponseAuthentication no
PubkeyAuthentication yes
PermitRootLogin no
UsePAM no
Subsystem sftp /usr/lib/openssh/sftp-server
PidFile $SSHD_PID_FILE
LogLevel INFO
AcceptEnv HARNESS_*
StrictModes no
EOF

    /usr/sbin/sshd -f "$SSHD_CONFIG" -E "$HARNESS_DIR/.harness_sshd.log"
    sleep 0.5
    if [ -f "$SSHD_PID_FILE" ]; then
        log_ok "sshd started (PID $(cat "$SSHD_PID_FILE"))"
    else
        log_fail "sshd failed to start. Check $HARNESS_DIR/.harness_sshd.log"
        cat "$HARNESS_DIR/.harness_sshd.log"
        exit 1
    fi
}

# ---------------------------------------------------------------------------
# Fake download server (Python)
# ---------------------------------------------------------------------------
start_fake_download_server() {
    local server_dir="$HARNESS_DIR/.fake_server"
    mkdir -p "$server_dir/download"

    # Create a minimal tarball containing a fake binary
    local tarball_dir
    tarball_dir=$(mktemp -d)
    echo '#!/bin/sh' > "$tarball_dir/oz-test"
    echo 'echo "fake oz binary running"' >> "$tarball_dir/oz-test"
    chmod +x "$tarball_dir/oz-test"
    tar -czf "$server_dir/download/oz.tar.gz" -C "$tarball_dir" oz-test
    rm -rf "$tarball_dir"

    # Start Python HTTP server
    cat > "$server_dir/server.py" <<'PYEOF'
import http.server
import os
import sys
import json
import time
import threading

PORT = int(sys.argv[1])
TARBALL_PATH = os.path.join(os.path.dirname(__file__), "download", "oz.tar.gz")
# Profile-specific behaviors loaded from env or config
BEHAVIOR = os.environ.get("FAKE_SERVER_BEHAVIOR", "normal")

class Handler(http.server.BaseHTTPRequestHandler):
    def log_message(self, format, *args):
        pass  # Suppress logging

    def do_GET(self):
        if "/download/cli" in self.path:
            if BEHAVIOR == "403":
                self.send_error(403, "Forbidden")
                return
            elif BEHAVIOR == "502":
                self.send_error(502, "Bad Gateway")
                return
            elif BEHAVIOR == "partial":
                # Send partial content then close
                with open(TARBALL_PATH, "rb") as f:
                    data = f.read()
                self.send_response(200)
                self.send_header("Content-Length", str(len(data)))
                self.send_header("Content-Type", "application/octet-stream")
                self.end_headers()
                # Send only first 10 bytes
                self.wfile.write(data[:10])
                self.wfile.flush()
                return
            elif BEHAVIOR == "slow":
                # Simulate timeout with very slow delivery
                with open(TARBALL_PATH, "rb") as f:
                    data = f.read()
                self.send_response(200)
                self.send_header("Content-Length", str(len(data)))
                self.send_header("Content-Type", "application/octet-stream")
                self.end_headers()
                for byte in data:
                    self.wfile.write(bytes([byte]))
                    self.wfile.flush()
                    time.sleep(2)  # Very slow
                return
            elif BEHAVIOR == "refuse":
                # Immediately close connection
                self.connection.close()
                return
            else:
                # Normal: serve the tarball
                with open(TARBALL_PATH, "rb") as f:
                    data = f.read()
                self.send_response(200)
                self.send_header("Content-Length", str(len(data)))
                self.send_header("Content-Type", "application/octet-stream")
                self.end_headers()
                self.wfile.write(data)
        else:
            self.send_error(404, "Not Found")

server = http.server.HTTPServer(("0.0.0.0", PORT), Handler)
print(f"Fake server on port {PORT}, behavior={BEHAVIOR}", flush=True)
server.serve_forever()
PYEOF

    FAKE_SERVER_BEHAVIOR="${FAKE_SERVER_BEHAVIOR:-normal}" \
        python3 "$server_dir/server.py" "$FAKE_DOWNLOAD_PORT" &
    echo $! > "$FAKE_DOWNLOAD_PID_FILE"
    sleep 0.5
    log_ok "Fake download server started on port $FAKE_DOWNLOAD_PORT (behavior=${FAKE_SERVER_BEHAVIOR:-normal})"
}

restart_fake_server_with_behavior() {
    local behavior="$1"
    if [ -f "$FAKE_DOWNLOAD_PID_FILE" ]; then
        kill "$(cat "$FAKE_DOWNLOAD_PID_FILE")" 2>/dev/null || true
        sleep 0.3
    fi
    FAKE_SERVER_BEHAVIOR="$behavior" start_fake_download_server
}

# ---------------------------------------------------------------------------
# User account management
# ---------------------------------------------------------------------------
create_test_user() {
    local username="$1"
    local home="/home/$username"
    if id "$username" &>/dev/null; then
        return 0  # Already exists
    fi
    useradd -m -s /bin/bash "$username" 2>/dev/null || true
    # Unlock the account for SSH pubkey auth (useradd creates locked accounts)
    passwd -u "$username" 2>/dev/null || usermod -p '*' "$username" 2>/dev/null || true
    mkdir -p "$home/.ssh"
    cp "${CLIENT_KEY}.pub" "$home/.ssh/authorized_keys"
    chmod 700 "$home/.ssh"
    chmod 600 "$home/.ssh/authorized_keys"
    chown -R "$username:$username" "$home/.ssh"
}

# ---------------------------------------------------------------------------
# SSH helper: run a command as a test user via SSH
# ---------------------------------------------------------------------------
ssh_run() {
    local username="$1"
    shift
    ssh -p "$SSH_PORT" \
        -i "$CLIENT_KEY" \
        -o StrictHostKeyChecking=no \
        -o UserKnownHostsFile=/dev/null \
        -o ConnectTimeout=10 \
        -o BatchMode=yes \
        -o LogLevel=ERROR \
        "$username@127.0.0.1" \
        "$@"
}

ssh_run_script() {
    local username="$1"
    local script="$2"
    ssh -p "$SSH_PORT" \
        -i "$CLIENT_KEY" \
        -o StrictHostKeyChecking=no \
        -o UserKnownHostsFile=/dev/null \
        -o ConnectTimeout=10 \
        -o BatchMode=yes \
        -o LogLevel=ERROR \
        "$username@127.0.0.1" \
        "bash -s" <<< "$script"
}

# ---------------------------------------------------------------------------
# Profile runner
# ---------------------------------------------------------------------------
run_profile() {
    local profile_name="$1"
    local profile_script="$PROFILES_DIR/${profile_name}.sh"

    if [ ! -f "$profile_script" ]; then
        log_warn "Profile script not found: $profile_script"
        return 1
    fi

    log_info "═══════════════════════════════════════════════════════════"
    log_info "Running profile: $profile_name"
    log_info "═══════════════════════════════════════════════════════════"

    local result_file="$RESULTS_DIR/${profile_name}.log"
    mkdir -p "$RESULTS_DIR"

    # Source the profile to get setup/teardown/expected behavior
    # Each profile defines:
    #   profile_setup()    — prepare the user environment
    #   profile_teardown() — cleanup after test
    #   PROFILE_USER       — username for this profile
    #   PROFILE_EXPECTED   — expected outcome description
    #   PROFILE_SERVER_BEHAVIOR — fake server behavior (optional)
    unset -f profile_setup profile_teardown profile_extra_setup profile_run 2>/dev/null
    unset PROFILE_USER PROFILE_EXPECTED PROFILE_SERVER_BEHAVIOR 2>/dev/null

    source "$profile_script"

    # Create user if needed
    if [ -n "$PROFILE_USER" ]; then
        create_test_user "$PROFILE_USER"
    fi

    # Set up server behavior if specified
    if [ -n "$PROFILE_SERVER_BEHAVIOR" ]; then
        restart_fake_server_with_behavior "$PROFILE_SERVER_BEHAVIOR"
    else
        restart_fake_server_with_behavior "normal"
    fi

    # Run profile setup
    if declare -f profile_setup >/dev/null 2>&1; then
        profile_setup
    fi

    # Prepare and run the install script
    local install_script
    install_script="$(prepare_install_script)"

    {
        echo "===== Profile: $profile_name ====="
        echo "Expected: $PROFILE_EXPECTED"
        echo "User: $PROFILE_USER"
        echo "Server behavior: ${PROFILE_SERVER_BEHAVIOR:-normal}"
        echo "Timestamp: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
        echo "===== Install Script Output ====="
    } > "$result_file"

    local exit_code=0
    # Some profiles need custom invocation
    if declare -f profile_run >/dev/null 2>&1; then
        profile_run "$install_script" >> "$result_file" 2>&1
        exit_code=$?
    else
        ssh_run_script "$PROFILE_USER" "$install_script" >> "$result_file" 2>&1
        exit_code=$?
    fi

    {
        echo ""
        echo "===== Exit Code: $exit_code ====="
    } >> "$result_file"

    # Run teardown
    if declare -f profile_teardown >/dev/null 2>&1; then
        profile_teardown
    fi

    # Report
    if [ $exit_code -ne 0 ]; then
        log_ok "Profile '$profile_name' failed as expected (exit $exit_code)"
        echo "RESULT: EXPECTED_FAILURE (exit $exit_code)" >> "$result_file"
    else
        log_warn "Profile '$profile_name' unexpectedly succeeded (exit 0)"
        echo "RESULT: UNEXPECTED_SUCCESS (exit 0)" >> "$result_file"
    fi

    echo ""
    cat "$result_file"
    echo ""
}

# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------
main() {
    log_info "Remote Install Failure Harness"
    log_info "Repo root: $REPO_ROOT"

    setup_ssh_keys
    start_sshd
    start_fake_download_server

    mkdir -p "$RESULTS_DIR"

    if [ -n "$SINGLE_PROFILE" ]; then
        run_profile "$SINGLE_PROFILE"
    else
        # Run all profiles in alphabetical order
        for profile_script in "$PROFILES_DIR"/*.sh; do
            [ -f "$profile_script" ] || continue
            local profile_name
            profile_name="$(basename "$profile_script" .sh)"
            run_profile "$profile_name"
        done
    fi

    # Summary
    echo ""
    log_info "═══════════════════════════════════════════════════════════"
    log_info "Summary"
    log_info "═══════════════════════════════════════════════════════════"
    for result in "$RESULTS_DIR"/*.log; do
        [ -f "$result" ] || continue
        local name result_line
        name="$(basename "$result" .log)"
        result_line="$(grep '^RESULT:' "$result" || echo 'RESULT: NO_RESULT')"
        echo "  $name: $result_line"
    done
}

main
