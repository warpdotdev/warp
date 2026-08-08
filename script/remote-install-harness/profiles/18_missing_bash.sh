# Profile: bash not available (shell is /bin/sh only).
# The install script is sent via `bash -s` so this simulates
# what happens when bash doesn't exist.
PROFILE_USER="harness_nobash"
PROFILE_EXPECTED="'bash: command not found' or connection failure if shell is not bash"

profile_setup() {
    local home="/home/$PROFILE_USER"
    mkdir -p "$home/.harness_bin"
    # Symlink everything except bash
    for bin in /usr/bin/*; do
        local name
        name="$(basename "$bin")"
        [ "$name" = "bash" ] && continue
        ln -sf "$bin" "$home/.harness_bin/$name" 2>/dev/null || true
    done
    for bin in /bin/*; do
        [ -f "$bin" ] || continue
        local name
        name="$(basename "$bin")"
        [ "$name" = "bash" ] && continue
        ln -sf "$bin" "$home/.harness_bin/$name" 2>/dev/null || true
    done
    chown -R "$PROFILE_USER:$PROFILE_USER" "$home/.harness_bin"
}

profile_run() {
    local script="$1"
    local home="/home/$PROFILE_USER"
    # Run with PATH that doesn't include bash
    ssh -p "$SSH_PORT" \
        -i "$CLIENT_KEY" \
        -o StrictHostKeyChecking=no \
        -o UserKnownHostsFile=/dev/null \
        -o ConnectTimeout=10 \
        -o BatchMode=yes \
        -o LogLevel=ERROR \
        "$PROFILE_USER@127.0.0.1" \
        "export PATH=$home/.harness_bin; bash -s" <<< "$script"
}

profile_teardown() {
    rm -rf "/home/$PROFILE_USER/.harness_bin"
}
