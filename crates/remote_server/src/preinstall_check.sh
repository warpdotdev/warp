#!/usr/bin/env bash
# Preinstall check for the Warp remote-server binary.
#
# Emits a structured key=value summary on stdout. Exits 0 on success.
# A non-zero exit indicates a probe-level failure; the client treats
# those as `status=unknown` (fail open).

set -u

# The minimum glibc a *dynamically* linked Linux CLI would require. The
# published `download/cli` Linux artifacts are static ELFs (no PT_INTERP),
# so musl hosts such as Alpine can run them. Keep this floor for the glibc
# probe so an old dynamically linked binary still fails closed.
# Built on Ubuntu 20.04 (see `.github/workflows/create_release.yml`).
required_glibc="2.31"
echo "required_glibc=${required_glibc}"

# 1. Detect libc family and (when glibc) its version.
libc_family="unknown"
libc_version=""

if version=$(getconf GNU_LIBC_VERSION 2>/dev/null); then
    # Output: "glibc 2.31"
    libc_family="glibc"
    libc_version="${version##* }"
elif ldd_out=$(ldd --version 2>&1 | head -n1); then
    case "$ldd_out" in
        *musl*)   libc_family="musl"   ;;
        *uClibc*) libc_family="uclibc" ;;
        *bionic*) libc_family="bionic" ;;
        *)
            v=$(printf '%s\n' "$ldd_out" | grep -oE '[0-9]+\.[0-9]+' | head -n1)
            if [ -n "$v" ]; then
                libc_family="glibc"
                libc_version="$v"
            fi
            ;;
    esac
fi

# Android/bionic has no GNU getconf and usually no recognizable ldd --version
# string, so the probes above leave libc_family unknown and the client would
# fail open. getprop and the Android dynamic linker do not exist on ordinary
# Linux.
if [ "$libc_family" = "unknown" ]; then
    if sdk=$(getprop ro.build.version.sdk 2>/dev/null); then
        case "$sdk" in
            ''|*[!0-9]*) ;;
            *) libc_family="bionic" ;;
        esac
    fi
    if [ "$libc_family" = "unknown" ] \
       && { [ -e /system/bin/linker ] || [ -e /system/bin/linker64 ]; }; then
        libc_family="bionic"
    fi
fi

echo "libc_family=${libc_family}"
[ -n "$libc_version" ] && echo "libc_version=${libc_version}"

# 2. Decide status from the gathered facts.
status="unknown"
reason=""

if [ "$libc_family" = "glibc" ] && [ -n "$libc_version" ]; then
    have_major="${libc_version%%.*}"
    have_minor="${libc_version#*.}"
    have_minor="${have_minor%%.*}"
    req_major="${required_glibc%%.*}"
    req_minor="${required_glibc#*.}"
    if [ "$have_major" -gt "$req_major" ] \
       || { [ "$have_major" -eq "$req_major" ] && [ "$have_minor" -ge "$req_minor" ]; }; then
        status="supported"
    else
        status="unsupported"
        reason="glibc_too_old"
    fi
elif [ "$libc_family" = "musl" ]; then
    # Static Linux CLI artifacts do not need glibc. Rejecting musl blocked Dev
    # Container attach, which has no ControlMaster fallback.
    status="supported"
elif [ "$libc_family" = "bionic" ] || [ "$libc_family" = "uclibc" ]; then
    status="unsupported"
    reason="non_glibc"
fi

echo "status=${status}"
if [ -n "$reason" ]; then
    echo "reason=${reason}"
fi
