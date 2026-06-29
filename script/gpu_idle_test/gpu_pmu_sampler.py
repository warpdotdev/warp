#!/usr/bin/env python3
"""
Whole-GPU utilization sampler via the i915 PMU -- the same source btop and
intel_gpu_top use. Reads per-engine *-busy perf counters (nanoseconds the
engine was busy) and normalizes by PERF_FORMAT_TOTAL_TIME_ENABLED:

    engine_util% = delta(busy_ns) / delta(time_enabled_ns) * 100

This measures the GPU system-wide (pid=-1), so it includes warp's rendering
AND the GNOME compositor compositing warp's frames -- matching btop.

Requires CAP_PERFMON or root on systems with perf_event_paranoid >= 2
(that's why btop ships with cap_perfmon). Run with sudo if it errors EACCES.

Usage: gpu_pmu_sampler.py <duration_s> <interval_ms> <out_csv>
"""
import ctypes
import ctypes.util
import os
import struct
import sys
import time

DURATION = float(sys.argv[1]) if len(sys.argv) > 1 else 20.0
INTERVAL = (float(sys.argv[2]) if len(sys.argv) > 2 else 200.0) / 1000.0
OUT = sys.argv[3] if len(sys.argv) > 3 else "/tmp/gpu_pmu.csv"

PMU = "/sys/bus/event_source/devices/i915"
PERF_FORMAT_TOTAL_TIME_ENABLED = 1 << 0
__NR_perf_event_open = 298  # x86_64

libc = ctypes.CDLL(ctypes.util.find_library("c") or "libc.so.6", use_errno=True)


class perf_event_attr(ctypes.Structure):
    _fields_ = [
        ("type", ctypes.c_uint32),
        ("size", ctypes.c_uint32),
        ("config", ctypes.c_uint64),
        ("sample_period", ctypes.c_uint64),
        ("sample_type", ctypes.c_uint64),
        ("read_format", ctypes.c_uint64),
        ("flags", ctypes.c_uint64),
        ("wakeup_events", ctypes.c_uint32),
        ("bp_type", ctypes.c_uint32),
        ("config1", ctypes.c_uint64),
        ("config2", ctypes.c_uint64),
        ("branch_sample_type", ctypes.c_uint64),
        ("sample_regs_user", ctypes.c_uint64),
        ("sample_stack_user", ctypes.c_uint32),
        ("clockid", ctypes.c_int32),
        ("sample_regs_intr", ctypes.c_uint64),
        ("aux_watermark", ctypes.c_uint32),
        ("sample_max_stack", ctypes.c_uint16),
        ("__reserved_2", ctypes.c_uint16),
        ("aux_sample_size", ctypes.c_uint32),
        ("__reserved_3", ctypes.c_uint32),
    ]


def perf_type():
    return int(open(f"{PMU}/type").read().strip())


def event_config(name):
    txt = open(f"{PMU}/events/{name}").read().strip()
    for part in txt.split(","):
        if part.startswith("config="):
            return int(part.split("=")[1], 0)
    return int(txt, 0)


def perf_open(ptype, config):
    attr = perf_event_attr()
    attr.size = ctypes.sizeof(perf_event_attr)
    attr.type = ptype
    attr.config = config
    attr.read_format = PERF_FORMAT_TOTAL_TIME_ENABLED
    nr_cpus = os.cpu_count() or 1
    for cpu in range(nr_cpus):
        fd = libc.syscall(__NR_perf_event_open, ctypes.byref(attr), -1, cpu, -1, 0)
        if fd >= 0:
            return fd
        err = ctypes.get_errno()
        if err != 22:
            raise OSError(err, f"perf_event_open failed: {os.strerror(err)}")
    raise OSError(err, f"perf_event_open failed on all cpus: {os.strerror(err)}")


def read_counter(fd):
    buf = os.read(fd, 16)
    value, time_enabled = struct.unpack("QQ", buf)
    return value, time_enabled


def main():
    ptype = perf_type()
    engines = []
    for name in ["rcs0-busy", "bcs0-busy", "vcs0-busy", "vecs0-busy"]:
        path = f"{PMU}/events/{name}"
        if not os.path.exists(path):
            continue
        try:
            fd = perf_open(ptype, event_config(name))
        except OSError as e:
            print(
                f"error opening {name}: {e}\n"
                f"  -> i915 PMU needs CAP_PERFMON/root here. Re-run with sudo.",
                file=sys.stderr,
            )
            sys.exit(13)
        engines.append((name.replace("-busy", ""), fd))

    if not engines:
        print("no i915 busy engines found", file=sys.stderr)
        sys.exit(1)

    labels = [e[0] for e in engines]
    prev = {lab: read_counter(fd) for lab, fd in engines}
    samples = {lab: [] for lab in labels}

    with open(OUT, "w") as f:
        f.write("t_ms," + ",".join(f"{l}_pct" for l in labels) + "\n")
        end = time.monotonic() + DURATION
        while time.monotonic() < end:
            time.sleep(INTERVAL)
            row = [str(int(time.monotonic() * 1000))]
            for lab, fd in engines:
                v, t = read_counter(fd)
                pv, pt = prev[lab]
                dt = t - pt or 1
                util = max(0.0, min(100.0, (v - pv) * 100.0 / dt))
                samples[lab].append(util)
                row.append(f"{util:.1f}")
                prev[lab] = (v, t)
            f.write(",".join(row) + "\n")

    print("  whole-GPU (i915 PMU, like btop):")
    for lab in labels:
        s = samples[lab]
        if not s:
            continue
        print(f"    {lab:6s}  avg={sum(s)/len(s):5.1f}%  peak={max(s):5.1f}%")
    rcs = samples.get("rcs0", [])
    if rcs:
        print(f"  -> render(rcs0) avg={sum(rcs)/len(rcs):.1f}%  peak={max(rcs):.1f}%")


if __name__ == "__main__":
    main()
