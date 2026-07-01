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

# Stage the download + extraction next to the final install dir so an existing,
# working install is only touched once the new build has been fully downloaded,
# extracted, and validated. Staging on the same filesystem as $INSTALL_DIR keeps
# the final swap a quick rename. Mirrors the Oz CLI installer.
PARENT_DIR="$(dirname "$INSTALL_DIR")"
mkdir -p "$PARENT_DIR"
STAGING_DIR="$(mktemp -d "$PARENT_DIR/.warp-tui-install.XXXXXX")"
trap 'rm -rf "$STAGING_DIR"' EXIT

echo "Downloading Warp TUI ($CHANNEL, macos/$ARCH)..."
if ! curl -fSL "$URL" -o "$STAGING_DIR/warp-tui.tar.gz"; then
    err "failed to download the Warp TUI. The dev channel is internal-only; ensure you have access."
fi

echo "Unpacking..."
# The tarball contains the renamed binary (`warp-tui-<channel>`) plus a sibling
# `resources/` tree. The binary resolves `resources/` relative to its own
# location at runtime, so they must be installed together.
PAYLOAD_DIR="$STAGING_DIR/payload"
mkdir -p "$PAYLOAD_DIR"
tar xzf "$STAGING_DIR/warp-tui.tar.gz" -C "$PAYLOAD_DIR"

# Validate the payload before replacing any existing install.
if [[ ! -f "$PAYLOAD_DIR/$BINARY_NAME" ]]; then
    err "downloaded archive did not contain expected binary '$BINARY_NAME'."
fi
if [[ ! -d "$PAYLOAD_DIR/resources" ]]; then
    err "downloaded archive did not contain the expected 'resources/' directory."
fi
chmod +x "$PAYLOAD_DIR/$BINARY_NAME"

# Standalone (non-app-bundle) binaries can't have a notarization ticket stapled,
# so clear any Gatekeeper quarantine attribute to avoid a first-run prompt.
xattr -dr com.apple.quarantine "$PAYLOAD_DIR" 2>/dev/null || true

# Swap the validated payload into place. Both moves are same-filesystem renames,
# and the previous install is only removed after the new one is in place; if the
# swap fails, the previous install is restored.
echo "Installing to $INSTALL_DIR..."
rm -rf "$INSTALL_DIR.old"
if [[ -e "$INSTALL_DIR" ]]; then
    mv "$INSTALL_DIR" "$INSTALL_DIR.old"
fi
if ! mv "$PAYLOAD_DIR" "$INSTALL_DIR"; then
    if [[ -e "$INSTALL_DIR.old" ]]; then
        mv "$INSTALL_DIR.old" "$INSTALL_DIR"
    fi
    err "failed to install the Warp TUI into $INSTALL_DIR."
fi
rm -rf "$INSTALL_DIR.old"

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
