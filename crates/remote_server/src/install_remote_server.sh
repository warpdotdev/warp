#!/usr/bin/env bash
# Installs the Warp remote server binary on a remote host, plus the
# artifact's `resources/` tree (bundled skills, settings schema) at a
# global, version-independent location:
#
#   {install_dir}/
#   ├── {binary_name}{version_suffix}   ← the executable
#   └── bundled_resources/              ← the artifact's resources tree
#
# Resources are deliberately decoupled from the binary version: the last
# install wins. An older daemon that is still running parsed its skills at
# startup, so a slightly newer resources tree underneath it is accepted.
#
# Placeholders (substituted at runtime by setup.rs):
#   {download_base_url}         — e.g. https://app.warp.dev/download/cli
#   {channel}                   — stable | preview | dev
#   {install_dir}               — e.g. ~/.warp/remote-server
#   {binary_name}               — e.g. oz | oz-dev | oz-preview
#   {version_query}             — e.g. &version=v0.2026... (empty when no release tag)
#   {version_suffix}            — e.g. -v0.2026...        (empty when no release tag)
#   {bundled_resources_dir_name} — global resources directory name (e.g. bundled_resources)
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
chmod +x "$bin"

# Install the resources tree at the global, version-independent location
# the daemon reads. `$tmpdir` lives inside `$install_dir`, so the `mv` is a
# same-filesystem rename. Installed before the binary so an interrupted
# install never leaves a new binary without its resources — the binary miss
# re-triggers this script. A tarball without resources is not an error: the
# daemon simply has no bundled skills.
resources="$(dirname "$bin")/resources"
if [ -d "$resources" ]; then
  rm -rf "$install_dir/{bundled_resources_dir_name}"
  mv "$resources" "$install_dir/{bundled_resources_dir_name}"
fi

mv "$bin" "$install_dir/{binary_name}{version_suffix}"
