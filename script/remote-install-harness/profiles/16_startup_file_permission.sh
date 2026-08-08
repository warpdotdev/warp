# Profile: .bashrc is not readable (permission denied on startup file).
# SSH login may still work but bash startup will emit errors.
# The install script itself should still run since it's piped via `bash -s`.
PROFILE_USER="harness_rcperm"
PROFILE_EXPECTED="Bash startup error from unreadable .bashrc; install may still succeed"

profile_setup() {
    local home="/home/$PROFILE_USER"
    # Create a .bashrc that the user cannot read
    echo 'echo "bashrc loaded"' > "$home/.bashrc"
    chown root:root "$home/.bashrc"
    chmod 000 "$home/.bashrc"
    # Also create .profile with the same issue
    echo 'echo "profile loaded"' > "$home/.profile"
    chown root:root "$home/.profile"
    chmod 000 "$home/.profile"
}

profile_teardown() {
    local home="/home/$PROFILE_USER"
    chmod 644 "$home/.bashrc" 2>/dev/null || true
    chown "$PROFILE_USER:$PROFILE_USER" "$home/.bashrc" 2>/dev/null || true
    chmod 644 "$home/.profile" 2>/dev/null || true
    chown "$PROFILE_USER:$PROFILE_USER" "$home/.profile" 2>/dev/null || true
}
