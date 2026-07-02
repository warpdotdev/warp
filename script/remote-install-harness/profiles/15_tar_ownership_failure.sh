# Profile: tar extraction fails due to ownership/permission issues.
# The staging temp directory is created but not writable by the user.
PROFILE_USER="harness_tarown"
PROFILE_EXPECTED="tar extraction failure; 'Cannot open' or 'Permission denied'"

profile_setup() {
    local home="/home/$PROFILE_USER"
    # Pre-create the install dir so mkdir -p succeeds, but make the
    # .install.XXXXXX temp dir creation succeed, then immediately revoke
    # write permission. We intercept mktemp to create a non-writable dir.
    mkdir -p "$home/.harness_bin"

    # Create a wrapper mktemp that creates a dir but makes it non-writable
    cat > "$home/.harness_bin/mktemp" <<'FEOF'
#!/bin/bash
# Create a temp dir that's not writable by the caller
real_mktemp=/usr/bin/mktemp
result=$($real_mktemp "$@")
if [ -d "$result" ]; then
    chmod 555 "$result"
fi
echo "$result"
FEOF
    chmod +x "$home/.harness_bin/mktemp"

    # Symlink all other binaries
    for bin in /usr/bin/*; do
        local name
        name="$(basename "$bin")"
        [ "$name" = "mktemp" ] && continue
        ln -sf "$bin" "$home/.harness_bin/$name" 2>/dev/null || true
    done
    for bin in /bin/*; do
        [ -f "$bin" ] || continue
        local name
        name="$(basename "$bin")"
        [ "$name" = "mktemp" ] && continue
        ln -sf "$bin" "$home/.harness_bin/$name" 2>/dev/null || true
    done
    chown -R "$PROFILE_USER:$PROFILE_USER" "$home/.harness_bin"
}

profile_run() {
    local script="$1"
    local home="/home/$PROFILE_USER"
    ssh_run_script "$PROFILE_USER" "export PATH=$home/.harness_bin:\$PATH; $script"
}

profile_teardown() {
    local home="/home/$PROFILE_USER"
    # Fix permissions for cleanup
    find "$home/.warp-test" -type d -exec chmod 755 {} \; 2>/dev/null || true
    rm -rf "$home/.harness_bin" "$home/.warp-test" 2>/dev/null || true
}
