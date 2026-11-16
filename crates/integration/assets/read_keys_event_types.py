#!/usr/bin/env python3
"""Key-reading script with Kitty keyboard protocol flags 1+2+8 (disambiguate + event types + report all).

Enables flags 11 via CSI =11u, then reads raw bytes and reassembles CSI u
sequences. Used by test_keyboard_protocol_modifier_key_reporting and
test_keyboard_protocol_modifier_self_bit to verify that standalone modifier
key press/release events produce CSI u sequences with the correct event type
field (:1 for press, :3 for release) and self-bit modifier encoding.
"""
import sys
import termios
import tty

# Put terminal in raw mode
old_settings = termios.tcgetattr(sys.stdin)
try:
    tty.setraw(sys.stdin.fileno())

    # Enable keyboard protocol with flags 1+2+8=11
    # 1 = disambiguate escape codes
    # 2 = report event types (press/repeat/release)
    # 8 = report all keys as escape codes
    sys.stdout.write('\x1b[=11u')
    sys.stdout.flush()

    print("Protocol enabled. Press keys then Ctrl+C")
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

        # Check for 'u' at end of CSI sequence to print summary
        if byte_val == ord('u'):
            esc_start = len(bytes_received) - 1
            while esc_start > 0 and bytes_received[esc_start] != 0x1b:
                esc_start -= 1
            sequence = ''.join(chr(b) for b in bytes_received[esc_start:])
            print(f"\nComplete sequence: {repr(sequence)}")
            sys.stdout.flush()
            bytes_received = []

finally:
    termios.tcsetattr(sys.stdin, termios.TCSADRAIN, old_settings)
    # Disable keyboard protocol
    sys.stdout.write('\x1b[=0u')
    print("\nDone!")
