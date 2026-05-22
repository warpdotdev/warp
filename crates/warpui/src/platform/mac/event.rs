use std::ffi::CStr;
use std::os::raw::c_char;

use cocoa::appkit::{NSEvent, NSEventModifierFlags, NSEventType};
use cocoa::base::{id, YES};
use cocoa::foundation::{NSString, NSUInteger};
use pathfinder_geometry::vector::vec2f;
use warpui_core::event::{KeyEventDetails, ModifiersState};
use warpui_core::keymap::Keystroke;
use warpui_core::platform::keyboard::{KeyCode, PhysicalKey};
use warpui_core::Event;

use super::keycode::{scancode_to_physicalkey, Keycode};
use super::utils::unicode_char_to_key;

// Unpublished but widely known and stable flags for distinguishing left/right alt.
// Google "NX_DEVICELALTKEYMASK" for more.
const LEFT_ALT_MASK: NSUInteger = 0x00000020;
const RIGHT_ALT_MASK: NSUInteger = 0x00000040;

fn modifier_flags_to_state(flags: NSEventModifierFlags) -> ModifiersState {
    ModifiersState {
        alt: flags.contains(NSEventModifierFlags::NSAlternateKeyMask),
        cmd: flags.contains(NSEventModifierFlags::NSCommandKeyMask),
        shift: flags.contains(NSEventModifierFlags::NSShiftKeyMask),
        ctrl: flags.contains(NSEventModifierFlags::NSControlKeyMask),
        func: flags.contains(NSEventModifierFlags::NSFunctionKeyMask),
    }
}

fn native_key_code_to_key_code(native_key_code: u16) -> Option<KeyCode> {
    let physical_key = scancode_to_physicalkey(native_key_code as u32);
    match physical_key {
        PhysicalKey::Code(key_code) => Some(key_code),
        _ => None,
    }
}

