#!/usr/bin/env python3
"""Key-reading script with Kitty keyboard protocol flags 1+8 (disambiguate + report all).

Enables protocol flags 9 via CSI =9u, then reads raw bytes and reassembles
CSI u sequences. Used by test_keyboard_protocol_enabled_shift_enter and
test_keyboard_protocol_enabled_shifted_symbol_uses_unshifted_keycode to
verify that keys produce the expected CSI u encodings.
"""
import sys
import termios
import tty

# Put terminal in raw mode
old_settings = termios.tcgetattr(sys.stdin)
try:
    tty.setraw(sys.stdin.fileno())

    # Enable keyboard protocol with flags:
    # - 1 = disambiguate escape codes
    # - 8 = report all keys as escape codes (including unmodified keys)
    # Total flags = 9 (1 + 8)
    # Use the set-flags form (CSI = flags u), which replaces active flags.
    sys.stdout.write('\x1b[=9u')
    sys.stdout.flush()

    print("Protocol enabled. Press Shift+Enter, then plain Enter, then Ctrl+C")
    sys.stdout.flush()

    bytes_received = []
    while True:
        char = sys.stdin.read(1)
        byte_val = ord(char)
        bytes_received.append(byte_val)

        # Print each byte in hex
        if byte_val == 0x1b:  # ESC
            print(f"\nESC sequence start: 0x{byte_val:02x}")
        else:
            print(f"0x{byte_val:02x}", end=' ')
        sys.stdout.flush()

        # Exit on Ctrl+C (0x03)
        if byte_val == 3:
            break

        # Also check for 'u' at end of CSI sequence to print summary
        if byte_val == ord('u'):
            # Extract the last CSI sequence
            esc_start = len(bytes_received) - 1
            while esc_start > 0 and bytes_received[esc_start] != 0x1b:
                esc_start -= 1
            sequence = ''.join(chr(b) for b in bytes_received[esc_start:])
            print(f"\nComplete sequence: {repr(sequence)}")
            sys.stdout.flush()
            bytes_received = []

finally:
    termios.tcsetattr(sys.stdin, termios.TCSADRAIN, old_settings)
    # Disable keyboard protocol by replacing flags with 0.
    sys.stdout.write('\x1b[=0u')
    print("\nDone!")
