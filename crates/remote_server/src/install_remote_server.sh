#!/usr/bin/env bash
# Installs the Warp remote server binary on a remote host.
#
# Placeholders (substituted at runtime by setup.rs):
#   {download_base_url}  — e.g. https://app.warp.dev/download/cli
#   {channel}            — stable | preview | dev
#   {install_dir}        — e.g. ~/.warp/remote-server
#   {binary_name}        — e.g. oz | oz-dev | oz-preview
#   {version_query}      — e.g. &version=v0.2026... (empty when no release tag)
#   {version_suffix}     — e.g. -v0.2026...        (empty when no release tag)
set -e

arch=$(uname -m)
case "$arch" in
  x86_64)        arch_name=x86_64 ;;
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
install_dir="${install_dir/#\~/"$HOME"}"
mkdir -p "$install_dir"

tmpdir=$(mktemp -d "$install_dir/.install.XXXXXX")
trap 'rm -rf "$tmpdir"' EXIT

curl -fSL "{download_base_url}?package=tar&os=$os_name&arch=$arch_name&channel={channel}{version_query}" \
  -o "$tmpdir/oz.tar.gz"
tar -xzf "$tmpdir/oz.tar.gz" -C "$tmpdir"

bin=$(find "$tmpdir" -type f -name 'oz*' ! -name '*.tar.gz' | head -n1)
if [ -z "$bin" ]; then echo "no binary found in tarball" >&2; exit 1; fi
chmod +x "$bin"
mv "$bin" "$install_dir/{binary_name}{version_suffix}"
