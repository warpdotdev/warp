//! Deterministic camera tracking for post-processed computer-use recordings.
//!
//! The camera is deliberately independent of ffmpeg. Pointer events are
//! remapped onto the compacted output timeline, converted into a bounded
//! per-frame track, and only then rendered by the Linux recording pipeline.

use std::time::Duration;

use pathfinder_geometry::vector::Vector2I;

use crate::ActionLogEntry;
use crate::overlay::{KeepSegment, remap_source_interval};

const MAX_FILTER_KEYFRAMES: usize = 256;
const EPSILON: f32 = 0.0001;

/// One sample of the virtual camera on the compacted output timeline.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct CameraKeyframe {
    pub(crate) t: Duration,
    pub(crate) zoom: f32,
    pub(crate) cx: f32,
    pub(crate) cy: f32,
}

/// A complete camera track sampled at the output frame cadence.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CameraTrack {
    pub(crate) keyframes: Vec<CameraKeyframe>,
    pub(crate) output_duration: Duration,
    pub(crate) max_zoom: f32,
}

impl CameraTrack {
    fn at_rest(dimensions: (u32, u32), output_duration: Duration, max_zoom: f32) -> Self {
        let (width, height) = dimensions;
        let center = CameraKeyframe {
            t: Duration::ZERO,
            zoom: 1.0,
            cx: width as f32 / 2.0,
            cy: height as f32 / 2.0,
        };
        Self {
            keyframes: vec![center],
            output_duration,
            max_zoom,
        }
    }

    pub(crate) fn is_at_rest(&self, dimensions: (u32, u32)) -> bool {
        let (width, height) = dimensions;
        let cx = width as f32 / 2.0;
        let cy = height as f32 / 2.0;
        self.keyframes.iter().all(|keyframe| {
            (keyframe.zoom - 1.0).abs() <= EPSILON
                && (keyframe.cx - cx).abs() <= EPSILON
                && (keyframe.cy - cy).abs() <= EPSILON
        })
    }
}

/// Policy for the deterministic camera model.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct CameraConfig {
    pub(crate) enabled: bool,
    pub(crate) target_zoom: f32,
    pub(crate) max_zoom: f32,
    pub(crate) zoom_in: Duration,
    pub(crate) zoom_out: Duration,
    pub(crate) idle_timeout: Duration,
    /// Spring-like follow rate in units per second.
    pub(crate) follow_stiffness: f32,
}

impl Default for CameraConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            target_zoom: 2.0,
            max_zoom: 2.5,
            zoom_in: Duration::from_millis(450),
            zoom_out: Duration::from_millis(650),
            idle_timeout: Duration::from_millis(1200),
            follow_stiffness: 8.0,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct MappedPointer {
    t: Duration,
    point: Vector2I,
}

#[derive(Debug, Clone)]
struct ActiveWindow {
    start: Duration,
    end: Duration,
    target_zoom: f32,
}

/// Builds a total, deterministic camera track from the recorded pointer
/// events. Events inside removed source gaps are dropped; all samples are
/// clamped so the zoomed viewport remains inside the source frame.
pub(crate) fn build_camera_track(
    entries: &[ActionLogEntry],
    segments: &[KeepSegment],
    dimensions: (u32, u32),
    frame_rate: u32,
    config: CameraConfig,
) -> CameraTrack {
    let output_duration = segments
        .last()
        .map(|segment| segment.output_start + (segment.source_end - segment.source_start))
        .unwrap_or(Duration::ZERO);
    let max_zoom = config.max_zoom.max(1.0);
    let mut track = if config.enabled {
        let pointers = mapped_pointers(entries, segments, frame_rate);
        if pointers.is_empty() {
            CameraTrack::at_rest(dimensions, output_duration, max_zoom)
        } else {
            build_enabled_track(
                &pointers,
                output_duration,
                dimensions,
                frame_rate,
                config,
                max_zoom,
            )
        }
    } else {
        CameraTrack::at_rest(dimensions, output_duration, max_zoom)
    };

    // Always return at least one safe sample, including for an empty segment
    // list or a zero-duration recording.
    if track.keyframes.is_empty() {
        track = CameraTrack::at_rest(dimensions, output_duration, max_zoom);
    }
    track
}

fn mapped_pointers(
    entries: &[ActionLogEntry],
    segments: &[KeepSegment],
    frame_rate: u32,
) -> Vec<MappedPointer> {
    let frame = Duration::from_secs_f64(1.0 / frame_rate.max(1) as f64);
    let mut pointers = entries
        .iter()
        .flat_map(|entry| entry.pointer_events.iter())
        .filter_map(|event| {
            // Treat a pointer event as occupying one source frame so events
            // exactly on a retained boundary are mapped consistently.
            let end = event.offset.saturating_add(frame);
            let (start, _) = remap_source_interval(event.offset, end, segments)?;
            Some(MappedPointer {
                t: start,
                point: event.point,
            })
        })
        .collect::<Vec<_>>();
    // Stable ordering preserves dispatch order for equal-offset events.
    pointers.sort_by_key(|pointer| pointer.t);
    pointers
}

