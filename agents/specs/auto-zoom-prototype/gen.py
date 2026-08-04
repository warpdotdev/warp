#!/usr/bin/env python3
"""Auto-zoom feasibility experiment.

Synthesizes a screen-recording-like master (grid background + a red "cursor"
box that jumps between known click points + a burned-in timestamp), then emits
a keyframed, eased zoom/pan camera as an ffmpeg `scale`+`crop` filtergraph.

The camera keyframes are generated programmatically from the same cursor
keyframes, which mirrors how the Rust post-process pass would emit a filtergraph
from the recorded `pointer_events`. Output is written to /tmp/az.
"""
import os

OUT_W, OUT_H, FPS, DUR = 1280, 720, 30, 9.0
AZ = "/tmp/az"

# Cursor path: (t_seconds, x, y) in source pixels. The camera should track this.
# Idle center -> click A (top-left) -> click B (bottom-right) -> back to center.
CURSOR = [
    (0.0, 640, 360),
    (2.0, 640, 360),
    (2.8, 320, 180),   # arrive at click A
    (4.0, 320, 180),
    (5.5, 960, 540),   # arrive at click B
    (6.5, 960, 540),
    (7.5, 640, 360),   # ease back to center
    (9.0, 640, 360),
]

# Camera zoom envelope: 1x idle, 2x while interacting, ease back to 1x.
ZOOM = [
    (0.0, 2.0, 1.0, 1.0),
    (2.0, 2.8, 1.0, 2.0),
    (2.8, 6.5, 2.0, 2.0),
    (6.5, 7.5, 2.0, 1.0),
    (7.5, 9.0, 1.0, 1.0),
]


def segs_from_points(points):
    """Turn a list of (t,v) keyframes into contiguous (ts,te,vs,ve) segments."""
    out = []
    for (t0, v0), (t1, v1) in zip(points, points[1:]):
        out.append((t0, t1, v0, v1))
    return out


def piecewise(segs, tv="t"):
    """Build an ffmpeg expr: smoothstep-eased piecewise-linear over time var `tv`."""
    first_ts = segs[0][0]
    last_val = segs[-1][3]
    e = f"{last_val:.4f}"
    for ts, te, vs, ve in reversed(segs):
        if te == ts or vs == ve:
            val = f"{vs:.4f}"
        else:
            x = f"(({tv}-{ts:.4f})/{(te - ts):.4f})"
            s = f"({x}*{x}*(3-2*{x}))"          # smoothstep ease in/out
            val = f"({vs:.4f}+({(ve - vs):.4f})*{s})"
        e = f"if(lt({tv},{te:.4f}),{val},{e})"
    return f"if(lt({tv},{first_ts:.4f}),{segs[0][2]:.4f},{e})"


cx = piecewise(segs_from_points([(t, x) for t, x, _ in CURSOR]))
cy = piecewise(segs_from_points([(t, y) for t, _, y in CURSOR]))
z = piecewise(ZOOM)
# zoompan uses its own `time` variable, not `t`.
cx_zp = piecewise(segs_from_points([(t, x) for t, x, _ in CURSOR]), tv="time")
cy_zp = piecewise(segs_from_points([(t, y) for t, _, y in CURSOR]), tv="time")
z_zp = piecewise(ZOOM, tv="time")

# Master: grid + moving red dot + timestamp. The dot is centered on the cursor
# path (offset by half its 24px size).
dot_x = f"({cx})-12"
dot_y = f"({cy})-12"
# drawtext is not compiled into the homebrew ffmpeg build (no libfreetype), so
# the timestamp is omitted; the grid + moving dot suffice to verify tracking.
draw_ts = ""
master_fc = (
    f"color=c=0x1e2430:s={OUT_W}x{OUT_H}:r={FPS}:d={DUR},"
    f"drawgrid=w=80:h=80:t=2:c=white@0.25{draw_ts}[bg];"
    f"color=c=red:s=24x24:r={FPS}:d={DUR}[dot];"
    f"[bg][dot]overlay=x='{dot_x}':y='{dot_y}'[out]"
)

# Zoom pass (scale->crop). Upscale whole frame by z(t), then crop a fixed
# OUT_WxOUT_H window centered on the cursor, clamped to frame bounds.
# scaled source point = c * iw / OUT_W  (== c*z since iw == OUT_W*z).
scale = (
    f"scale=w='ceil({OUT_W}*({z})/2)*2':h='ceil({OUT_H}*({z})/2)*2'"
    ":eval=frame:flags=bicubic"
)
crop = (
    f"crop={OUT_W}:{OUT_H}"
    f":x='clip(({cx})*iw/{OUT_W}-{OUT_W // 2},0,iw-{OUT_W})'"
    f":y='clip(({cy})*ih/{OUT_H}-{OUT_H // 2},0,ih-{OUT_H})'"
)
zoom_vf = f"{scale},{crop},setsar=1"

# zoompan alternative (for comparison only). zoompan crops an (iw/zoom x
# ih/zoom) region and scales it to `s`; center source point (px,py) by placing
# the crop origin at px-(iw/zoom)/2. Uses zoompan's `time`/`zoom` variables.
zoompan_vf = (
    f"zoompan=z='{z_zp}':d=1:fps={FPS}:s={OUT_W}x{OUT_H}"
    f":x='({cx_zp})-(iw/zoom/2)':y='({cy_zp})-(ih/zoom/2)'"
)

os.makedirs(AZ, exist_ok=True)
with open(f"{AZ}/master_fc.txt", "w") as f:
    f.write(master_fc)
with open(f"{AZ}/zoom_vf.txt", "w") as f:
    f.write(zoom_vf)
with open(f"{AZ}/zoompan_vf.txt", "w") as f:
    f.write(zoompan_vf)

print("cx(t) =", cx[:120], "...")
print("cy(t) =", cy[:120], "...")
print("z(t)  =", z)
print("\nwrote master_fc.txt, zoom_vf.txt, zoompan_vf.txt to", AZ)
