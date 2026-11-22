#!/usr/bin/env python3
"""Raw keyboard and mouse recordings, recorded from the real devices and replayed into an X display through XTEST,
so two game clients get exactly the same input (tools/client_packet_parity.py).

  input_recording.py record run.jsonl --device /dev/input/event5 --device /dev/input/event7   # F8 marks, F9 stops
  input_recording.py record run.jsonl --all-devices      # every keyboard and mouse this user can read
  input_recording.py replay run.jsonl --display :42      # into that display's focused window
  input_recording.py show run.jsonl

A recording is JSON lines. The first is the header ({"format": "sunrise-input-recording", "version": 1}); every
other line is one event with `t`, seconds since the first marker:
  {"t": 0.0, "type": "marker", "route": "walk"}       # the replayer sends the chat command /wire-mark NN here
  {"t": 0.5, "type": "key", "code": "KEY_W", "down": true}
  {"t": 0.6, "type": "button", "code": "BTN_LEFT", "down": true}
  {"t": 0.7, "type": "rel", "dx": 12, "dy": -3}       # raw mouse counts, as the device reported them
  {"t": 0.8, "type": "wheel", "dy": -1}               # one notch; negative is towards the user
Codes are evdev's names (KEY_*, BTN_*). Key repeats are not recorded: the replay display has autorepeat off.
NN is the route's place in the recording, from 01, so a diff names each route by its own marker. A route's input
starts MARKER_SECONDS after its marker at the earliest, once the marker's command has been typed: `routes` refuses
a recording whose input comes sooner, and `record` keeps that time free after each marker.

Recording reads /dev/input (the `input` group) without grabbing the devices: the game keeps getting them, and so does
the recording, from every window, passwords included, so it names its devices (or --all-devices) and its file is
readable by this user only. It starts at the first marker key and stops at the stop key; the marker and stop keys
themselves are not recorded, and an existing file is never overwritten.

Replay sets the display's pointer acceleration to 1 so a relative move reaches the game as the same counts the
device sent (raw input), as on a desktop with a flat acceleration profile, and turns its key repeat off; it types
into whatever window has that display's focus. So give it a display of its own (Xvfb): it refuses only the display
it runs on ($DISPLAY) and :0, the ones it can tell are a desktop's.
"""
import argparse
import json
import os
import re
import sys
import time
from pathlib import Path

FORMAT = {"format": "sunrise-input-recording", "version": 1}
MARKER_COMMAND = "wire-mark"
# evdev names that don't map to an X keysym by their lower-case suffix.
KEYSYMS = {
    "KEY_LEFTCTRL": "Control_L", "KEY_RIGHTCTRL": "Control_R", "KEY_LEFTSHIFT": "Shift_L",
    "KEY_RIGHTSHIFT": "Shift_R", "KEY_LEFTALT": "Alt_L", "KEY_RIGHTALT": "Alt_R", "KEY_SPACE": "space",
    "KEY_ESC": "Escape", "KEY_ENTER": "Return", "KEY_TAB": "Tab", "KEY_BACKSPACE": "BackSpace",
    "KEY_SLASH": "slash", "KEY_MINUS": "minus", "KEY_EQUAL": "equal", "KEY_DOT": "period", "KEY_COMMA": "comma",
    "KEY_SEMICOLON": "semicolon", "KEY_APOSTROPHE": "apostrophe", "KEY_GRAVE": "grave",
    "KEY_LEFTBRACE": "bracketleft", "KEY_RIGHTBRACE": "bracketright", "KEY_BACKSLASH": "backslash",
    "KEY_UP": "Up", "KEY_DOWN": "Down", "KEY_LEFT": "Left", "KEY_RIGHT": "Right", "KEY_CAPSLOCK": "Caps_Lock",
    "KEY_DELETE": "Delete", "KEY_HOME": "Home", "KEY_END": "End", "KEY_PAGEUP": "Prior", "KEY_PAGEDOWN": "Next",
    "KEY_INSERT": "Insert",
}
BUTTONS = {"BTN_LEFT": 1, "BTN_MIDDLE": 2, "BTN_RIGHT": 3, "BTN_SIDE": 8, "BTN_EXTRA": 9}
# Characters the marker command types, as evdev keys (a US layout, Xvfb's default).
CHARS = {"/": "KEY_SLASH", "-": "KEY_MINUS", " ": "KEY_SPACE"}
# A route's name becomes a file name in a run's folder: one plain component.
ROUTE_NAME = re.compile(r"[a-z0-9][a-z0-9_]{0,47}")


