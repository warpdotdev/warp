#!/bin/bash
#
# Warp TUI installer.
#
# Downloads the latest Warp TUI build for the current macOS architecture and
# installs it as `warp-tui` on your PATH. Mirrors the one-line install UX of
# other terminal agents (e.g. Claude Code, Codex), e.g.:
#
#     curl -fsSL https://app.warp.dev/download/tui/install.sh | bash
#
# The Warp TUI is currently a DEV-ONLY, macOS-only artifact. The `dev` channel
# is served only to internal/dogfood builds of the download service, so this
# installer is intended for internal use.
#
# Environment overrides:
#   WARP_TUI_CHANNEL       Channel to install (default: dev).
#   WARP_TUI_DOWNLOAD_URL  Download endpoint (default: https://app.warp.dev/download/tui).
#   WARP_TUI_INSTALL_DIR   Where the binary + resources are unpacked
#                          (default: ~/.warp/tui).
#   WARP_TUI_BIN_DIR       Directory for the `warp-tui` symlink on PATH
#                          (default: ~/.local/bin).

set -euo pipefail

CHANNEL="${WARP_TUI_CHANNEL:-dev}"
DOWNLOAD_URL="${WARP_TUI_DOWNLOAD_URL:-https://app.warp.dev/download/tui}"
INSTALL_DIR="${WARP_TUI_INSTALL_DIR:-$HOME/.warp/tui}"
BIN_DIR="${WARP_TUI_BIN_DIR:-$HOME/.local/bin}"

err() {
    echo "error: $*" >&2
    exit 1
}

# The TUI is macOS-only for now.
if [[ "$(uname -s)" != "Darwin" ]]; then
    err "the Warp TUI is currently only supported on macOS."
fi

# Map the machine architecture to the release naming (aarch64 / x86_64).
case "$(uname -m)" in
    arm64 | aarch64)
        ARCH="aarch64"
        ;;
    x86_64)
        ARCH="x86_64"
        ;;
    *)
        err "unsupported architecture: $(uname -m) (expected arm64 or x86_64)."
        ;;
esac

BINARY_NAME="warp-tui-$CHANNEL"
URL="$DOWNLOAD_URL?os=macos&arch=$ARCH&channel=$CHANNEL"

TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT

echo "Downloading Warp TUI ($CHANNEL, macos/$ARCH)..."
if ! curl -fSL "$URL" -o "$TMP_DIR/warp-tui.tar.gz"; then
    err "failed to download the Warp TUI. The dev channel is internal-only; ensure you have access."
fi

echo "Unpacking to $INSTALL_DIR..."
# The tarball contains the renamed binary (`warp-tui-<channel>`) plus a sibling
# `resources/` tree. The binary resolves `resources/` relative to its own
# location at runtime, so they must be installed together.
rm -rf "$INSTALL_DIR"
mkdir -p "$INSTALL_DIR"
tar xzf "$TMP_DIR/warp-tui.tar.gz" -C "$INSTALL_DIR"

if [[ ! -f "$INSTALL_DIR/$BINARY_NAME" ]]; then
    err "downloaded archive did not contain expected binary '$BINARY_NAME'."
fi
chmod +x "$INSTALL_DIR/$BINARY_NAME"

# Standalone (non-app-bundle) binaries can't have a notarization ticket stapled,
# so clear any Gatekeeper quarantine attribute to avoid a first-run prompt.
xattr -dr com.apple.quarantine "$INSTALL_DIR" 2>/dev/null || true

echo "Linking $BIN_DIR/warp-tui..."
mkdir -p "$BIN_DIR"
ln -sf "$INSTALL_DIR/$BINARY_NAME" "$BIN_DIR/warp-tui"

echo ""
echo "Warp TUI installed to $INSTALL_DIR"
echo "Run it with: warp-tui"

# Nudge the user if the bin dir isn't on PATH yet.
case ":$PATH:" in
    *":$BIN_DIR:"*) ;;
    *)
        echo ""
        echo "note: $BIN_DIR is not on your PATH. Add it, e.g.:"
        echo "    echo 'export PATH=\"$BIN_DIR:\$PATH\"' >> ~/.zshrc && source ~/.zshrc"
        ;;
esac
