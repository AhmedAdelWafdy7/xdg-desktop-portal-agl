// Copyright 2026 Ahmed Wafdy <ahmedadelwafdy782@gmail.com>
//
// This file is part of xdg-desktop-portal-agl.
//
// xdg-desktop-portal-agl is free software: you can redistribute it and/or
// modify it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 2 of the License, or
// (at your option) any later version.
//
// xdg-desktop-portal-agl is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the GNU General
// Public License for more details.
//
// You should have received a copy of the GNU General Public License along with
// xdg-desktop-portal-agl. If not, see <https://www.gnu.org/licenses/>.

use wayland_client::{
    Connection, Dispatch, QueueHandle,
    globals::registry_queue_init,
    protocol::{wl_compositor::WlCompositor, wl_registry},
};
use wayland_protocols::xdg::shell::client::xdg_wm_base::XdgWmBase;

use crate::shell::xdg::{XdgShellSurface, XdgState};
use crate::shell::{ShellBackend, ShellSurface, SurfaceRole};

impl Dispatch<WlCompositor, ()> for XdgState {
    fn event(
        _state: &mut Self,
        _proxy: &WlCompositor,
        _event: wayland_client::protocol::wl_compositor::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_registry::WlRegistry, wayland_client::globals::GlobalListContents> for XdgState {
    fn event(
        _state: &mut Self,
        _proxy: &wl_registry::WlRegistry,
        _event: wl_registry::Event,
        _data: &wayland_client::globals::GlobalListContents,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

use std::os::unix::io::{AsRawFd, BorrowedFd, FromRawFd, OwnedFd};
use std::time::Duration;

use wayland_client::protocol::{
    wl_buffer::WlBuffer,
    wl_shm::{Format, WlShm},
    wl_shm_pool::WlShmPool,
};

impl Dispatch<WlShm, ()> for XdgState {
    fn event(
        _state: &mut Self,
        _proxy: &WlShm,
        _event: wayland_client::protocol::wl_shm::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

#[cfg(test)]
impl Dispatch<WlShmPool, ()> for XdgState {
    fn event(
        _state: &mut Self,
        _proxy: &WlShmPool,
        _event: wayland_client::protocol::wl_shm_pool::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

#[cfg(test)]
impl Dispatch<WlBuffer, ()> for XdgState {
    fn event(
        _state: &mut Self,
        _proxy: &WlBuffer,
        _event: wayland_client::protocol::wl_buffer::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}
#[cfg(test)]
mod pixel_format_tests {
    use crate::capture::types::PixelFormat;

    #[test]
    fn known_formats_parse_correctly() {
        assert_eq!(PixelFormat::from_raw(0x00000000), PixelFormat::Argb8888);
        assert_eq!(PixelFormat::from_raw(0x00000001), PixelFormat::Xrgb8888);
        assert_eq!(PixelFormat::from_raw(0x34324241), PixelFormat::Abgr8888);
        assert_eq!(PixelFormat::from_raw(0x34324258), PixelFormat::Xbgr8888);
    }

    #[test]
    fn unknown_format_preserved() {
        let raw = 0xDEADBEEFu32;
        assert_eq!(PixelFormat::from_raw(raw), PixelFormat::Unknown(raw));
    }

    #[test]
    fn known_formats_report_four_bytes_per_pixel() {
        let formats: [PixelFormat; 4] = [
            PixelFormat::Argb8888,
            PixelFormat::Xrgb8888,
            PixelFormat::Abgr8888,
            PixelFormat::Xbgr8888,
        ];
        for fmt in formats {
            assert_eq!(fmt.bytes_per_pixel(), Some(4), "{fmt:?} should be 4 bpp");
        }
    }

    #[test]
    fn unknown_format_has_no_bpp() {
        assert_eq!(PixelFormat::Unknown(0xFF).bytes_per_pixel(), None);
    }
}

#[test]
#[ignore = "requires a live Wayland compositor with xdg-shell"]
fn xdg_shell_smoke_test() {
    let conn = Connection::connect_to_env().expect("connect to Wayland compositor");
    let (globals, mut event_queue) =
        registry_queue_init::<XdgState>(&conn).expect("initialize Wayland registry");

    let qh = event_queue.handle();

    let compositor = globals
        .bind::<WlCompositor, _, _>(&qh, 1..=6, ())
        .expect("bind wl_compositor");

    let xdg_wm_base = globals
        .bind::<XdgWmBase, _, _>(&qh, 1..=6, ())
        .expect("bind xdg_wm_base");

    let mut state = XdgState;

    let mut surface: XdgShellSurface =
        XdgShellSurface::new::<XdgState>(&compositor, &xdg_wm_base, &qh);
    surface
        .assign_role(SurfaceRole::Toplevel, &qh)
        .expect("assign xdg toplevel role");

    assert_eq!(surface.backend(), ShellBackend::Xdg);

    surface.commit();

    event_queue
        .roundtrip(&mut state)
        .expect("dispatch xdg configure/ping events");
}

#[test]
#[ignore = "requires a live Wayland compositor"]
fn xdg_shell_visible_window_test() {
    let conn = Connection::connect_to_env().expect("connect to Wayland");
    let (globals, mut event_queue) = registry_queue_init::<XdgState>(&conn).expect("init registry");
    let qh = event_queue.handle();

    let compositor = globals
        .bind::<WlCompositor, _, _>(&qh, 1..=6, ())
        .expect("bind wl_compositor");

    let shm = globals
        .bind::<WlShm, _, _>(&qh, 1..=1, ())
        .expect("bind wl_shm");

    let xdg_wm_base = globals
        .bind::<XdgWmBase, _, _>(&qh, 1..=6, ())
        .expect("bind xdg_wm_base");

    let mut state = XdgState;

    let mut surface = XdgShellSurface::new::<XdgState>(&compositor, &xdg_wm_base, &qh);
    surface
        .assign_role(SurfaceRole::Toplevel, &qh)
        .expect("assign toplevel");

    // First commit with no buffer.
    // This lets the compositor configure the xdg_surface.
    surface.commit();

    event_queue
        .roundtrip(&mut state)
        .expect("wait for initial xdg configure");

    let width = 320;
    let height = 180;
    let stride = width * 4;
    let size = stride * height;

    let fd = unsafe { libc::memfd_create(c"xdg-visible-test".as_ptr(), libc::MFD_CLOEXEC) };
    assert!(fd >= 0);

    unsafe {
        assert_eq!(libc::ftruncate(fd, size as libc::off_t), 0);
    }

    let ptr = unsafe {
        libc::mmap(
            std::ptr::null_mut(),
            size as usize,
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_SHARED,
            fd,
            0,
        )
    };
    assert_ne!(ptr, libc::MAP_FAILED);

    let pixels = unsafe { std::slice::from_raw_parts_mut(ptr as *mut u8, size as usize) };

    for px in pixels.chunks_exact_mut(4) {
        px[0] = 0x20; // B
        px[1] = 0x20; // G
        px[2] = 0xff; // R
        px[3] = 0xff; // X/A
    }

    let owned_fd = unsafe { OwnedFd::from_raw_fd(fd) };
    let borrowed_fd = unsafe { BorrowedFd::borrow_raw(owned_fd.as_raw_fd()) };

    let pool = shm.create_pool(borrowed_fd, size, &qh, ());
    let buffer = pool.create_buffer(0, width, height, stride, Format::Xrgb8888, &qh, ());

    surface.wl_surface().attach(Some(&buffer), 0, 0);
    surface.wl_surface().damage_buffer(0, 0, width, height);
    surface.commit();

    event_queue
        .roundtrip(&mut state)
        .expect("commit visible buffer");

    std::thread::sleep(Duration::from_secs(5));

    unsafe {
        libc::munmap(ptr, size as usize);
    }
}

#[cfg(test)]
mod format_conversion_tests {
    use crate::capture::types::PixelFormat;
    use wayland_client::protocol::wl_shm::Format;

    #[test]
    fn drm_fourcc_maps_to_named_formats() {
        // DRM FourCC codes differ from the wl_shm enum for ARGB/XRGB.
        assert_eq!(
            PixelFormat::from_drm_fourcc(0x34325241),
            PixelFormat::Argb8888
        );
        assert_eq!(
            PixelFormat::from_drm_fourcc(0x34325258),
            PixelFormat::Xrgb8888
        );
        assert_eq!(
            PixelFormat::from_drm_fourcc(0x34324241),
            PixelFormat::Abgr8888
        );
        assert_eq!(
            PixelFormat::from_drm_fourcc(0x34324258),
            PixelFormat::Xbgr8888
        );
        assert_eq!(
            PixelFormat::from_drm_fourcc(0x36314752),
            PixelFormat::Rgb565
        );
    }

    #[test]
    fn drm_xrgb_is_not_confused_with_wl_shm_xrgb() {
        // wl_shm XRGB8888 is 1; DRM XRGB8888 is 0x34325258. They must not collide.
        assert_eq!(PixelFormat::from_raw(1), PixelFormat::Xrgb8888);
        assert_eq!(
            PixelFormat::from_drm_fourcc(0x34325258),
            PixelFormat::Xrgb8888
        );
        assert_eq!(
            PixelFormat::from_raw(0x34325258),
            PixelFormat::Unknown(0x34325258)
        );
    }

    #[test]
    fn wl_shm_format_conversion_roundtrips() {
        assert_eq!(
            PixelFormat::Argb8888.to_wl_shm_format(),
            Some(Format::Argb8888)
        );
        assert_eq!(
            PixelFormat::Xrgb8888.to_wl_shm_format(),
            Some(Format::Xrgb8888)
        );
        assert_eq!(
            PixelFormat::Abgr8888.to_wl_shm_format(),
            Some(Format::Abgr8888)
        );
        assert_eq!(
            PixelFormat::Xbgr8888.to_wl_shm_format(),
            Some(Format::Xbgr8888)
        );
        assert_eq!(PixelFormat::Unknown(0xFF).to_wl_shm_format(), None);
    }
}

#[cfg(test)]
mod rgba_and_png_tests {
    use crate::capture::types::{PixelBuffer, PixelFormat};

    // A 2x1 image where each pixel is laid out in the source format's memory order.
    fn buffer(format: PixelFormat, rows: &[u8]) -> PixelBuffer {
        PixelBuffer {
            data: rows.to_vec(),
            width: 2,
            height: 1,
            stride: 8,
            format,
        }
    }

    #[test]
    fn argb_swizzles_bgra_memory_to_rgba() {
        // Memory order for ARGB8888 is [B, G, R, A].
        let px = buffer(
            PixelFormat::Argb8888,
            &[0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88],
        );
        // Expect RGBA: R=0x33 G=0x22 B=0x11 A=0x44, then R=0x77 G=0x66 B=0x55 A=0x88.
        assert_eq!(
            px.to_rgba8().unwrap(),
            vec![0x33, 0x22, 0x11, 0x44, 0x77, 0x66, 0x55, 0x88]
        );
    }

    #[test]
    fn xrgb_forces_opaque_alpha() {
        let px = buffer(
            PixelFormat::Xrgb8888,
            &[0x11, 0x22, 0x33, 0x00, 0x55, 0x66, 0x77, 0x00],
        );
        assert_eq!(
            px.to_rgba8().unwrap(),
            vec![0x33, 0x22, 0x11, 0xFF, 0x77, 0x66, 0x55, 0xFF]
        );
    }

    #[test]
    fn abgr_is_already_rgba() {
        // Memory order for ABGR8888 is [R, G, B, A].
        let px = buffer(
            PixelFormat::Abgr8888,
            &[0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88],
        );
        assert_eq!(px.to_rgba8().unwrap(), px.data);
    }

    #[test]
    fn unknown_format_is_rejected() {
        let px = buffer(PixelFormat::Unknown(0xABCD), &[0u8; 8]);
        assert!(px.to_rgba8().is_err());
    }

    #[test]
    fn png_encodes_and_decodes_back_to_pixels() {
        let px = buffer(
            PixelFormat::Xrgb8888,
            &[0x11, 0x22, 0x33, 0x00, 0x55, 0x66, 0x77, 0x00],
        );
        let png_bytes = px.encode_png().expect("encode png");

        // A real PNG starts with the 8-byte signature.
        assert_eq!(
            &png_bytes[..8],
            &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]
        );

        let decoder = png::Decoder::new(std::io::Cursor::new(&png_bytes));
        let mut reader = decoder.read_info().expect("read png info");
        let info = reader.info();
        assert_eq!(info.width, 2);
        assert_eq!(info.height, 1);
        assert_eq!(info.color_type, png::ColorType::Rgba);

        let mut out = vec![0u8; reader.output_buffer_size()];
        let frame = reader.next_frame(&mut out).expect("decode png frame");
        out.truncate(frame.buffer_size());
        assert_eq!(out, px.to_rgba8().unwrap());
    }
}

#[cfg(test)]
mod backend_selection_tests {
    use crate::registry::{Capabilities, GlobalInfo, SelectedCapturebackend};

    fn info() -> GlobalInfo {
        GlobalInfo {
            name: 1,
            version: 1,
        }
    }

    #[test]
    fn no_capture_globals_selects_nothing() {
        assert_eq!(Capabilities::new().selected_capture_backend(), None);
    }

    #[test]
    fn weston_is_preferred_over_agl_and_wlr() {
        let mut caps = Capabilities::new();
        caps.weston_screenshooter = Some(info());
        caps.agl_screenshooter = Some(info());
        caps.zwlr_screencopy_manager = Some(info());
        assert_eq!(
            caps.selected_capture_backend(),
            Some(SelectedCapturebackend::WestonScreenshooter)
        );
    }

    #[test]
    fn agl_is_preferred_over_wlr_when_weston_absent() {
        let mut caps = Capabilities::new();
        caps.agl_screenshooter = Some(info());
        caps.zwlr_screencopy_manager = Some(info());
        assert_eq!(
            caps.selected_capture_backend(),
            Some(SelectedCapturebackend::AglScreenshooter)
        );
    }

    #[test]
    fn wlr_is_last_resort() {
        let mut caps = Capabilities::new();
        caps.zwlr_screencopy_manager = Some(info());
        assert_eq!(
            caps.selected_capture_backend(),
            Some(SelectedCapturebackend::WlrScreencopy)
        );
    }
}