fn build_enabled_track(
    pointers: &[MappedPointer],
    output_duration: Duration,
    dimensions: (u32, u32),
    frame_rate: u32,
    config: CameraConfig,
    max_zoom: f32,
) -> CameraTrack {
    let windows = active_windows(pointers, output_duration, dimensions, config);
    let (width, height) = dimensions;
    let center_x = width as f32 / 2.0;
    let center_y = height as f32 / 2.0;
    let fps = frame_rate.max(1) as f32;
    let frame_duration = Duration::from_secs_f32(1.0 / fps);
    let frame_count = ((output_duration.as_secs_f32() * fps).ceil() as usize).max(1);

    let mut current_x = center_x;
    let mut current_y = center_y;
    let mut keyframes = Vec::with_capacity(frame_count + 1);
    for index in 0..=frame_count {
        let t = frame_duration.mul_f32(index as f32);
        let active = windows
            .iter()
            .find(|window| t >= window.start && t <= window.end);
        let target = active
            .and_then(|_| latest_pointer_at(pointers, t))
            .map(|pointer| (pointer.point.x() as f32, pointer.point.y() as f32))
            .unwrap_or((center_x, center_y));

        let zoom = active
            .map(|window| zoom_at(t, window, config.zoom_in, config.zoom_out))
            .unwrap_or(1.0)
            .clamp(1.0, max_zoom);

        // A critically-damped approximation. The explicit velocity cap makes
        // the output bounded even for a pointer jump from one frame edge to the
        // other.
        let alpha = 1.0 - (-config.follow_stiffness.max(0.0) / fps).exp();
        let max_delta = (width.min(height) as f32 * 0.75 / fps).max(1.0);
        current_x += ((target.0 - current_x) * alpha).clamp(-max_delta, max_delta);
        current_y += ((target.1 - current_y) * alpha).clamp(-max_delta, max_delta);
        let (current_x, current_y) = clamp_center(current_x, current_y, zoom, dimensions);

        keyframes.push(CameraKeyframe {
            t: t.min(output_duration),
            zoom,
            cx: current_x,
            cy: current_y,
        });
    }

    // Ensure the final sample is exactly full-frame and centered. At zoom 1 the
    // legal center is uniquely the frame center, regardless of preceding motion.
    if let Some(last) = keyframes.last_mut() {
        last.t = output_duration;
        last.zoom = 1.0;
        last.cx = center_x;
        last.cy = center_y;
    }
    CameraTrack {
        keyframes,
        output_duration,
        max_zoom,
    }
}

fn active_windows(
    pointers: &[MappedPointer],
    output_duration: Duration,
    dimensions: (u32, u32),
    config: CameraConfig,
) -> Vec<ActiveWindow> {
    let mut windows: Vec<(Duration, Duration, Vec<Vector2I>)> = Vec::new();
    for pointer in pointers {
        let end = pointer
            .t
            .saturating_add(config.idle_timeout)
            .min(output_duration);
        if let Some(last) = windows.last_mut()
            && pointer.t <= last.1
        {
            last.1 = last.1.max(end);
            last.2.push(pointer.point);
        } else {
            windows.push((pointer.t, end, vec![pointer.point]));
        }
    }

    windows
        .into_iter()
        .map(|(start, end, points)| ActiveWindow {
            start,
            end,
            target_zoom: target_zoom_for_points(&points, dimensions, config),
        })
        .collect()
}

fn target_zoom_for_points(
    points: &[Vector2I],
    dimensions: (u32, u32),
    config: CameraConfig,
) -> f32 {
    let (width, height) = dimensions;
    let min_x = points
        .iter()
        .map(|point| point.x())
        .min()
        .unwrap_or(0)
        .max(0) as f32;
    let max_x = points
        .iter()
        .map(|point| point.x())
        .max()
        .unwrap_or(width as i32)
        .min(width as i32) as f32;
    let min_y = points
        .iter()
        .map(|point| point.y())
        .min()
        .unwrap_or(0)
        .max(0) as f32;
    let max_y = points
        .iter()
        .map(|point| point.y())
        .max()
        .unwrap_or(height as i32)
        .min(height as i32) as f32;
    let spread_x = (max_x - min_x).max(1.0);
    let spread_y = (max_y - min_y).max(1.0);
    let spread_limit = (width as f32 / spread_x).min(height as f32 / spread_y);
    config
        .target_zoom
        .min(config.max_zoom.max(1.0))
        .min(spread_limit.max(1.0))
        .max(1.0)
}

