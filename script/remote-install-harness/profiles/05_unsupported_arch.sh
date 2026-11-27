# Profile: unsupported architecture via a mocked uname.
# The install script should exit 2 with "unsupported arch: mips64".
PROFILE_USER="harness_badarch"
PROFILE_EXPECTED="Exit 2; 'unsupported arch: mips64'"

profile_setup() {
    local home="/home/$PROFILE_USER"
    mkdir -p "$home/.harness_bin"

    # Create fake uname that returns mips64
    cat > "$home/.harness_bin/uname" <<'FEOF'
#!/bin/sh
# Fake uname: return mips64 for -m, Linux for -s
case "$*" in
    *-m*) echo "mips64" ;;
    *-s*) echo "Linux" ;;
    *-sm*|*-ms*) echo "Linux mips64" ;;
    *) echo "Linux mips64" ;;
esac
FEOF
    chmod +x "$home/.harness_bin/uname"

    # Symlink other binaries
    for bin in /usr/bin/*; do
        local name
        name="$(basename "$bin")"
        [ "$name" = "uname" ] && continue
        ln -sf "$bin" "$home/.harness_bin/$name" 2>/dev/null || true
    done
    for bin in /bin/*; do
        [ -f "$bin" ] || continue
        local name
        name="$(basename "$bin")"
        [ "$name" = "uname" ] && continue
        ln -sf "$bin" "$home/.harness_bin/$name" 2>/dev/null || true
    done
    chown -R "$PROFILE_USER:$PROFILE_USER" "$home/.harness_bin"
}

profile_run() {
    local script="$1"
    local home="/home/$PROFILE_USER"
    ssh_run_script "$PROFILE_USER" "export PATH=$home/.harness_bin; $script"
}

profile_teardown() {
    rm -rf "/home/$PROFILE_USER/.harness_bin"
}
