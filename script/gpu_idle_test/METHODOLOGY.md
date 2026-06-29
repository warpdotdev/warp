# Active-path GPU methodology (Linux / Intel i915)

How the **active** (typing / scrolling / output) GPU cost was measured and
attributed on Linux + Intel integrated graphics, and the reasoning behind the
partial-repaint fix. Companion to `README.md` (which covers the idle path).

Hardware this was developed on: Intel Iris Xe (i7-1270P), Wayland (GNOME/mutter),
**3840×2400** display.

## Tools

| Script | Measures | Source |
|---|---|---|
| `gpu_pmu_sampler.py` | whole-GPU render-engine busy % (system-wide) | i915 PMU (`perf_event_open`), same source as btop |
| `cpu_thread_profile.py` | warp's per-thread CPU % | `/proc/<pid>/task/*/stat` |
| `gpu_split_probe.py` | per-process GPU render % (Warp vs compositor) | `/proc/<pid>/fdinfo` `drm-engine-render` |
| `profile_active.sh` | drives a regime (type/scroll/flood) + samples the above | ydotool |

## Methodology lessons (each one cost real debugging time)

1. **Measure release, never debug.** A debug build is CPU-starved under
   key-mash; it stutters and *under*-reports GPU because the GPU is starved
   waiting on the CPU. Every number below is from a `--release` build.

2. **Sample at ≤100 ms, or you will lie to yourself.** The i915 busy counter is
   integrated over the sample window, so a coarse interval averages activity
   with idle:
   - At **100 ms**, GPU drops to idle within ~0.1 s of input stopping (matches
     btop), and peaks read true (~82%).
   - At **2000–5000 ms**, the window straddling input-stop blends busy+idle and
     reports ~20–35% for *seconds* after you stop — a phantom "tail" that does
     not exist — while peaks get smoothed down to ~55%.

   A long-perceived "GPU keeps working after I stop typing" turned out to be
   purely this averaging artifact. Re-aggregating one fine trace into coarse
   windows reproduces it exactly.

3. **Gate on a focus check.** ydotool input goes to the *focused* window; a
   missed focus silently produces a no-load run. `profile_active.sh` aborts if
   the GPU stays < 5% during a workload.

4. **A/B same-commit release binaries.** Absolute numbers depend on
   hardware/compositor/resolution; only same-commit base-vs-candidate deltas are
   attributable. See `build_release_pair.sh`.

## Findings (release, Iris Xe, 3840×2400, 100 ms PMU)

**Per regime — different bottlenecks:**

| Regime | GPU render (rcs0) | Hottest warp thread | Bound by |
|---|---|---|---|
| Typing at prompt | ~60% avg / ~82% peak | main ~14% (CPU ~20%) | **GPU rasterization** |
| Scrolling scrollback | ~46% avg / ~74% peak | main ~5% | **GPU rasterization** |
| Output flood (`seq`) | ~32% avg / ~64% peak | **PTY reader ~64%** | **CPU — terminal parsing** |

**GPU attribution during typing** (`gpu_split_probe.py`, fdinfo, debug split but
the ratio is frame-rate-independent): Warp's own rasterization ≈ **76%** of the
render-engine time, GNOME Shell (compositor) ≈ **24%**. The split closes against
the system PMU total, so the dominant cost is Warp re-rasterizing its own window
— not the compositor.

**Frame-rate is not the lever.** Halving the terminal wakeup throttle (60→30 Hz)
left active GPU unchanged (same-commit release A/B: 40.6% vs 40.6%). The render
rate already sits below the cap because each full-window frame is expensive; vsync
pacing (#13119) is the same dead lever for the same reason.

**Root cause of the typing cost:** every keystroke invalidates ~10 input-bar
views (`EditorView`, `PromptDisplay`, `AgentMessageBar`, `AgentViewFooter`,
`TerminalInputMessageBar`, `DisplayChip`×N, `ActionButton`,
`AcceptAutosuggestionKeybinding`), each forcing a full-window repaint. The
architecture re-rasterizes the whole window on any change, which is ~free on the
Apple-Silicon target but expensive on a 4K iGPU.

## The fix and its measured effect

Damage the frame to the bounds of the views that actually re-rendered (keyed off
the invalidated set, so it also covers re-created views like context chips), and
scissor the rasterization to that region. Same-commit release A/B, typing:

```
render(rcs0)  avg 54.8% -> 46.5%   peak 75.1% -> 63.4%
```

**Ceiling / follow-up:** this only shrinks Warp's *own* scene raster. The
per-frame full offscreen→swapchain copy and the compositor's full-surface present
(~24%) are unaffected — a further drop needs `wl_surface` buffer damage on the
present path.

## Running it

```bash
# one release binary, one regime
PROFILE_S=8 ./profile_active.sh /path/to/warp-oss type     # or: scroll | flood

# same-commit A/B (base = no fix, candidate = fix)
./build_release_pair.sh
PROFILE_S=8 ./profile_active.sh target/.../warp-oss-rel-base      type
PROFILE_S=8 ./profile_active.sh target/.../warp-oss-rel-candidate type

# attribute the GPU during a workload (run while a flood/scroll is going)
./gpu_split_probe.py 8
```

Do not run the input-injecting scripts while using the machine — they take over
the focused window.
