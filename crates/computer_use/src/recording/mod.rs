//! The recording core shared by the real ffmpeg-backed recorders.
//!
//! Both capture substrates — macOS `avfoundation` and Linux `x11grab` — record a
//! 1x master to an ephemeral MP4 and then run the same post-stop pass: cut the
//! master down to the retained action segments and burn the action and pointer
//! overlays into the result. Everything independent of the capture substrate
//! lives here: temp-file allocation, encode and output settings, launch and
//! finalize supervision, and post-processing.
//!
//! Platform adapters keep what genuinely differs: input selection and its
//! arguments, target preparation and dimensions, cursor flags, and startup
//! classification, permissions, and retry policy.

pub(crate) mod capture;
pub(crate) mod post_process;
#[cfg(any(macos, test))]
pub(crate) mod window_crop;
