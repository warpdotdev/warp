#include <AppKit/AppKit.h>
#include <Carbon/Carbon.h>
#include <CoreFoundation/CoreFoundation.h>

// One virtual key could map to multiple physical keys on the keyboard. So we keep
// a mapping from key name to an array of keycodes.
NSMutableDictionary<NSString*, NSMutableArray<NSNumber*>*>* keycodeDict;

BOOL IsUnicodeControl(unichar c) {
    // C0 control characters: http://unicode.org/charts/PDF/U0000.pdf
    // C1 control characters: http://unicode.org/charts/PDF/U0080.pdf
    return c <= 0x1F || (c >= 0x7F && c <= 0x9F);
}

// Control character naming needs to be in sync with the corresponding rust definition in
// `event.rs`: see
// https://github.com/warpdotdev/warp-internal/blob/master/ui/src/platform/mac/utils.rs#L42
// The list of control characters are referenced from chromium code here:
// https://chromium.googlesource.com/chromium/src/+/lkgr/ui/events/keycodes/keyboard_code_conversion_mac.mm#329
NSString* KeyFromControlKeyCode(unsigned short keyCode) {
    switch (keyCode) {
        case kVK_ANSI_KeypadEnter:
            return @"numpadenter";
        case kVK_Return:
            return @"enter";
        case kVK_Tab:
            return @"tab";
        case kVK_Delete:
            return @"backspace";
        case kVK_Escape:
            return @"escape";
        case kVK_F1:
            return @"f1";
        case kVK_F2:
            return @"f2";
        case kVK_F3:
            return @"f3";
        case kVK_F4:
            return @"f4";
        case kVK_F5:
            return @"f5";
        case kVK_F6:
            return @"f6";
        case kVK_F7:
            return @"f7";
        case kVK_F8:
            return @"f8";
        case kVK_F9:
            return @"f9";
        case kVK_F10:
            return @"f10";
        case kVK_F11:
            return @"f11";
        case kVK_F12:
            return @"f12";
        case kVK_F13:
            return @"f13";
        case kVK_F14:
            return @"f14";
        case kVK_F15:
            return @"f15";
        case kVK_F16:
            return @"f16";
        case kVK_F17:
            return @"f17";
        case kVK_F18:
            return @"f18";
        case kVK_F19:
            return @"f19";
        case kVK_F20:
            return @"f20";
        case kVK_ForwardDelete:
            return @"delete";
        case kVK_Help:
            return @"insert";
        case kVK_Home:
            return @"home";
        case kVK_PageUp:
            return @"pageup";
        case kVK_End:
            return @"end";
        case kVK_PageDown:
            return @"pagedown";
        case kVK_LeftArrow:
            return @"left";
        case kVK_RightArrow:
            return @"right";
        case kVK_DownArrow:
            return @"down";
        case kVK_UpArrow:
            return @"up";
        default:
            return nil;
    }
}

// Helper function to get the keyboard layout data
CFDataRef GetKeyboardLayoutData() {
    TISInputSourceRef source = TISCopyCurrentKeyboardInputSource();
    CFDataRef layout_data =
        (CFDataRef)(TISGetInputSourceProperty(source, kTISPropertyUnicodeKeyLayoutData));
    if (!layout_data) {
        // TISGetInputSourceProperty returns null with some keyboard layouts (e.g. Japanese and
        // Chinese). Using TISCopyCurrentKeyboardLayoutInputSource to fix NULL return.
        source = TISCopyCurrentKeyboardLayoutInputSource();
        layout_data =
            (CFDataRef)(TISGetInputSourceProperty(source, kTISPropertyUnicodeKeyLayoutData));
    }
    return layout_data;
}

