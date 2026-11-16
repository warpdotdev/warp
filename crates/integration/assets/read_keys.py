#!/usr/bin/env python3
"""Baseline key-reading script without the Kitty keyboard protocol.

Reads raw bytes from stdin and prints each as hex. Used by
test_keyboard_protocol_disabled_shift_enter to verify that Shift+Enter
and plain Enter produce legacy byte values (0x0a and 0x0d) when the
protocol is not enabled.
"""
import sys
import termios
import tty

# Put terminal in raw mode
old_settings = termios.tcgetattr(sys.stdin)
try:
    tty.setraw(sys.stdin.fileno())
    print("Ready. Press Shift+Enter, then plain Enter, then Ctrl+C")
    sys.stdout.flush()

    bytes_received = []
    while True:
        char = sys.stdin.read(1)
        byte_val = ord(char)
        bytes_received.append(byte_val)

        # Print each byte in hex
        print(f"\nReceived byte: 0x{byte_val:02x} ({chr(byte_val) if 32 <= byte_val < 127 else repr(chr(byte_val))})")
        sys.stdout.flush()

        # Exit on Ctrl+C
        if byte_val == 3:
            break
finally:
    termios.tcsetattr(sys.stdin, termios.TCSADRAIN, old_settings)
    print("\nDone!")
