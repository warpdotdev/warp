#!/usr/bin/env bash
# Installs the Warp remote server on a remote host.
#
# Install layout:
#
#   {install_dir}/
#   ├── bundles/{bundle_version}/      ← the artifact's shipped layout, unchanged:
#   │   ├── {binary_name}                 the executable...
#   │   └── resources/                    ...and its sibling resources
#   └── {binary_name}{version_suffix}  ── symlink ─▶ bundles/{bundle_version}/{binary_name}
#
# The bundle preserves the executable-beside-resources pairing exactly as the
# release artifact ships it, because the daemon locates its resources by
# canonicalizing its own executable path and reading the sibling `resources/`
# directory. Bundles are version-scoped because the daemon outlives client
# updates: an older daemon may still be running while a newer client installs,
# and a single shared resources directory would swap skills and schema files
# out from under it. The version-suffixed symlink keeps the launch path that
# every client codepath already uses, while the shipped binary keeps its
# unsuffixed name inside the bundle.
#
# Placeholders (substituted at runtime by setup.rs):
#   {download_base_url}         — e.g. https://app.warp.dev/download/cli
#   {channel}                   — stable | preview | dev
#   {install_dir}               — e.g. ~/.warp/remote-server
#   {binary_name}               — e.g. oz | oz-dev | oz-preview
#   {version_query}             — e.g. &version=v0.2026... (empty when no release tag)
#   {version_suffix}            — e.g. -v0.2026...        (empty when no release tag)
#   {bundle_version}            — version-scoped bundle directory name
#   {binary_symlink_target}     — relative target for the compatibility symlink
#   {no_http_client_exit_code}  — exit code when neither curl nor wget is available
#   {staging_tarball_path}      — path to a pre-uploaded tarball (SCP fallback; empty normally)
set -e

arch=$(uname -m)
case "$arch" in
  x86_64|amd64)  arch_name=x86_64 ;;
  aarch64|arm64) arch_name=aarch64 ;;
  *) echo "unsupported arch: $arch" >&2; exit 2 ;;
esac

os_kernel=$(uname -s)
case "$os_kernel" in
  Darwin) os_name=macos ;;
  Linux)  os_name=linux ;;
  *) echo "unsupported OS: $os_kernel" >&2; exit 2 ;;
esac

install_dir="{install_dir}"
# Avoid `${var/pattern/replacement}` for tilde expansion. Two
# interpreter quirks make it dangerous in this script:
#   1. bash 3.2 (macOS /bin/bash) keeps inner double-quotes around the
#      replacement literal, so `"$HOME"` ends up as 6 literal
#      characters and the install lands under a directory tree
#      literally named `"`.
#   2. bash 5.2+ enables `patsub_replacement` by default, which makes
#      `&` in the replacement expand to the matched pattern, so a
#      `$HOME` containing `&` resolves to a `~`-substituted path.
# Use `case` + `${var#\~}` instead — works on bash 3.2 and bash 5.2+
# without surprises.
case "$install_dir" in
  "~"|"~/"*) install_dir="${HOME}${install_dir#\~}" ;;
esac
mkdir -p "$install_dir"

tmpdir=$(mktemp -d "$install_dir/.install.XXXXXX")
# Best-effort cleanup of the staging directory. A failure here (e.g.
# EBUSY or "Directory not empty" races on some filesystems/mounts)
# must not fail the install: by the time this fires the binary has
# either already been moved into its final location, or the script
# has already failed for an unrelated reason that we want to surface
# instead of clobbering with the cleanup's exit code.
cleanup() {
  rm -rf "$tmpdir" 2>/dev/null || true
  if [ -n "${install_bundle_tmp:-}" ]; then
    rm -rf "$install_bundle_tmp" 2>/dev/null || true
  fi
  if [ -n "${symlink_tmp:-}" ]; then
    rm -f "$symlink_tmp" 2>/dev/null || true
  fi
}
trap cleanup EXIT

staging_tarball_path="{staging_tarball_path}"
if [ -n "$staging_tarball_path" ]; then
  # SCP fallback: tarball already uploaded by the client.
  # Same tilde-expansion caveat as install_dir above.
  case "$staging_tarball_path" in
    "~"|"~/"*) staging_tarball_path="${HOME}${staging_tarball_path#\~}" ;;
  esac
  mv "$staging_tarball_path" "$tmpdir/oz.tar.gz"
else
  # Normal path: download via curl or wget.
  url="{download_base_url}?package=tar&os=$os_name&arch=$arch_name&channel={channel}{version_query}"

  if command -v curl >/dev/null 2>&1; then
    curl -fSL --connect-timeout 15 "$url" -o "$tmpdir/oz.tar.gz"
  elif command -v wget >/dev/null 2>&1; then
    wget -q -O "$tmpdir/oz.tar.gz" "$url"
  else
    echo "error: neither curl nor wget is available" >&2
    exit {no_http_client_exit_code}
  fi
fi

tar -xzf "$tmpdir/oz.tar.gz" -C "$tmpdir"

# The executable and its resources are siblings in the artifact. Exclude the
# resources tree from the search: bundled skills may ship companion files
# whose names also start with `oz`.
bin=$(find "$tmpdir" -type f -name 'oz*' ! -name '*.tar.gz' ! -path '*/resources/*' | head -n1)
if [ -z "$bin" ]; then echo "no binary found in tarball" >&2; exit 1; fi
resources="$(dirname "$bin")/resources"
if [ ! -d "$resources" ]; then echo "no resources directory found in tarball" >&2; exit 1; fi

# Assemble the complete bundle in the temp dir first: assembly takes multiple
# moves, so it must happen at a path nothing launches from. The shipped
# executable-beside-resources pairing is preserved as-is.
staged_bundle="$tmpdir/bundle"
mkdir -p "$staged_bundle"
chmod +x "$bin"
mv "$bin" "$staged_bundle/{binary_name}"
mv "$resources" "$staged_bundle/resources"

bundles_dir="$install_dir/bundles"
bundle_dir="$bundles_dir/{bundle_version}"
mkdir -p "$bundles_dir"

# Install the staged bundle with a single directory rename so the bundle
# either exists in full or not at all — an interrupted install must never
# leave a binary without its resources at a launchable path. The temp name is
# dot-prefixed and PID-suffixed so concurrent installs cannot collide.
# Replacing an existing bundle is only needed to repair an incomplete install
# at this version; healthy bundles are skipped by the pre-install binary check.
install_bundle_tmp="$bundles_dir/.{bundle_version}.install.$$"
rm -rf "$install_bundle_tmp"
mv "$staged_bundle" "$install_bundle_tmp"
rm -rf "$bundle_dir"
mv "$install_bundle_tmp" "$bundle_dir"

# Point the historical launch path at the bundle. Clients launch the
# version-suffixed path they have always used, while the shipped binary keeps
# its unsuffixed name beside its resources — the symlink bridges the two, and
# because the daemon canonicalizes through it, the daemon resolves its own
# bundle's resources. The target is relative to keep the install relocatable.
# The link is created at a temp name and renamed into place because `ln -s`
# cannot atomically replace an existing path (including a legacy regular-file
# binary being upgraded to the bundle layout).
binary_path="$install_dir/{binary_name}{version_suffix}"
symlink_tmp="$install_dir/.{binary_name}{version_suffix}.link.$$"
rm -f "$symlink_tmp"
ln -s "{binary_symlink_target}" "$symlink_tmp"
mv -f "$symlink_tmp" "$binary_path"
