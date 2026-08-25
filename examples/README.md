# capture

Screenshots one output and writes a PNG. Used to check the `wayland_capture`
backends against a real compositor.

```
cargo run --example capture -- [output-name] [out.png]
```

- `output-name` — output to grab (e.g. `wayland0`). Omit for the first output.
- `out.png` — destination path. Defaults to `capture.png`.

It prints the globals it found and the backend it picked, then captures.

## Unit tests

No compositor needed:

```
cargo test
```

## Live capture against agl-compositor

`weston_capture_v1` is privileged, so the compositor must run with `--debug` or
the capture returns `unauthorized`.

Start it nested inside your existing Wayland session:

```
export AGL=/path/to/AGL
export LD_LIBRARY_PATH=$AGL/lib/aarch64-linux-gnu

WAYLAND_DISPLAY=wayland-0 setsid $AGL/bin/agl-compositor \
    -B wayland-backend.so -r pixman -S wayland-agl --no-config --debug \
    >/tmp/agl.log 2>&1 &
disown
```

Capture from it (point at the nested socket, not your desktop):

```
WAYLAND_DISPLAY=wayland-agl cargo run --example capture -- wayland0 out.png
file out.png    # PNG image data, 1024 x 768, 8-bit/color RGBA
```

Stop it when done:

```
kill $(pgrep -x agl-compositor)
rm -f $XDG_RUNTIME_DIR/wayland-agl $XDG_RUNTIME_DIR/wayland-agl.lock
```

## Screenshot portal against agl-compositor

Exercises the full D-Bus path: `org.freedesktop.impl.portal.Screenshot` ->
`wayland_capture` -> PNG. This is what xdg-desktop-portal calls at runtime.

```
export AGL=/path/to/AGL
export LD_LIBRARY_PATH=$AGL/lib/aarch64-linux-gnu
export XDG_RUNTIME_DIR=${XDG_RUNTIME_DIR:-/run/user/$(id -u)}

# 1. agl-compositor nested, with --debug (authorizes weston_capture_v1)
pkill -x agl-compositor; rm -f $XDG_RUNTIME_DIR/wayland-agl*
WAYLAND_DISPLAY=wayland-0 setsid $AGL/bin/agl-compositor \
    -B wayland-backend.so -r pixman -S wayland-agl --no-config --debug \
    >/tmp/aglc.log 2>&1 &
disown
sleep 4

# 2. Build and run the portal, pointed at agl-compositor for its captures
cargo build
WAYLAND_DISPLAY=wayland-agl setsid ./target/debug/xdg-desktop-portal-agl \
    >/tmp/portal.log 2>&1 &
disown
sleep 2

# 3. Inspect the interface
busctl --user introspect org.freedesktop.impl.portal.desktop.agl \
    /org/freedesktop/portal/desktop org.freedesktop.impl.portal.Screenshot

# 4. Call Screenshot(handle, app_id, parent_window, options)
gdbus call --session --dest org.freedesktop.impl.portal.desktop.agl \
    --object-path /org/freedesktop/portal/desktop \
    --method org.freedesktop.impl.portal.Screenshot.Screenshot \
    /req/1 "test.app" "" "{}"
# -> (uint32 0, {'uri': <'file:///.../Pictures/Screenshot-<ts>.png'>})

# 5. PickColor returns the centre pixel as (ddd) in [0,1]
gdbus call --session --dest org.freedesktop.impl.portal.desktop.agl \
    --object-path /org/freedesktop/portal/desktop \
    --method org.freedesktop.impl.portal.Screenshot.PickColor \
    /req/2 "test.app" "" "{}"
```

Stop it when done:

```
kill $(pgrep -x xdg-desktop-portal-agl) $(pgrep -x agl-compositor)
rm -f $XDG_RUNTIME_DIR/wayland-agl*
```

With no clients running, the shot is agl-compositor's black curtain (a correct
capture). On a booted AGL image the homescreen provides the visible UI. The
capture requires agl-compositor to run with `--debug`, or to be patched to add a
screenshot authority for the portal.

## Full-color shot via weston

agl-compositor draws a black curtain until a shell client calls
`agl_shell.ready()`, and it is unstable when run nested. For a quick non-black
capture through the *same* `weston_capture_v1` backend, use weston from the AGL
prefix — its desktop-shell shows a panel and wallpaper out of the box.

One command:

```
AGL=/path/to/AGL examples/shot-weston.sh shot.png
```

The script sets `LD_LIBRARY_PATH`, kills any stale compositor, launches AGL
weston nested with `--debug`, opens a couple of clients, and captures.

### Doing it by hand

Two things bite if done manually: `$AGL` must be exported (otherwise `$AGL/bin`
becomes `/bin` and you get the system weston, which has no `weston_capture_v1`),
and a stale compositor holding the socket must be cleared first.

```
export AGL=/path/to/AGL
export LD_LIBRARY_PATH=$AGL/lib/aarch64-linux-gnu
$AGL/bin/weston --version            # must say 13.x, not 9.x

pkill -x weston; rm -f $XDG_RUNTIME_DIR/wayland-wes*   # clear any stale one

WAYLAND_DISPLAY=wayland-0 setsid $AGL/bin/weston \
    -B wayland-backend.so -S wayland-wes --width=1024 --height=768 --debug &
disown
sleep 4

WAYLAND_DISPLAY=wayland-wes $AGL/bin/weston-terminal &
WAYLAND_DISPLAY=wayland-wes cargo run --example capture -- wayland0 shot.png

pkill -x weston                      # stop it when done
```

## Notes

- With no clients running the compositor shows its black curtain, so the shot is
  solid black — that is the real framebuffer, not an empty buffer. Launch a
  client first for something visible:
  `WAYLAND_DISPLAY=wayland-agl $AGL/bin/weston-terminal &`
- Run against your normal desktop and you'll see `selected backend: None` if the
  compositor exposes none of the supported protocols (weston-output-capture,
  agl-screenshooter, wlr-screencopy). GNOME/mutter is one such case.
- The `output-name` selector needs a compositor that sends `wl_output.name`
  (version 4+). On older outputs, omit the name to grab the first output.
- The daemon takes its output, pixel-source and timeout settings from the
  environment (`XDP_AGL_OUTPUT`, `XDP_AGL_CAPTURE_SOURCE`,
  `XDP_AGL_CAPTURE_TIMEOUT_MS`) — see the Configuration table in the top-level
  README. `XDP_AGL_CAPTURE_SOURCE` and `XDP_AGL_CAPTURE_TIMEOUT_MS` apply to
  this example too, since it goes through the same capture library.