def keysym_name(code):
    if code in KEYSYMS:
        return KEYSYMS[code]
    name = code.removeprefix("KEY_")
    if len(name) == 1:
        return name.lower()
    if name.startswith("F") and name[1:].isdigit():
        return name
    raise ValueError(f"no X keysym for {code}")


def char_key(c):
    return CHARS.get(c) or f"KEY_{c.upper()}"


def write(path, events):
    with open(os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600), "w") as out:
        out.write(json.dumps(FORMAT) + "\n")
        for e in sorted(events, key=lambda e: e["t"]):
            out.write(json.dumps(e) + "\n")


def read(path):
    lines = [json.loads(line) for line in Path(path).read_text().splitlines() if line.strip()]
    if not lines or {k: lines[0].get(k) for k in FORMAT} != FORMAT:
        raise ValueError(f"{path}: not a {FORMAT['format']} v{FORMAT['version']} file")
    return lines[1:]


KINDS = {"marker", "key", "button", "rel", "wheel"}


def check_event(e):
    """Refuses an event replay couldn't send as it says: an unknown kind, a key or button with no X code, a move
    or a wheel that isn't whole counts."""
    kind = e.get("type")
    if kind not in KINDS or not isinstance(e.get("t"), (int, float)):
        raise ValueError(f"event {e!r}: an unknown kind or no time")
    if kind == "key":
        keysym_name(e["code"])
    elif kind == "button" and e.get("code") not in BUTTONS:
        raise ValueError(f"event {e!r}: no X button for {e.get('code')}")
    elif kind == "rel" and not (isinstance(e.get("dx"), int) and isinstance(e.get("dy"), int)):
        raise ValueError(f"event {e!r}: a move is whole counts dx, dy")
    elif kind == "wheel" and not isinstance(e.get("dy"), int):
        raise ValueError(f"event {e!r}: a wheel turn is whole notches dy")
    if kind in ("key", "button") and not isinstance(e.get("down"), bool):
        raise ValueError(f"event {e!r}: down is true or false")


def routes(events):
    """The recording cut at its markers: [(route name, events with t from that marker)]. Every event is checked
    first (check_event); input before the first marker, a name that isn't one plain file name, a repeated name, or
    input sooner than MARKER_SECONDS after its marker is refused."""
    for e in events:
        check_event(e)
    out = []
    for e in events:
        if not out and e["type"] != "marker":
            raise ValueError(f"input at {e['t']:.3f} s comes before the first marker")
        if e["type"] == "marker":
            out.append((e.get("route", f"route{len(out) + 1}"), [], e["t"]))
        if out:
            out[-1][1].append(dict(e, t=e["t"] - out[-1][2]))
    names = [name for name, _, _ in out]
    for name, evs, _ in out:
        if not ROUTE_NAME.fullmatch(name) or names.count(name) > 1:
            raise ValueError(f"route name {name!r}: one per route, lower-case letters, digits and _")
        early = [e for e in evs if e["type"] != "marker" and e["t"] < MARKER_SECONDS - 1e-6]
        if early:
            raise ValueError(f"{name}: input at {early[0]['t']:.3f} s, while the marker is typed (until "
                             f"{MARKER_SECONDS:.2f} s)")
    if len(out) > 99:
        raise ValueError("at most 99 routes: a marker numbers its route in two digits")
    return [(name, evs) for name, evs, _ in out]


def mark(number):
    """The marker command of the recording's route `number` (from 1)."""
    return f"{MARKER_COMMAND} {number:02d}"


def marker_events(t, command=mark(1)):
    """The keys that send `/command` from the chat: T opens it, the text, Enter. 60 ms a key, so a 20 fps client
    still sees each press and release in its own frame."""
    out, step = [], 0.06
    keys = ["KEY_T"] + [char_key(c) for c in "/" + command] + ["KEY_ENTER"]
    for i, key in enumerate(keys):
        start = t + i * 2 * step + (0.3 if i else 0)
        out += [{"t": start, "type": "key", "code": key, "down": True},
                {"t": start + step, "type": "key", "code": key, "down": False}]
    return out


