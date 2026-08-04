# Auto-zoom prototype (feasibility evidence)

Companion to `agents/specs/QUALITY-auto-zoom-for-computer-use-recordings.md`.

`gen.py` synthesizes a 1280x720@30fps screen-recording-like clip (grid + a red
"cursor" box that jumps between known points) and emits an eased, keyframed
camera track as ffmpeg filtergraphs — the same pattern the Rust post-process
pass will use to turn recorded `pointer_events` into a zoom/pan camera.

## Reproduce
```bash
mkdir -p /tmp/az && cp gen.py /tmp/az && cd /tmp/az && python3 gen.py
# master (synthetic recording)
ffmpeg -y -filter_complex "$(cat master_fc.txt)" -map "[out]" -c:v libx264 -preset ultrafast -pix_fmt yuv420p -r 30 master.mp4
# zoom via zoompan (recommended mechanism)
ffmpeg -y -i master.mp4 -vf "$(cat zoompan_vf.txt)" -c:v libx264 -preset ultrafast -pix_fmt yuv420p zoom_zoompan.mp4
# zoom via scale+crop (works for common cases; mis-frames clamp-boundary targets)
ffmpeg -y -i master.mp4 -vf "$(cat zoom_vf.txt)" -c:v libx264 -preset ultrafast -pix_fmt yuv420p zoom_scalecrop.mp4
```

## Findings (ffmpeg 8.1.2)
- Post-processing cost is negligible: a 9 s clip zoom pass encodes in ~0.2 s.
- Output geometry is preserved (1280x720, 30fps, 270 frames) by both mechanisms.
- `zoompan` centers and clamps every target correctly, including a frame-edge
  target; it requires its own `time`/`zoom` expression variables (not `t`).
- Naive `scale(eval=frame)+crop` tracked common cases but mis-framed the
  clamp-boundary target (crop size can't vary per frame). `zoompan` is the
  recommended mechanism.
