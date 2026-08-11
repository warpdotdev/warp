use std::slice;
use std::str::Utf8Error;

use core_foundation::base::TCFType;
use core_graphics::base::CGFloat;
use core_graphics::color::CGColor;
use core_graphics::sys::CGColorRef;
use objc::runtime::Object;
use objc2_app_kit::{
    NSDeleteFunctionKey, NSDownArrowFunctionKey, NSEndFunctionKey, NSF1FunctionKey,
    NSF2FunctionKey, NSF3FunctionKey, NSF4FunctionKey, NSF5FunctionKey, NSF6FunctionKey,
    NSF7FunctionKey, NSF8FunctionKey, NSF9FunctionKey, NSF10FunctionKey, NSF11FunctionKey,
    NSF12FunctionKey, NSF13FunctionKey, NSF14FunctionKey, NSF15FunctionKey, NSF16FunctionKey,
    NSF17FunctionKey, NSF18FunctionKey, NSF19FunctionKey, NSF20FunctionKey, NSHelpFunctionKey,
    NSHomeFunctionKey, NSInsertFunctionKey, NSLeftArrowFunctionKey, NSPageDownFunctionKey,
    NSPageUpFunctionKey, NSRightArrowFunctionKey, NSUpArrowFunctionKey,
};
use objc2_foundation::{NSString, NSUTF8StringEncoding};
use pathfinder_color::ColorU;

// AppKit exposes the function-key Unicode values as `c_uint`, but the lookup
// below compares them against a `u16` code unit. Narrow each one to `u16`;
// every value lies in the 0xF700..=0xF8FF private-use range and fits losslessly.
const ARROW_UP_KEY: u16 = NSUpArrowFunctionKey as u16;
const ARROW_DOWN_KEY: u16 = NSDownArrowFunctionKey as u16;
const ARROW_LEFT_KEY: u16 = NSLeftArrowFunctionKey as u16;
const ARROW_RIGHT_KEY: u16 = NSRightArrowFunctionKey as u16;
const HOME_KEY: u16 = NSHomeFunctionKey as u16;
const END_KEY: u16 = NSEndFunctionKey as u16;
const PAGE_UP_KEY: u16 = NSPageUpFunctionKey as u16;
const PAGE_DOWN_KEY: u16 = NSPageDownFunctionKey as u16;
const HELP_KEY: u16 = NSHelpFunctionKey as u16;
const INSERT_KEY: u16 = NSInsertFunctionKey as u16;
const DELETE_KEY: u16 = NSDeleteFunctionKey as u16;
const F1_FUNCTION_KEY: u16 = NSF1FunctionKey as u16;
const F2_FUNCTION_KEY: u16 = NSF2FunctionKey as u16;
const F3_FUNCTION_KEY: u16 = NSF3FunctionKey as u16;
const F4_FUNCTION_KEY: u16 = NSF4FunctionKey as u16;
const F5_FUNCTION_KEY: u16 = NSF5FunctionKey as u16;
const F6_FUNCTION_KEY: u16 = NSF6FunctionKey as u16;
const F7_FUNCTION_KEY: u16 = NSF7FunctionKey as u16;
const F8_FUNCTION_KEY: u16 = NSF8FunctionKey as u16;
const F9_FUNCTION_KEY: u16 = NSF9FunctionKey as u16;
const F10_FUNCTION_KEY: u16 = NSF10FunctionKey as u16;
const F11_FUNCTION_KEY: u16 = NSF11FunctionKey as u16;
const F12_FUNCTION_KEY: u16 = NSF12FunctionKey as u16;
const F13_FUNCTION_KEY: u16 = NSF13FunctionKey as u16;
const F14_FUNCTION_KEY: u16 = NSF14FunctionKey as u16;
const F15_FUNCTION_KEY: u16 = NSF15FunctionKey as u16;
const F16_FUNCTION_KEY: u16 = NSF16FunctionKey as u16;
const F17_FUNCTION_KEY: u16 = NSF17FunctionKey as u16;
const F18_FUNCTION_KEY: u16 = NSF18FunctionKey as u16;
const F19_FUNCTION_KEY: u16 = NSF19FunctionKey as u16;
const F20_FUNCTION_KEY: u16 = NSF20FunctionKey as u16;

