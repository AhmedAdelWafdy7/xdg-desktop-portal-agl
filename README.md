# xdg-desktop-portal-agl

xdg-desktop-portal backend for AGL. Implements the Screenshot and Settings
portal interfaces on top of AGL's Wayland compositor.

## Implemented

Screenshot (v1): Screenshot and PickColor. Capture tries weston_capture_v1
first (the AGL/libweston protocol), falls back to agl_screenshooter, then
to wlr-screencopy for dev desktops that don't have either.

Settings (v2): read/read_all for org.freedesktop.appearance. No live
updates yet, SettingChanged is defined but nothing ever emits it.

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

- output list is read once at connect and never updated, so a hotplugged
  output won't be found by name until something reconnects.

- settings never watches its source file, so ReadAll is a snapshot from
  startup.
