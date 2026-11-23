#!/usr/bin/env python3
"""B0 spike: a simulated human at the X server (XTEST: real input through
the window system, as a person's mouse and keyboard). Usage:
  human.py click X Y
  human.py type TEXT [delay_ms]
  human.py key KEYSYM"""
import sys, time
from Xlib import X, XK, display
from Xlib.ext import xtest

d = display.Display()


def click(x, y):
    xtest.fake_input(d, X.MotionNotify, x=x, y=y)
    d.sync()
    time.sleep(0.05)
    xtest.fake_input(d, X.ButtonPress, 1)
    d.sync()
    time.sleep(0.03)
    xtest.fake_input(d, X.ButtonRelease, 1)
    d.sync()


def key(combo):
    """A key or chord: Return, ctrl+l, ctrl+shift+Tab."""
    codes = [d.keysym_to_keycode(XK.string_to_keysym({"ctrl": "Control_L", "shift": "Shift_L", "alt": "Alt_L"}.get(k, k)))
             for k in combo.split("+")]
    for c in codes:
        xtest.fake_input(d, X.KeyPress, c)
    d.sync()
    for c in reversed(codes):
        xtest.fake_input(d, X.KeyRelease, c)
    d.sync()


def typ(text, delay):
    shift_code = d.keysym_to_keycode(XK.XK_Shift_L)
    for ch in text:
        sym = ord(ch)  # Latin-1 keysyms are their code points
        code = d.keysym_to_keycode(sym)
        shift = d.keycode_to_keysym(code, 0) != sym
        if shift:
            xtest.fake_input(d, X.KeyPress, shift_code)
        xtest.fake_input(d, X.KeyPress, code)
        xtest.fake_input(d, X.KeyRelease, code)
        if shift:
            xtest.fake_input(d, X.KeyRelease, shift_code)
        d.sync()
        time.sleep(delay)


cmd = sys.argv[1]
if cmd == "click":
    click(int(sys.argv[2]), int(sys.argv[3]))
elif cmd == "type":
    typ(sys.argv[2], int(sys.argv[3]) / 1000 if len(sys.argv) > 3 else 0.06)
elif cmd == "key":
    key(sys.argv[2])