const BACKSPACE_KEY: u16 = 0x7f;
const ENTER_KEY: u16 = 0x0d;
const NUMPAD_ENTER_KEY: u16 = 0x03;
const ESCAPE_KEY: u16 = 0x1b;
const TAB_KEY: u16 = '\t' as u16;
const SHIFTED_TAB_KEY: u16 = 0x19;
unsafe extern "C" {
    fn CGColorGetComponents(color: CGColorRef) -> *const CGFloat;
}

pub fn unicode_char_to_key(char: u16) -> Option<&'static str> {
    // Control character naming needs to be in sync with the corresponding
    // objective-c definition in `keycode.m`. See:
    // https://github.com/warpdotdev/warp-internal/blob/master/ui/src/platform/mac/objc/keycode.m#L17
    match char {
        ARROW_UP_KEY => Some("up"),
        ARROW_DOWN_KEY => Some("down"),
        ARROW_LEFT_KEY => Some("left"),
        ARROW_RIGHT_KEY => Some("right"),
        HOME_KEY => Some("home"),
        END_KEY => Some("end"),
        PAGE_UP_KEY => Some("pageup"),
        PAGE_DOWN_KEY => Some("pagedown"),
        BACKSPACE_KEY => Some("backspace"),
        ENTER_KEY => Some("enter"),
        // Mac treats the help key as synonymous with the insert key.
        HELP_KEY | INSERT_KEY => Some("insert"),
        DELETE_KEY => Some("delete"),
        ESCAPE_KEY => Some("escape"),
        TAB_KEY => Some("tab"),
        SHIFTED_TAB_KEY => Some("tab"),
        NUMPAD_ENTER_KEY => Some("numpadenter"),
        F1_FUNCTION_KEY => Some("f1"),
        F2_FUNCTION_KEY => Some("f2"),
        F3_FUNCTION_KEY => Some("f3"),
        F4_FUNCTION_KEY => Some("f4"),
        F5_FUNCTION_KEY => Some("f5"),
        F6_FUNCTION_KEY => Some("f6"),
        F7_FUNCTION_KEY => Some("f7"),
        F8_FUNCTION_KEY => Some("f8"),
        F9_FUNCTION_KEY => Some("f9"),
        F10_FUNCTION_KEY => Some("f10"),
        F11_FUNCTION_KEY => Some("f11"),
        F12_FUNCTION_KEY => Some("f12"),
        F13_FUNCTION_KEY => Some("f13"),
        F14_FUNCTION_KEY => Some("f14"),
        F15_FUNCTION_KEY => Some("f15"),
        F16_FUNCTION_KEY => Some("f16"),
        F17_FUNCTION_KEY => Some("f17"),
        F18_FUNCTION_KEY => Some("f18"),
        F19_FUNCTION_KEY => Some("f19"),
        F20_FUNCTION_KEY => Some("f20"),
        _ => None,
    }
}

/// # Safety
///
/// This code is only unsafe since it requires interfacing with platform code.
pub unsafe fn nsstring_as_str<'a>(nsstring: *const Object) -> Result<&'a str, Utf8Error> {
    unsafe {
        // The caller guarantees `nsstring` points at a live Objective-C string, so
        // reinterpret it as an `NSString` for typed access to its UTF-8 bytes.
        let nsstring = &*nsstring.cast::<NSString>();
        let cstr = nsstring.UTF8String();
        let len = nsstring.lengthOfBytesUsingEncoding(NSUTF8StringEncoding);
        std::str::from_utf8(slice::from_raw_parts(cstr.cast::<u8>(), len))
    }
}

pub fn color_u_to_cg_color(color: ColorU) -> CGColor {
    CGColor::rgb(
        f64::from(color.r) / 255.,
        f64::from(color.g) / 255.,
        f64::from(color.b) / 255.,
        f64::from(color.a) / 255.,
    )
}

pub fn cg_color_to_color_u(color: CGColor) -> ColorU {
    unsafe {
        let components = CGColorGetComponents(color.as_concrete_TypeRef());

        ColorU::new(
            (*components.offset(0) * 255.) as u8,
            (*components.offset(1) * 255.) as u8,
            (*components.offset(2) * 255.) as u8,
            (*components.offset(3) * 255.) as u8,
        )
    }
}