fn latest_pointer_at(pointers: &[MappedPointer], t: Duration) -> Option<&MappedPointer> {
    pointers.iter().rev().find(|pointer| pointer.t <= t)
}

fn zoom_at(t: Duration, window: &ActiveWindow, zoom_in: Duration, zoom_out: Duration) -> f32 {
    let target = window.target_zoom;
    if target <= 1.0 {
        return 1.0;
    }
    let in_duration = zoom_in.max(Duration::from_millis(1));
    let out_duration = zoom_out.max(Duration::from_millis(1));
    let in_end = window.start.saturating_add(in_duration);
    let out_start = window.end.saturating_sub(out_duration);
    if out_start <= in_end {
        let midpoint = window.start + (window.end - window.start) / 2;
        if t <= midpoint {
            let p = progress(window.start, midpoint, t);
            return 1.0 + (target - 1.0) * smoothstep(p);
        }
        let p = progress(midpoint, window.end, t);
        return 1.0 + (target - 1.0) * (1.0 - smoothstep(p));
    }
    if t < in_end {
        return 1.0 + (target - 1.0) * smoothstep(progress(window.start, in_end, t));
    }
    if t >= out_start {
        return 1.0 + (target - 1.0) * (1.0 - smoothstep(progress(out_start, window.end, t)));
    }
    target
}

fn progress(start: Duration, end: Duration, t: Duration) -> f32 {
    if end <= start {
        1.0
    } else {
        (t.saturating_sub(start).as_secs_f32() / (end - start).as_secs_f32()).clamp(0.0, 1.0)
    }
}

fn smoothstep(x: f32) -> f32 {
    x * x * (3.0 - 2.0 * x)
}

fn clamp_center(cx: f32, cy: f32, zoom: f32, dimensions: (u32, u32)) -> (f32, f32) {
    let (width, height) = dimensions;
    let half_width = width as f32 / (2.0 * zoom.max(1.0));
    let half_height = height as f32 / (2.0 * zoom.max(1.0));
    (
        cx.clamp(half_width, width as f32 - half_width),
        cy.clamp(half_height, height as f32 - half_height),
    )
}

/// Builds the Linux `zoompan` video filter for a camera track.
pub(crate) fn build_zoompan_filter(
    track: &CameraTrack,
    dimensions: (u32, u32),
    frame_rate: u32,
) -> String {
    let (width, height) = dimensions;
    let zoom = piecewise_expression(track, |keyframe| keyframe.zoom, "time");
    let cx = piecewise_expression(track, |keyframe| keyframe.cx, "time");
    let cy = piecewise_expression(track, |keyframe| keyframe.cy, "time");
    let max_zoom = track.max_zoom.max(1.0);
    format!(
        "zoompan=z='clip({zoom},1,{max_zoom:.6})':d=1:fps={}:s={}x{}:x='clip(({cx})-(iw/zoom/2),0,iw-(iw/zoom))':y='clip(({cy})-(ih/zoom/2),0,ih-(ih/zoom))',setsar=1",
        frame_rate.max(1),
        width,
        height,
    )
}

fn piecewise_expression(
    track: &CameraTrack,
    value: impl Fn(CameraKeyframe) -> f32,
    variable: &str,
) -> String {
    let keyframes = compact_keyframes(track);
    let Some(first) = keyframes.first().copied() else {
        return "1.000000".to_string();
    };
    let mut expression = format!("{:.6}", value(*keyframes.last().unwrap()));
    for pair in keyframes.windows(2).rev() {
        let start = pair[0];
        let end = pair[1];
        let duration = (end.t - start.t).as_secs_f64();
        let start_value = value(start);
        let end_value = value(end);
        let segment = if duration <= 0.0 || (start_value - end_value).abs() <= EPSILON {
            format!("{start_value:.6}")
        } else {
            let x = format!("(({variable}-{:.6})/{duration:.6})", start.t.as_secs_f64());
            let eased = format!("({x}*{x}*(3-2*{x}))");
            format!(
                "({start_value:.6}+({:.6})*{eased})",
                end_value - start_value
            )
        };
        expression = format!(
            "if(lt({variable},{:.6}),{segment},{expression})",
            end.t.as_secs_f64()
        );
    }
    format!(
        "if(lt({variable},{:.6}),{:.6},{expression})",
        first.t.as_secs_f64(),
        value(first)
    )
}

