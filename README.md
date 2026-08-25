# xdg-desktop-portal-agl

xdg-desktop-portal backend for AGL. Implements the Screenshot and Settings
portal interfaces on top of AGL's Wayland compositor.

## Implemented

Screenshot (v1): Screenshot and PickColor. Capture tries weston_capture_v1
first (the AGL/libweston protocol), falls back to agl_screenshooter, then
to wlr-screencopy for dev desktops that don't have either.

Settings (v2): read/read_all for org.freedesktop.appearance. No live
updates yet, SettingChanged is defined but nothing ever emits it.

## Building

Rust 1.87 or newer — edition 2024 needs 1.85, and `u32::is_multiple_of`
(weston.rs stride check) needs 1.87. It is declared as `rust-version` in
Cargo.toml, so an older toolchain fails with a clear message rather than a
confusing parse error. Worth checking against the `rust` recipe in whichever
Yocto release series you build against before picking this up.

```
cargo build --release
```

## Configuration

There are no D-Bus options for any of this; the Screenshot interface has no
output argument, and the capture path has a couple of choices worth being able
to override on a target. All are read from the daemon's environment — set them
in `xdg-desktop-portal-agl.service`.

| Variable | Default | Meaning |
| --- | --- | --- |
| `XDP_AGL_OUTPUT` | first output | Which screen to capture. An index (`0`, `1`) or a `wl_output` name (`HDMI-A-1`). Names need a compositor sending `wl_output.name` (v4+). **Effectively required on a multi-display unit** — an instrument cluster plus a centre stack will otherwise always get whichever output the compositor happens to advertise first. |
| `XDP_AGL_CAPTURE_SOURCE` | negotiated | Pins the `weston_capture_v1` pixel source: `writeback`, `framebuffer`, `full-framebuffer` or `blending`. Unset, `writeback` is tried first and `framebuffer` is the fallback. `framebuffer` temporarily disables hardware planes, so video offloaded to a KMS overlay captures black; pin `writeback` if the compositor offers it. Ignored by the other two backends. |
| `XDP_AGL_CAPTURE_TIMEOUT_MS` | `5000` | How long one capture may wait on the compositor before giving up. The budget covers a whole capture, not each phase. Raise it on a slow target where a repaint can take longer than 5s; lowering it bounds how long a hung compositor holds a request open. |
| `XDG_PICTURES_DIR` | from `user-dirs.dirs` | Where PNGs are written. Falls back to `$HOME/Pictures`, then `$XDG_STATE_HOME/xdg-desktop-portal-agl`. |

## TODO / known issues

- weston_capture_v1 needs --debug on the compositor to work. agl-compositor
  only registers a screenshot authority when --debug is set, and that
  authority allows every client, not just us. Can't turn --debug on in
  production, so production ends up on the agl_screenshooter fallback,
  which can't see hardware planes and shows video as black. libweston does
  support per-client authorization (weston's own screen-recorder uses it,
  see weston_compositor_add_screenshot_authority) — agl-compositor would
  need to register an authority that checks for the portal's client
  instead of gating on --debug. That's a compositor-side patch, not
  something we can do from this repo.

- Screenshot interface is v1, no Request object, so nothing can cancel a
  capture in progress. v2 would fix that, haven't started.

- agl_screenshooter reports no capture size, and the protocol doesn't say
  whether the buffer should be the output's physical mode size or its
  post-transform logical size. On a rotated output the backend tries the
  mode size and retries transposed if the compositor answers bad_buffer,
  so either convention works — but the pixels come back in whatever
  orientation the compositor wrote them, which on a 90/270 output may not
  match what's on the glass. weston_capture_v1 is unaffected: it reports
  its own size.

- settings never watches its source file, so ReadAll is a snapshot from
  startup.
