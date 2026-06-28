# GPU idle utilization testing (Linux / Intel i915)

This directory contains scripts used to measure **GPU utilization** (not VRAM)
for Warp on Linux with Intel integrated graphics. It was developed while
debugging high idle GPU usage when a terminal window is focused but otherwise
idle (cursor blinking).

## Prerequisites

- **Linux** with Intel i915 GPU and PMU events under `/sys/bus/event_source/devices/i915/`
- **`perf_event_paranoid`** low enough to read PMU counters (often `0`; btop works ⇒ you are fine)
- **`ydotool` + `ydotoold`** for autonomous keyboard input during harness runs
- **`python3`** (stdlib only for `gpu_pmu_sampler.py`)
- **Wayland** (set `WARP_ENABLE_WAYLAND=1` when launching test binaries)

Install on Arch:

```bash
sudo pacman -S ydotool
ydotoold --socket-path "$XDG_RUNTIME_DIR/.ydotool_socket" --socket-own "$(id -u):$(id -g)" &
```

## Important: use **release** builds for meaningful results

`cargo build` (debug) produces an unoptimized binary that is **CPU-bound** under
key-mash workloads and will **stutter** even when GPU numbers look lower. Always
compare **release** binaries for responsiveness and for idle GPU measurements
that reflect real usage.

Release builds of the full `warp` crate can peak at **~12GB RSS** during
compilation. On 32GB machines, use the provided build flags:

```bash
export CARGO_PROFILE_RELEASE_CODEGEN_UNITS=256
export CARGO_PROFILE_RELEASE_DEBUG=0
```

Optional: run `bash script/gpu_idle_test/mem_watchdog.sh 2500000` in another
terminal during builds to kill cargo before the OOM killer takes down your session.

## Fast feedback loop

The intended iteration cycle:

1. **Change code** on your branch (e.g. cursor-blink damage rendering).
2. **Build release pair** (base + candidate at same merge-base):

   ```bash
   ./script/gpu_idle_test/build_release_pair.sh
   ```

   Outputs:
   - `target/gpu_idle_test/bin/warp-oss-rel-base` (merge-base, no fix)
   - `target/gpu_idle_test/bin/warp-oss-rel-candidate` (HEAD, with fix)

3. **Run automated A/B** (~2 minutes, hands-off):

   ```bash
   cd script/gpu_idle_test
   IDLE_S=25 ACTIVE_S=12 ./compare_base_vs_candidate.sh 4000
   ```

4. **Read the summary** — focus on **IDLE** avg/peak (ACTIVE is expected ~50% on
   4K integrated GPUs and is unchanged by the idle blink fix).

5. **Optional human sanity check** — launch one binary at a time; watch the GPU
   graph in **Mission Center** (or btop render-engine %) for ~20s while idle:

   ```bash
   WARP_ENABLE_WAYLAND=1 target/gpu_idle_test/bin/warp-oss-rel-base
   WARP_ENABLE_WAYLAND=1 target/gpu_idle_test/bin/warp-oss-rel-candidate
   ```

   Focus the window, do not type, watch GPU for ~20s (cursor blinking only).

## What the harness measures

`gpu_pmu_sampler.py` reads i915 **`*-busy`** perf counters (same family as btop /
Mission Center render engine). Each sample computes:

```text
util% = delta(busy_ns) / delta(time_enabled_ns) * 100
```

`verify_idle_vs_active.sh`:

1. Launches Warp (Wayland), dismisses startup modal with Escape
2. Samples **IDLE** GPU for `IDLE_S` seconds (no input)
3. Optionally floods scrollback (`seq 1 N`), then runs an **ACTIVE** workload
   (typing + PageUp/PageDown) while sampling

`compare_base_vs_candidate.sh` runs verify on base then candidate and prints:

```text
IDLE    base avg= ...  ->  cand avg= ...
ACTIVE  base avg= ...  ->  cand avg= ...
```

Use **IDLE** delta for the blink-fix validation. ACTIVE validates no regression.

## Reproduce from a PR checkout (`gh` CLI)

```bash
# 1. Check out the PR branch
gh pr checkout <PR_NUMBER>
cd warp   # if gh cloned elsewhere, cd to your checkout

# 2. Build release base + candidate (~15–25 min first time; reuse cached deps after)
./script/gpu_idle_test/build_release_pair.sh

# 3. Run automated comparison (leave machine alone ~2 min; uses ydotool keyboard)
cd script/gpu_idle_test
chmod +x *.sh *.py
IDLE_S=25 ACTIVE_S=12 ./compare_base_vs_candidate.sh 4000

# 4. Manual idle check (optional)
WARP_ENABLE_WAYLAND=1 ../../target/gpu_idle_test/bin/warp-oss-rel-base
# quit, then:
WARP_ENABLE_WAYLAND=1 ../../target/gpu_idle_test/bin/warp-oss-rel-candidate
```

### Expected results (Intel Iris Xe, Wayland, 3840×2400, release, warp-oss config)

These are **reference numbers** from one machine; absolute % varies with focus,
session restore, and UI settings. The **delta** is what matters.

| Measurement | rel-base | rel-candidate |
|-------------|----------|---------------|
| Harness IDLE avg | ~12.4% | ~9.6% |
| Manual IDLE (Mission Center graph) | ~6% | ~3% |

Active key-mash / scroll GPU (~50%) and **responsiveness** should match between
base and candidate in release builds.

## Caveats

- **Not VRAM / #2319**: this measures GPU **utilization** (render engine busy %).
- **warp-oss vs stable**: dev `warp-oss` uses `~/.config/warp-oss/` and
  `~/.local/state/warp-oss/`. Shipped stable uses `warp-terminal` paths and
  often has more UI enabled (vertical tabs, AI, etc.) — do not compare absolute
  idle % across channels without matching config.
- **Session restore**: Warp restores blocks from `warp.sqlite` on launch. Set
  `RESET_SESSION=1` when calling verify to start from an empty terminal.
- **Process cleanup**: release test binaries named `warp-oss-rel-*` truncate to
  15 chars in `/proc/comm`; scripts kill by path pattern under `TEST_BIN_DIR`.
- **Do not use absolute ydotool mousemove** on GNOME without care — it can hit
  the top-left hot corner and open Overview. The harness uses keyboard only.

## Files

| File | Purpose |
|------|---------|
| `gpu_pmu_sampler.py` | i915 PMU sampler → CSV |
| `verify_idle_vs_active.sh` | Single-binary idle + active run |
| `compare_base_vs_candidate.sh` | Same-commit A/B comparison |
| `build_release_pair.sh` | Build rel-base + rel-candidate |
| `mem_watchdog.sh` | Optional OOM guard during `cargo build --release` |