fn compact_keyframes(track: &CameraTrack) -> Vec<CameraKeyframe> {
    if track.keyframes.len() <= MAX_FILTER_KEYFRAMES {
        return track.keyframes.clone();
    }
    let last = track.keyframes.len() - 1;
    let step = last as f32 / (MAX_FILTER_KEYFRAMES - 1) as f32;
    (0..MAX_FILTER_KEYFRAMES)
        .map(|index| track.keyframes[(index as f32 * step).round() as usize])
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{MouseButton, PointerEvent, PointerEventKind};

    fn segment(source_start: u64, source_end: u64, output_start: u64) -> KeepSegment {
        KeepSegment {
            source_start: Duration::from_secs(source_start),
            source_end: Duration::from_secs(source_end),
            output_start: Duration::from_secs(output_start),
        }
    }

    fn entry(offset: u64, point: (i32, i32)) -> ActionLogEntry {
        ActionLogEntry {
            offset: Duration::from_millis(offset * 100),
            finish_offset: Duration::from_millis(offset * 100 + 100),
            labels: Vec::new(),
            pointer_events: vec![PointerEvent {
                offset: Duration::from_millis(offset * 100),
                kind: PointerEventKind::Down,
                button: Some(MouseButton::Left),
                point: Vector2I::new(point.0, point.1),
            }],
        }
    }

    fn enabled_config() -> CameraConfig {
        CameraConfig {
            enabled: true,
            ..CameraConfig::default()
        }
    }

    #[test]
    fn idle_track_is_full_frame() {
        let track = build_camera_track(&[], &[segment(0, 4, 0)], (1280, 720), 10, enabled_config());
        assert!(track.is_at_rest((1280, 720)));
        assert!(track.keyframes.iter().all(|keyframe| keyframe.zoom == 1.0));
    }

    #[test]
    fn interaction_eases_and_clamps_edge_target() {
        let track = build_camera_track(
            &[entry(2, (0, 0))],
            &[segment(0, 6, 0)],
            (1280, 720),
            10,
            enabled_config(),
        );
        assert!(track.keyframes.iter().all(|keyframe| {
            let half_width = 1280.0 / (2.0 * keyframe.zoom);
            let half_height = 720.0 / (2.0 * keyframe.zoom);
            keyframe.zoom >= 1.0
                && keyframe.zoom <= 2.5
                && keyframe.cx >= half_width - EPSILON
                && keyframe.cx <= 1280.0 - half_width + EPSILON
                && keyframe.cy >= half_height - EPSILON
                && keyframe.cy <= 720.0 - half_height + EPSILON
        }));
        let max_zoom = track
            .keyframes
            .iter()
            .map(|keyframe| keyframe.zoom)
            .fold(1.0, f32::max);
        assert!(max_zoom > 1.0);
        assert_eq!(track.keyframes.first().unwrap().zoom, 1.0);
        assert_eq!(track.keyframes.last().unwrap().zoom, 1.0);
    }

    #[test]
    fn event_after_removed_gap_uses_output_time() {
        let track = build_camera_track(
            &[entry(45, (900, 300))],
            &[segment(0, 2, 0), segment(4, 6, 2)],
            (1000, 600),
            10,
            enabled_config(),
        );
        let first_active = track
            .keyframes
            .iter()
            .find(|keyframe| keyframe.zoom > 1.0)
            .expect("event should activate camera");
        assert!(
            first_active.t.as_secs_f32() < 4.5,
            "source event should shift left by the removed gap: {:?}",
            first_active.t
        );
        assert!(first_active.t.as_secs_f32() >= 2.0);
    }

    #[test]
    fn filter_is_deterministic_and_uses_zoompan_variables() {
        let track = build_camera_track(
            &[entry(2, (640, 360))],
            &[segment(0, 6, 0)],
            (1280, 720),
            15,
            enabled_config(),
        );
        let first = build_zoompan_filter(&track, (1280, 720), 15);
        let second = build_zoompan_filter(&track, (1280, 720), 15);
        assert_eq!(first, second);
        assert!(first.contains("zoompan"));
        assert!(first.contains("time"));
        assert!(first.contains("zoom"));
        assert!(first.contains("iw"));
        assert!(first.contains("ih"));
        assert!(first.contains("d=1:fps=15:s=1280x720"));
        assert!(first.contains("clip("));
        assert!(!first.contains("t)"));
    }

    #[test]
    fn degenerate_and_disabled_inputs_never_panic() {
        let mut config = enabled_config();
        config.max_zoom = 0.0;
        let track = build_camera_track(
            &[ActionLogEntry {
                offset: Duration::from_secs(9),
                finish_offset: Duration::from_secs(9),
                labels: Vec::new(),
                pointer_events: Vec::new(),
            }],
            &[],
            (100, 100),
            0,
            config,
        );
        assert!(track.is_at_rest((100, 100)));

        config.enabled = false;
        let disabled = build_camera_track(
            &[entry(1, (0, 0))],
            &[segment(0, 2, 0)],
            (100, 100),
            10,
            config,
        );
        assert!(disabled.is_at_rest((100, 100)));
    }
}
