//! Memory-behavior repro for APP-4154 batch 1.C (warpui-platform-nsstring).
//!
//! Exercises the `NSString::alloc(nil).init_str(...)` → `make_nsstring(...)` conversions
//! applied to `pasteboard_type_for_image_mime_type` and related clipboard
//! helpers. The helper is shared by every retained-NSString site in this file
//! (6 in `read_image_data_from_pasteboard`, 2 in `Clipboard::write`, and this
//! one in `pasteboard_type_for_image_mime_type`), so it is representative for
//! the whole file.
//!
//! On master the raw `NSString::alloc(nil).init_str(...)` returns a string with
//! a +1 retain count that no pool drain can clean up, so even inside an outer
//! `NSAutoreleasePool` the string survives past `pool.drain()` and memory grows
//! linearly with iteration count.
//!
//! On the PR branch `make_nsstring` autoreleases, so every string returned here
//! is released when the surrounding pool drains and peak RSS stays flat across
//! outer iterations.
//!
//! Run as:
//!   cargo test --release -p warpui \
//!       pasteboard_type_for_image_mime_type_memory_behavior -- --nocapture --ignored
//! and measure peak RSS with `/usr/bin/time -l`.
use cocoa::base::nil;
use cocoa::foundation::NSAutoreleasePool;

use super::pasteboard_type_for_image_mime_type;

/// Number of outer pool cycles. Each cycle creates an `NSAutoreleasePool`,
/// runs the inner loop, then drains. On master the retained NSStrings survive
/// the drain, so memory usage grows proportionally to OUTER * INNER.
const OUTER: usize = 60;

/// Number of inner iterations per pool cycle. Must be large enough to produce
/// a measurable RSS delta but small enough to fit easily in memory on the
/// branch side (where strings are reclaimed per cycle).
const INNER: usize = 20_000;

const MIME_TYPES: &[&str] = &[
    "image/png",
    "image/jpeg",
    "image/gif",
    "image/webp",
    "image/svg+xml",
];

#[test]
#[ignore = "memory repro; run with --ignored --nocapture in release mode"]
fn pasteboard_type_for_image_mime_type_memory_behavior() {
    unsafe {
        for _ in 0..OUTER {
            let pool = NSAutoreleasePool::new(nil);
            for _ in 0..INNER {
                for mime in MIME_TYPES {
                    let ns = pasteboard_type_for_image_mime_type(mime);
                    assert!(ns.is_some(), "mime {mime} should be mapped");
                }
            }
            pool.drain();
        }
    }
}
