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

#[cfg(test)]
mod dispatch_deadline_tests {
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    use wayland_client::Connection;

    use crate::capture::types::{CaptureError, CaptureState};
    use crate::capture::{WlrScreencopyState, dispatch_until};

    /// A compositor that stops answering must not park the caller forever: the portal runs
    /// captures on `spawn_blocking`, so an unbounded wait leaks one thread per hung request.
    /// Nothing is ever sent on this fresh queue, so the deadline is the only way out.
    #[test]
    fn dispatch_until_gives_up_at_the_deadline() {
        let Ok(conn) = Connection::connect_to_env() else {
            eprintln!("skipping: no Wayland compositor in this environment");
            return;
        };

        let mut queue = conn.new_event_queue::<WlrScreencopyState>();
        let mut state = WlrScreencopyState {
            state: Arc::new(Mutex::new(CaptureState::Pending)),
        };

        let budget = Duration::from_millis(300);
        let started = Instant::now();
        let result = dispatch_until(&conn, &mut queue, &mut state, started + budget, |_| false);
        let elapsed = started.elapsed();

        assert!(
            matches!(result, Err(CaptureError::Timeout(_))),
            "expected a timeout, got {result:?}"
        );
        assert!(elapsed >= budget, "returned early after {elapsed:?}");
        assert!(elapsed < budget * 10, "overshot the deadline: {elapsed:?}");
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
        // wl_shm XRGB8888 is 1; DRM XRGB8888 is 0x34325258. Both name the same format, and the
        // two encodings cannot collide, so `from_raw` resolves either to Xrgb8888 rather than
        // dropping a misencoded DRM code into `Unknown` and failing the capture.
        assert_eq!(PixelFormat::from_raw(1), PixelFormat::Xrgb8888);
        assert_eq!(
            PixelFormat::from_drm_fourcc(0x34325258),
            PixelFormat::Xrgb8888
        );
        assert_eq!(PixelFormat::from_raw(0x34325258), PixelFormat::Xrgb8888);
        assert_eq!(PixelFormat::from_raw(0x34325241), PixelFormat::Argb8888);
    }

    #[test]
    fn genuinely_unknown_codes_are_still_preserved() {
        // Widening from_raw must not swallow codes the crate does not name.
        assert_eq!(
            PixelFormat::from_raw(0xDEADBEEF),
            PixelFormat::Unknown(0xDEADBEEF)
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
            data: rows.to_vec().into(),
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
        assert_eq!(px.to_rgba8().unwrap(), px.data.to_vec());
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

#[cfg(test)]
mod output_transform_tests {
    use crate::probe::transform_swaps_axes;
    use wayland_client::protocol::wl_output::Transform;

    /// The agl backend derives its buffer geometry from `wl_output.mode`, which is the physical
    /// pre-transform size. Getting this predicate wrong is what would make a portrait output
    /// capture at the wrong shape.
    #[test]
    fn quarter_turns_swap_axes() {
        for t in [
            Transform::_90,
            Transform::_270,
            Transform::Flipped90,
            Transform::Flipped270,
        ] {
            assert!(transform_swaps_axes(t), "{t:?} should swap axes");
        }
    }

    #[test]
    fn half_turns_and_flips_keep_axes() {
        // A flip mirrors within the same axes, so 180 and the un-rotated flip do not swap.
        for t in [
            Transform::Normal,
            Transform::_180,
            Transform::Flipped,
            Transform::Flipped180,
        ] {
            assert!(!transform_swaps_axes(t), "{t:?} should not swap axes");
        }
    }
}

#[cfg(test)]
mod live_connection_tests {
    use crate::probe;

    /// The connection is cached across portal requests, so its output list has to survive being
    /// re-read repeatedly. The regression this guards: output state was snapshotted on an event
    /// queue that died when `connect()` returned, so nothing could refresh it afterwards.
    #[test]
    fn outputs_can_be_refreshed_repeatedly_on_a_cached_connection() {
        let Ok(conn) = probe::connect() else {
            eprintln!("skipping: no Wayland compositor in this environment");
            return;
        };

        let first = conn.outputs();
        for round in 0..3 {
            conn.refresh_outputs()
                .unwrap_or_else(|e| panic!("refresh {round} failed: {e}"));
            let now = conn.outputs();
            // Nothing is hotplugging under a unit test, so membership should be stable.
            assert_eq!(
                now.len(),
                first.len(),
                "output count changed on refresh {round}"
            );
            for (a, b) in first.iter().zip(now.iter()) {
                assert_eq!(a.global_name, b.global_name, "output identity churned");
                assert_eq!((a.width, a.height), (b.width, b.height), "geometry churned");
            }
        }
    }
}