MARKER_SECONDS = 0.3 + (len(mark(1)) + 3) * 0.12 + 0.6
# When Enter sends the marker, from the marker event: a route's ticks start there.
MARKER_SENT = 0.3 + (len(mark(1)) + 2) * 0.12


class Injector:
    """XTEST input into one X display: into `window` (an X window id), given the focus before each route and checked
    to have it at the route's start and end, or else whatever window has the display's focus."""

    def __init__(self, display_name, window=None):
        from Xlib import X, display
        from Xlib.ext import xtest
        self.X, self.xtest = X, xtest
        self.d = display.Display(display_name)
        if not self.d.has_extension("XTEST"):
            raise RuntimeError(f"{display_name} has no XTEST")
        self.d.change_pointer_control(accel=(1, 1), threshold=1)
        self.d.change_keyboard_control(auto_repeat_mode=X.AutoRepeatModeOff)
        self.held = set()
        self.window = window
        self.d.sync()

    def focus(self):
        """Gives `window` the input focus and checks it took it; nothing without a window."""
        if self.window is None:
            return
        target = self.d.create_resource_object("window", self.window)
        target.set_input_focus(self.X.RevertToParent, self.X.CurrentTime)
        self.d.sync()
        if self.d.get_input_focus().focus.id != self.window:
            raise RuntimeError(f"window {self.window:#x} did not take the input focus")

    def keycode(self, code):
        from Xlib import XK
        sym = XK.string_to_keysym(keysym_name(code))
        kc = self.d.keysym_to_keycode(sym)
        if not kc:
            raise ValueError(f"the display has no key for {code}")
        return kc

    def send(self, e):
        X, fake = self.X, self.xtest.fake_input
        kind = e["type"]
        if kind == "key":
            fake(self.d, X.KeyPress if e["down"] else X.KeyRelease, self.keycode(e["code"]))
            (self.held.add if e["down"] else self.held.discard)(("key", e["code"]))
        elif kind == "button":
            fake(self.d, X.ButtonPress if e["down"] else X.ButtonRelease, BUTTONS[e["code"]])
            (self.held.add if e["down"] else self.held.discard)(("button", e["code"]))
        elif kind == "rel":
            fake(self.d, X.MotionNotify, True, x=int(e["dx"]), y=int(e["dy"]))
        elif kind == "wheel":
            button = 4 if e["dy"] > 0 else 5
            for _ in range(abs(int(e["dy"]))):
                fake(self.d, X.ButtonPress, button)
                fake(self.d, X.ButtonRelease, button)
        self.d.flush()

    def release_all(self):
        for kind, code in sorted(self.held):
            self.send({"type": kind, "code": code, "down": False})
        self.held.clear()

    def play(self, events, start=None, number=1):
        """Sends each event at its time from `start` (default now); a marker becomes route `number`'s chat command.
        Whatever it pressed is released even when sending fails, and with a window, the route fails if that window
        lost the focus while it played."""
        for e in events:
            check_event(e)
        self.focus()
        expanded = []
        for e in events:
            expanded += marker_events(e["t"], mark(number)) if e["type"] == "marker" else [e]
        expanded.sort(key=lambda e: e["t"])
        start = time.perf_counter() if start is None else start
        try:
            for e in expanded:
                wait = start + e["t"] - time.perf_counter()
                if wait > 0.002:
                    time.sleep(wait - 0.001)
                while time.perf_counter() < start + e["t"]:
                    pass
                self.send(e)
        finally:
            self.release_all()
        if self.window is not None and self.d.get_input_focus().focus.id != self.window:
            raise RuntimeError(f"window {self.window:#x} lost the input focus during the route")
        return time.perf_counter() - start