/// # Safety
/// This code is only unsafe since it requires interfacing with platform code.
/// Creates an event from a native event, taking in the current window_height and whether this is
/// the first mouse event on an inactive window that is causing the window to activate.
pub unsafe fn from_native(
    native_event: id,
    window_height: Option<f32>,
    is_first_mouse: bool,
) -> Option<Event> {
    let event_type = native_event.eventType();

    // Filter out event types that aren't in the NSEventType enum.
    // See https://github.com/servo/cocoa-rs/issues/155#issuecomment-323482792 for details.
    match event_type as u64 {
        0 | 21 | 32 | 33 | 35 | 36 | 37 => {
            return None;
        }
        _ => {}
    }
    let modifiers = modifier_flags_to_state(native_event.modifierFlags());

    match event_type {
        NSEventType::NSKeyDown => {
            let native_modifiers = native_event.modifierFlags();

            // Get the base character for this key without any modifiers (including Shift)
            // using UCKeyTranslate via the platform's keyCodeToChar function.
            // For example, Shift+1 on a US keyboard gives '!' as the key, but
            // key_without_modifiers will be '1'.
            let key_without_modifiers = Keycode(native_event.keyCode()).try_to_key_name(false);

            let details = KeyEventDetails {
                left_alt: (native_modifiers.bits() & LEFT_ALT_MASK) != 0,
                right_alt: (native_modifiers.bits() & RIGHT_ALT_MASK) != 0,
                key_without_modifiers,
            };
            let unmodified_chars = native_event.charactersIgnoringModifiers();
            let unmodified_chars = CStr::from_ptr(unmodified_chars.UTF8String() as *mut c_char)
                .to_str()
                .ok()?;

            let unmodified_chars = if let Some(first_char) = unmodified_chars.chars().next() {
                unicode_char_to_key(first_char as u16).unwrap_or(unmodified_chars)
            } else {
                return None;
            };

            let keystroke = Keystroke {
                ctrl: native_modifiers.contains(NSEventModifierFlags::NSControlKeyMask),
                alt: native_modifiers.contains(NSEventModifierFlags::NSAlternateKeyMask),
                shift: native_modifiers.contains(NSEventModifierFlags::NSShiftKeyMask),
                cmd: native_modifiers.contains(NSEventModifierFlags::NSCommandKeyMask),
                meta: false, /* handled separately */
                key: unmodified_chars.into(),
            };

            let chars = native_event.characters().UTF8String() as *mut c_char;
            let chars = if chars.is_null() {
                // `UTF8String` can return null in some rare cases where the
                // string isn't valid UTF-8.  For example, if the user
                // enters a UTF-8 surrogate character, e.g. U+DDDD, via the
                // Unicode Hex Input keyboard, the conversion will produce
                // null.
                String::new()
            } else {
                CStr::from_ptr(chars).to_str().ok()?.to_owned()
            };

            Some(Event::KeyDown {
                keystroke,
                chars,
                details,
                is_composing: false,
            })
        }
        NSEventType::NSMouseMoved => window_height.map(|window_height| Event::MouseMoved {
            position: vec2f(
                native_event.locationInWindow().x as f32,
                window_height - native_event.locationInWindow().y as f32,
            ),
            cmd: native_event
                .modifierFlags()
                .contains(NSEventModifierFlags::NSCommandKeyMask),
            shift: native_event
                .modifierFlags()
                .contains(NSEventModifierFlags::NSShiftKeyMask),
            is_synthetic: false,
        }),
        NSEventType::NSFlagsChanged => {
            let key_code = native_key_code_to_key_code(native_event.keyCode());

            window_height.map(|window_height| Event::ModifierStateChanged {
                mouse_position: vec2f(
                    native_event.locationInWindow().x as f32,
                    window_height - native_event.locationInWindow().y as f32,
                ),
                modifiers,
                key_code,
            })
        }
        NSEventType::NSLeftMouseDown => window_height.map(|window_height| {
            let position = vec2f(
                native_event.locationInWindow().x as f32,
                window_height - native_event.locationInWindow().y as f32,
            );
            let click_count = native_event.clickCount() as u32;

            // ctrl-click should actually be registered as a right-click
            // https://support.apple.com/guide/mac-help/right-click-mh35853/mac
            if modifiers.ctrl {
                Event::RightMouseDown {
                    position,
                    cmd: modifiers.cmd,
                    shift: modifiers.shift,
                    click_count,
                }
            } else {
                Event::LeftMouseDown {
                    position,
                    modifiers,
                    click_count,
                    is_first_mouse,
                }
            }
        }),
        NSEventType::NSLeftMouseUp => window_height.map(|window_height| Event::LeftMouseUp {
            position: vec2f(
                native_event.locationInWindow().x as f32,
                window_height - native_event.locationInWindow().y as f32,
            ),
            modifiers,
        }),
        NSEventType::NSLeftMouseDragged => {
            window_height.map(|window_height| Event::LeftMouseDragged {
                position: vec2f(
                    native_event.locationInWindow().x as f32,
                    window_height - native_event.locationInWindow().y as f32,
                ),
                modifiers,
            })
        }
        // TODO: This option is deprecated by Apple in favour of NSEventTypeOtherMouseDown
        // but we'll likely need to update cocoa.
        // See https://developer.apple.com/documentation/appkit/nsothermousedown.
        NSEventType::NSOtherMouseDown => {
            let window_height = window_height?;
            let window_location = native_event.locationInWindow();
            let position = vec2f(
                window_location.x as f32,
                window_height - (window_location.y as f32),
            );
            let modifier_flags = native_event.modifierFlags();
            let cmd = modifier_flags.contains(NSEventModifierFlags::NSCommandKeyMask);
            let shift = modifier_flags.contains(NSEventModifierFlags::NSShiftKeyMask);
            let click_count = native_event.clickCount() as u32;

            match native_event.buttonNumber() {
                2 => Some(Event::MiddleMouseDown {
                    position,
                    cmd,
                    shift,
                    click_count,
                }),
                3 => Some(Event::BackMouseDown {
                    position,
                    cmd,
                    shift,
                    click_count,
                }),
                4 => Some(Event::ForwardMouseDown {
                    position,
                    cmd,
                    shift,
                    click_count,
                }),
                _ => None,
            }
        }
        // For trackpads, this event will get triggered by the user's secondary click setting.
        NSEventType::NSRightMouseDown => window_height.map(|window_height| Event::RightMouseDown {
            position: vec2f(
                native_event.locationInWindow().x as f32,
                window_height - native_event.locationInWindow().y as f32,
            ),
            cmd: native_event
                .modifierFlags()
                .contains(NSEventModifierFlags::NSCommandKeyMask),
            shift: native_event
                .modifierFlags()
                .contains(NSEventModifierFlags::NSShiftKeyMask),
            click_count: native_event.clickCount() as u32,
        }),
        NSEventType::NSScrollWheel => window_height.map(|window_height| Event::ScrollWheel {
            position: vec2f(
                native_event.locationInWindow().x as f32,
                window_height - native_event.locationInWindow().y as f32,
            ),
            delta: vec2f(
                native_event.scrollingDeltaX() as f32,
                native_event.scrollingDeltaY() as f32,
            ),
            precise: native_event.hasPreciseScrollingDeltas() == YES,
            modifiers,
        }),
        _ => None,
    }
}
