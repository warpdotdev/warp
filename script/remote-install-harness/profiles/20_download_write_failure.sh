# Profile: download write failure — the staging temp dir exists but curl
# cannot write the output file (simulate disk/permission issue mid-download).
# We use a mktemp wrapper that creates a dir owned by root.
PROFILE_USER="harness_writefail"
PROFILE_EXPECTED="curl/wget cannot write output file; 'Permission denied' or similar"

profile_setup() {
    local home="/home/$PROFILE_USER"
    mkdir -p "$home/.harness_bin"

    # Wrapper mktemp: create the dir but make it owned by root so the
    # user can't write into it (but the dir itself exists so `set -e`
    # in the install script won't trap on mktemp itself).
    cat > "$home/.harness_bin/mktemp" <<'FEOF'
#!/bin/bash
real_mktemp=/usr/bin/mktemp
result=$($real_mktemp "$@")
if [ -d "$result" ]; then
    # Transfer ownership to root so the test user can't write
    chown root:root "$result" 2>/dev/null || true
    chmod 555 "$result" 2>/dev/null || true
fi
echo "$result"
FEOF
    chmod +x "$home/.harness_bin/mktemp"
    # suid so the chown call inside works (fallback: test will still
    # demonstrate the scenario because the dir will be non-writable)

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
    # The mktemp wrapper itself needs to run chown as root
    chown root:root "$home/.harness_bin/mktemp"
    chmod 4755 "$home/.harness_bin/mktemp"
}

profile_run() {
    local script="$1"
    local home="/home/$PROFILE_USER"
    ssh_run_script "$PROFILE_USER" "export PATH=$home/.harness_bin:\$PATH; $script"
}

profile_teardown() {
    local home="/home/$PROFILE_USER"
    find "$home/.warp-test" -type d -exec chmod 755 {} \; 2>/dev/null || true
    find "$home/.warp-test" -exec chown "$PROFILE_USER:$PROFILE_USER" {} \; 2>/dev/null || true
    rm -rf "$home/.harness_bin" "$home/.warp-test" 2>/dev/null || true
}
