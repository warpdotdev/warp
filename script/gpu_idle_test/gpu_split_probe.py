#!/usr/bin/env python3
"""
Attribute i915 render-engine busy time to processes via /proc/<pid>/fdinfo
(drm-engine-render, cumulative ns per DRM client). Reports Warp's own GPU
render % vs GNOME Shell's (the compositor) over a window, so we can tell whether
the cost is Warp's rasterization or mutter re-compositing Warp's full surface.

Compare these against the system-wide i915 PMU (gpu_pmu_sampler.py) run over the
same window.

Usage: gpu_split_probe.py <duration_s>
"""
import os, sys, glob, time

DURATION = float(sys.argv[1]) if len(sys.argv) > 1 else 8.0


def is_drm_fd(pid, fd):
    try:
        return "/dev/dri/" in os.readlink(f"/proc/{pid}/fd/{fd}")
    except OSError:
        return False


def proc_render_ns(pid, sample_keys=None):
    """Sum drm-engine-render ns across distinct DRM clients of this pid.
    If sample_keys is a set, also records which drm-engine-* keys were seen."""
    clients = {}
    try:
        fds = os.listdir(f"/proc/{pid}/fd")
    except OSError:
        return 0
    for fd in fds:
        if not is_drm_fd(pid, fd):
            continue
        try:
            txt = open(f"/proc/{pid}/fdinfo/{fd}").read()
        except OSError:
            continue
        cid, render_ns = None, None
        for line in txt.splitlines():
            if line.startswith("drm-client-id:"):
                cid = line.split(":", 1)[1].strip()
            elif line.startswith("drm-engine-") and sample_keys is not None:
                sample_keys.add(line.split(":", 1)[0])
            if line.startswith("drm-engine-render:"):
                parts = line.split(":", 1)[1].split()
                if parts and parts[0].isdigit():
                    render_ns = int(parts[0])
        if render_ns is not None:
            clients[cid if cid is not None else f"{pid}:{fd}"] = render_ns
    return sum(clients.values())


def pids(prefix=None, exact=None):
    out = []
    for d in glob.glob("/proc/[0-9]*"):
        pid = d.split("/")[-1]
        try:
            comm = open(f"{d}/comm").read().strip()
        except OSError:
            continue
        if (prefix and comm.startswith(prefix)) or (exact and comm == exact):
            out.append(int(pid))
    return out


def group_ns(group, sample_keys=None):
    return sum(proc_render_ns(p, sample_keys) for p in group)


def main():
    warp = pids(prefix="warp-oss")
    shell = pids(exact="gnome-shell")
    if not warp:
        print("no warp-oss processes found")
        sys.exit(1)

    keys = set()
    w0 = group_ns(warp, keys)
    s0 = group_ns(shell)
    t0 = time.monotonic()
    time.sleep(DURATION)
    w1 = group_ns(warp)
    s1 = group_ns(shell)
    dt_ns = (time.monotonic() - t0) * 1e9

    wp = (w1 - w0) / dt_ns * 100
    sp = (s1 - s0) / dt_ns * 100
    print(f"window={DURATION:.1f}s   (drm-engine-render via fdinfo)")
    print(f"  Warp        render: {wp:5.1f}%   pids={sorted(warp)}")
    print(f"  GNOME Shell render: {sp:5.1f}%   pids={sorted(shell)}")
    print(f"  Warp + GNOME      : {wp + sp:5.1f}%")
    if w1 - w0 == 0:
        print(f"  [warn] Warp render delta was 0; drm-engine-* keys seen: {sorted(keys) or 'NONE'}")


if __name__ == "__main__":
    main()
