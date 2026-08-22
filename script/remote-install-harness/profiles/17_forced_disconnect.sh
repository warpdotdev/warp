# Profile: forced SSH disconnect (exit 255).
# Simulate a host that kills the SSH session mid-install.
PROFILE_USER="harness_disconnect"
PROFILE_EXPECTED="SSH connection killed mid-script; exit 255 from ssh"

profile_setup() {
    local home="/home/$PROFILE_USER"
    # Create a wrapper bash that kills its parent ssh after a brief delay
    mkdir -p "$home/.harness_bin"

    cat > "$home/.harness_bin/force_disconnect.sh" <<'FEOF'
#!/bin/bash
# This script reads from stdin (the install script) but kills the
# SSH connection after 1 second to simulate a forced disconnect.
sleep 1
# Kill the parent process (sshd child handling this session)
kill -9 $PPID 2>/dev/null
exit 255
FEOF
    chmod +x "$home/.harness_bin/force_disconnect.sh"
    chown -R "$PROFILE_USER:$PROFILE_USER" "$home/.harness_bin"
}

profile_run() {
    local script="$1"
    local home="/home/$PROFILE_USER"
    # Instead of running `bash -s`, run the disconnect script
    ssh -p "$SSH_PORT" \
        -i "$CLIENT_KEY" \
        -o StrictHostKeyChecking=no \
        -o UserKnownHostsFile=/dev/null \
        -o ConnectTimeout=5 \
        -o BatchMode=yes \
        -o LogLevel=ERROR \
        -o ServerAliveInterval=1 \
        -o ServerAliveCountMax=2 \
        "$PROFILE_USER@127.0.0.1" \
        "bash $home/.harness_bin/force_disconnect.sh" <<< "$script"
}

profile_teardown() {
    rm -rf "/home/$PROFILE_USER/.harness_bin"
}