// Referenced from chromium:
// https://chromium.googlesource.com/chromium/src/+/lkgr/ui/events/keycodes/keyboard_code_conversion_mac.mm
// Here we take the keyboard layout, keycode, modifier keys, and keyboard type to
// determine the output character.
// Notice that we don't yet handle multiple character case here. But this should
// be minor given Chrome also doesn't support it.
UniChar TranslatedUnicodeCharFromKeyCode(CFDataRef layout_data, UInt16 key_code,
                                         UInt32 modifier_key_state, UInt32 keyboard_type) {
    if (!layout_data) return 0xFFFD;  // REPLACEMENT CHARACTER

    const UCKeyboardLayout* keyboardLayout = (const UCKeyboardLayout*)CFDataGetBytePtr(layout_data);

    UInt32 deadKeyState = 0;
    UniCharCount maxStringLength = 255;
    UniCharCount actualStringLength = 0;
    UniChar unicodeString[maxStringLength];

    UCKeyTranslate(keyboardLayout, key_code, kUCKeyActionDown, modifier_key_state, keyboard_type,
                   kUCKeyTranslateNoDeadKeysBit, &deadKeyState, maxStringLength,
                   &actualStringLength, unicodeString);
    // TODO(kevin): Handle multiple character case. Should be rare.
    return unicodeString[0];
}

// Convert keycode to its corresponding character on the keyboard.
NSString* keyCodeToChar(UInt16 keyCode, BOOL shifted) {
    UInt32 modifier_key_state = 0;

    // The shift key representation in Carbon is 1 << 9.
    // However, UCKeyTranslate takes the modifier keys and shift them by 8 bits. So we
    // only need to pass in 1 << 1 here.
    if (shifted) {
        modifier_key_state = 1 << 1;
    }

    CFDataRef layout_data = GetKeyboardLayoutData();
    UniChar translated_char =
        TranslatedUnicodeCharFromKeyCode(layout_data, keyCode, modifier_key_state, LMGetKbdLast());

    // UCKeyTranslate can't translate control characters like function keys and arrow
    // keys. We keep a separate mapping for this case. This is the same behavior as chromium:
    // https://chromium.googlesource.com/chromium/src/+/lkgr/ui/events/keycodes/keyboard_code_conversion_mac.mm#873
    if (IsUnicodeControl(translated_char)) {
        return KeyFromControlKeyCode(keyCode);
    } else {
        return [NSString stringWithFormat:@"%C", translated_char];
    }
}

NSArray<NSNumber*>* charToKeyCodes(NSString* keyChar) {
    if (keycodeDict == nil) {
        keycodeDict = [[NSMutableDictionary alloc] init];
        CFDataRef layout_data = GetKeyboardLayoutData();

        // For every keycode.
        size_t i;
        for (i = 0; i < 128; ++i) {
            UInt32 shift_key = 1 << 1;

            // Compute a shifted and unshifted version for one keycode.
            UniChar unshifted =
                TranslatedUnicodeCharFromKeyCode(layout_data, (UInt16)i, 0, LMGetKbdLast());
            UniChar shifted =
                TranslatedUnicodeCharFromKeyCode(layout_data, (UInt16)i, shift_key, LMGetKbdLast());

            NSString* unshifted_str;
            if (IsUnicodeControl(unshifted)) {
                unshifted_str = KeyFromControlKeyCode(i);
            } else {
                unshifted_str = [NSString stringWithFormat:@"%C", unshifted];
            }

            NSString* shifted_str;
            if (IsUnicodeControl(shifted)) {
                shifted_str = KeyFromControlKeyCode(i);
            } else {
                shifted_str = [NSString stringWithFormat:@"%C", shifted];
            }

            if (unshifted_str != nil && [unshifted_str length] > 0) {
                if ([keycodeDict objectForKey:unshifted_str] == nil) {
                    [keycodeDict setObject:[[[NSMutableArray alloc] init] autorelease]
                                    forKey:unshifted_str];
                }
                NSMutableArray* keycodes = [keycodeDict objectForKey:unshifted_str];
                [keycodes addObject:[NSNumber numberWithInt:i]];
            }

            if (shifted_str != nil && [shifted_str length] > 0) {
                if ([keycodeDict objectForKey:shifted_str] == nil) {
                    [keycodeDict setObject:[[[NSMutableArray alloc] init] autorelease]
                                    forKey:shifted_str];
                }
                NSMutableArray* keycodes = [keycodeDict objectForKey:shifted_str];
                [keycodes addObject:[NSNumber numberWithInt:i]];
            }
        }
    }

    return [keycodeDict objectForKey:keyChar];
}
