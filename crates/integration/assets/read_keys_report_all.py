#!/usr/bin/env python3
"""Key-reading script with Kitty keyboard protocol flag 8 only (report all keys).

Enables flag 8 via CSI =8u, then reads raw bytes and identifies both CSI u
sequences and legacy arrow key sequences. Used by
test_keyboard_protocol_report_all_keys_printable_and_cursor to verify that
printable keys produce CSI u, cursor keys use legacy encoding, and multi-byte
UTF-8 characters (e.g. é) are handled correctly.
"""
import sys
import termios
import tty

# Put terminal in raw mode
old_settings = termios.tcgetattr(sys.stdin)
try:
    tty.setraw(sys.stdin.fileno())

    # Enable keyboard protocol with flag 8 only (report all keys as escape codes).
    # This means every key press gets a CSI u sequence, but without disambiguate (flag 1).
    sys.stdout.write('\x1b[=8u')
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

        # Also check for 'u' at end of CSI sequence to print summary
        if byte_val == ord('u'):
            esc_start = len(bytes_received) - 1
            while esc_start > 0 and bytes_received[esc_start] != 0x1b:
                esc_start -= 1
            sequence = ''.join(chr(b) for b in bytes_received[esc_start:])
            print(f"\nComplete sequence: {repr(sequence)}")
            sys.stdout.flush()
            bytes_received = []

        # Check for legacy arrow key sequences (ESC [ A/B/C/D)
        if byte_val in (ord('A'), ord('B'), ord('C'), ord('D')) and len(bytes_received) >= 3:
            tail = bytes_received[-3:]
            if tail[0] == 0x1b and tail[1] == 0x5b:
                sequence = ''.join(chr(b) for b in tail)
                print(f"\nLegacy arrow: {repr(sequence)}")
                sys.stdout.flush()
                bytes_received = []

finally:
    termios.tcsetattr(sys.stdin, termios.TCSADRAIN, old_settings)
    # Disable keyboard protocol
    sys.stdout.write('\x1b[=0u')
    print("\nDone!")
