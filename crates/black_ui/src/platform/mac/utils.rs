use std::os::raw::c_char;
use std::slice;
use std::str::Utf8Error;

use cocoa::appkit::{
    NSDeleteFunctionKey as DELETE_KEY, NSDownArrowFunctionKey as ARROW_DOWN_KEY,
    NSEndFunctionKey as END_KEY, NSF10FunctionKey as F10_FUNCTION_KEY,
    NSF11FunctionKey as F11_FUNCTION_KEY, NSF12FunctionKey as F12_FUNCTION_KEY,
    NSF13FunctionKey as F13_FUNCTION_KEY, NSF14FunctionKey as F14_FUNCTION_KEY,
    NSF15FunctionKey as F15_FUNCTION_KEY, NSF16FunctionKey as F16_FUNCTION_KEY,
    NSF17FunctionKey as F17_FUNCTION_KEY, NSF18FunctionKey as F18_FUNCTION_KEY,
    NSF19FunctionKey as F19_FUNCTION_KEY, NSF1FunctionKey as F1_FUNCTION_KEY,
    NSF20FunctionKey as F20_FUNCTION_KEY, NSF2FunctionKey as F2_FUNCTION_KEY,
    NSF3FunctionKey as F3_FUNCTION_KEY, NSF4FunctionKey as F4_FUNCTION_KEY,
    NSF5FunctionKey as F5_FUNCTION_KEY, NSF6FunctionKey as F6_FUNCTION_KEY,
    NSF7FunctionKey as F7_FUNCTION_KEY, NSF8FunctionKey as F8_FUNCTION_KEY,
    NSF9FunctionKey as F9_FUNCTION_KEY, NSHelpFunctionKey as HELP_KEY,
    NSHomeFunctionKey as HOME_KEY, NSInsertFunctionKey as INSERT_KEY,
    NSLeftArrowFunctionKey as ARROW_LEFT_KEY, NSPageDownFunctionKey as PAGE_DOWN_KEY,
    NSPageUpFunctionKey as PAGE_UP_KEY, NSRightArrowFunctionKey as ARROW_RIGHT_KEY,
    NSUpArrowFunctionKey as ARROW_UP_KEY,
};
use core_foundation::base::TCFType;
use core_graphics::base::CGFloat;
use core_graphics::color::CGColor;
use core_graphics::sys::CGColorRef;
use objc::runtime::Object;
use objc::{msg_send, sel, sel_impl};
use pathfinder_color::ColorU;

const BACKSPACE_KEY: u16 = 0x7f;
const ENTER_KEY: u16 = 0x0d;
const NUMPAD_ENTER_KEY: u16 = 0x03;
const ESCAPE_KEY: u16 = 0x1b;
const TAB_KEY: u16 = '\t' as u16;
const SHIFTED_TAB_KEY: u16 = 0x19;
extern "C" {
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
    const UTF8_ENCODING: usize = 4;

    let cstr: *const c_char = msg_send![nsstring, UTF8String];
    let len: usize = msg_send![nsstring, lengthOfBytesUsingEncoding: UTF8_ENCODING];
    std::str::from_utf8(slice::from_raw_parts(cstr as *const u8, len))
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