def record(path, devices, marker, stop, all_devices=False):
    import evdev
    from evdev import ecodes
    from select import select
    if devices:
        found = [evdev.InputDevice(p) for p in devices]
    elif not all_devices:
        sys.exit("name the devices to record (--device), or --all-devices for every keyboard and mouse")
    else:
        found = []
        for p in evdev.list_devices():
            dev = evdev.InputDevice(p)
            caps = dev.capabilities()
            keys, rels = caps.get(ecodes.EV_KEY, []), caps.get(ecodes.EV_REL, [])
            if ecodes.KEY_W in keys or ecodes.REL_X in rels:
                found.append(dev)
    if not found:
        sys.exit("no keyboard or mouse readable in /dev/input (is this user in the `input` group?)")
    for dev in found:
        print(f"reading {dev.path}: {dev.name}", file=sys.stderr)
    print(f"press {marker} to start a route, {stop} to stop", file=sys.stderr)
    events, t0, rel, routes_seen = [], None, {}, 0
    by_fd = {dev.fd: dev for dev in found}
    while True:
        ready, _, _ = select(by_fd, [], [])
        for fd in ready:
            for ev in by_fd[fd].read():
                now = ev.timestamp()
                if ev.type == ecodes.EV_KEY and ev.value in (0, 1):
                    code = ecodes.KEY.get(ev.code) or ecodes.BTN.get(ev.code)
                    code = code[0] if isinstance(code, list) else code
                    if code == stop and ev.value == 1:
                        write(path, reserve_markers(events))
                        print(f"{len(events)} events, {routes_seen} routes: {path}", file=sys.stderr)
                        return
                    if code == marker:
                        if ev.value == 1:
                            t0 = now if t0 is None else t0
                            routes_seen += 1
                            events.append({"t": round(now - t0, 6), "type": "marker",
                                           "route": f"route{routes_seen}"})
                        continue
                    if t0 is None or code is None:
                        continue
                    kind = "button" if code.startswith("BTN_") else "key"
                    events.append({"t": round(now - t0, 6), "type": kind, "code": code, "down": ev.value == 1})
                elif ev.type == ecodes.EV_REL and t0 is not None:
                    if ev.code == ecodes.REL_WHEEL:
                        events.append({"t": round(now - t0, 6), "type": "wheel", "dy": ev.value})
                    elif ev.code in (ecodes.REL_X, ecodes.REL_Y):
                        rel.setdefault(fd, [now, 0, 0])
                        rel[fd][1 if ev.code == ecodes.REL_X else 2] += ev.value
                elif ev.type == ecodes.EV_SYN and fd in rel:
                    when, dx, dy = rel.pop(fd)
                    events.append({"t": round(when - t0, 6), "type": "rel", "dx": dx, "dy": dy})


def reserve_markers(events):
    """A recording with MARKER_SECONDS added after each marker, where the replay types it, so a route's input keeps
    its own timing and never meets the marker's keys."""
    out, shift = [], 0.0
    for e in sorted(events, key=lambda e: e["t"]):
        if e["type"] == "marker":
            out.append(dict(e, t=round(e["t"] + shift, 6)))
            shift += MARKER_SECONDS
        else:
            out.append(dict(e, t=round(e["t"] + shift, 6)))
    return out


def desktop(display_name):
    """Whether `display_name` is this session's own display (or :0), which replay never types into."""
    own = {os.environ.get("DISPLAY"), ":0", ":0.0"} - {None, ""}
    return display_name in own


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(dest="command", required=True)
    r = sub.add_parser("record")
    r.add_argument("out", type=Path)
    r.add_argument("--device", action="append", default=[], help="an evdev device to record")
    r.add_argument("--all-devices", action="store_true", help="record every keyboard and mouse this user can read")
    r.add_argument("--marker", default="KEY_F8")
    r.add_argument("--stop", default="KEY_F9")
    p = sub.add_parser("replay")
    p.add_argument("recording", type=Path)
    p.add_argument("--display", required=True, help="an X display, never the desktop's own")
    p.add_argument("--route", help="only this route")
    s = sub.add_parser("show")
    s.add_argument("recording", type=Path)
    args = parser.parse_args()
    if args.command == "record":
        record(args.out, args.device, args.marker, args.stop, args.all_devices)
    elif args.command == "replay":
        if desktop(args.display):
            parser.error(f"--display {args.display} is the desktop's: replay into a display of its own (Xvfb)")
        injector = Injector(args.display)
        for number, (name, events) in enumerate(routes(read(args.recording)), 1):
            if args.route in (None, name):
                print(f"{name}: {injector.play(events, number=number):.1f} s")
    else:
        for name, events in routes(read(args.recording)):
            kinds = {}
            for e in events:
                kinds[e["type"]] = kinds.get(e["type"], 0) + 1
            print(f"{name}: {events[-1]['t']:.1f} s, {kinds}")


if __name__ == "__main__":
    main()
