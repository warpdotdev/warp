#!/usr/bin/env python3
"""
Per-thread CPU sampler for all warp-oss* processes, read straight from /proc
(no perf/pidstat needed). Pair with gpu_pmu_sampler.py to decide whether the
render pipeline is CPU-bound (a thread pegged ~100% while GPU has headroom) or
GPU-bound (render engine near 100%).

CPU% is per-core: 100% = one full core. Thread comm names hint at the role
(main/render/tokio worker/pty reader).

Usage: cpu_thread_profile.py <duration_s>
"""
import os, sys, glob, time

DURATION = float(sys.argv[1]) if len(sys.argv) > 1 else 10.0
CLK = os.sysconf("SC_CLK_TCK")
NCPU = os.cpu_count() or 1


def warp_pids():
    pids = []
    for d in glob.glob("/proc/[0-9]*"):
        try:
            comm = open(f"{d}/comm").read().strip()
        except OSError:
            continue
        if comm.startswith("warp-oss"):
            pids.append(int(d.split("/")[-1]))
    return pids


def snapshot(pids):
    """{(pid,tid): (name, jiffies)} across all threads of all pids."""
    snap = {}
    for pid in pids:
        for t in glob.glob(f"/proc/{pid}/task/*"):
            tid = t.split("/")[-1]
            try:
                name = open(f"{t}/comm").read().strip()
                st = open(f"{t}/stat").read()
                after = st[st.rfind(")") + 2:].split()
                utime, stime = int(after[11]), int(after[12])  # fields 14,15
            except (OSError, IndexError, ValueError):
                continue
            snap[(pid, tid)] = (name, utime + stime)
    return snap


def main():
    pids = warp_pids()
    if not pids:
        print("no warp-oss processes found")
        sys.exit(1)
    print(f"profiling {len(pids)} warp-oss pid(s): {sorted(pids)}  "
          f"({NCPU} cores; 100% = one full core)\n")
    s0 = snapshot(pids)
    time.sleep(DURATION)
    s1 = snapshot(pids)

    rows = []
    per_pid = {}
    for key, (name, j1) in s1.items():
        if key in s0:
            dj = j1 - s0[key][1]
            pct = dj / (DURATION * CLK) * 100.0
            if pct >= 0.5:
                rows.append((pct, key[0], key[1], name))
                per_pid[key[0]] = per_pid.get(key[0], 0.0) + pct

    rows.sort(reverse=True)
    print("Top threads by CPU% (>=0.5%):")
    print(f"  {'CPU%':>7}  {'pid':>8}  {'tid':>8}  comm")
    for pct, pid, tid, name in rows[:18]:
        print(f"  {pct:>6.1f}%  {pid:>8}  {tid:>8}  {name}")

    print("\nPer-process total CPU%:")
    for pid in sorted(per_pid, key=lambda p: -per_pid[p]):
        print(f"  pid {pid:>8}: {per_pid[pid]:>6.1f}%")
    total = sum(per_pid.values())
    print(f"\n  ALL warp-oss CPU% = {total:.1f}%  "
          f"(= {total/100:.2f} cores of {NCPU})")
    hottest = rows[0] if rows else None
    if hottest:
        print(f"  hottest single thread = {hottest[0]:.1f}% ({hottest[3]})")
        print("  -> near 100% on one thread while GPU has headroom == CPU-bound")


if __name__ == "__main__":
    main()
