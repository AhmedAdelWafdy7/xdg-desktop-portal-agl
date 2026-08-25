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

pub mod capture;
pub mod probe;
pub mod protocols;
pub mod registry;

#[cfg(test)]
mod tests;

pub use capture::types::{CaptureError, PixelBuffer, PixelData, PixelFormat};
pub use probe::{OutputSelector, WaylandConnection};

use capture::WlrScreencopy;
use capture::WlrScreencopyState;
use capture::agl::AglScreenshot;
use capture::weston::WestonCapture;
use registry::SelectedCapturebackend;

/// Connect to the compositor, select a capture backend, and screenshot the chosen output.
///
/// Backend precedence is weston-output-capture (primary on AGL), then agl-screenshooter
/// (legacy fallback), then wlr-screencopy. wlr-screencopy exists for wlroots-based dev
/// desktops only; no libweston/AGL compositor advertises `zwlr_screencopy_manager_v1`,
/// so in practice it and the two AGL-native backends above are mutually exclusive.
/// Returns the raw captured [`PixelBuffer`].
pub fn capture_output(selector: &OutputSelector) -> Result<PixelBuffer, CaptureError> {
    let connection = probe::connect()?;
    capture_on(&connection, selector)
}

/// Capture the chosen output and encode the result as a PNG image.
pub fn capture_output_png(selector: &OutputSelector) -> Result<Vec<u8>, CaptureError> {
    capture_output(selector)?.encode_png()
}

/// Perform the capture using an already-established [`WaylandConnection`].
pub fn capture_on(
    connection: &WaylandConnection,
    selector: &OutputSelector,
) -> Result<PixelBuffer, CaptureError> {
    let output = connection
        .select_output(selector)
        .ok_or_else(|| CaptureError::CaptureFailed(format!("no such output: {selector:?}")))?;

    let missing =
        |what: &str| CaptureError::CaptureFailed(format!("{what} advertised but not bound"));

    match connection.capabilities.selected_capture_backend() {
        Some(SelectedCapturebackend::WestonScreenshooter) => {
            let factory = connection
                .weston_capture
                .as_ref()
                .ok_or_else(|| missing("weston_capture_v1"))?;
            let backend = WestonCapture::new(factory.clone(), connection.shm.clone());
            backend.capture(&connection.conn, &output.wl_output)
        }
        Some(SelectedCapturebackend::AglScreenshooter) => {
            let info = connection
                .capabilities
                .agl_screenshooter
                .ok_or_else(|| missing("agl_screenshooter"))?;
            let backend =
                AglScreenshot::new(info.name, info.version.min(1), connection.shm.clone());
            backend.capture(
                &connection.conn,
                &connection.registry,
                &output.wl_output,
                output.width,
                output.height,
            )
        }
        Some(SelectedCapturebackend::WlrScreencopy) => {
            let manager = connection
                .wlr_screencopy
                .as_ref()
                .ok_or_else(|| missing("zwlr_screencopy_manager_v1"))?;
            let mut event_queue = connection.conn.new_event_queue::<WlrScreencopyState>();
            let mut backend = WlrScreencopy::new(manager.clone(), connection.shm.clone());
            backend.capture(&connection.conn, &output.wl_output, &mut event_queue)
        }
        None => Err(CaptureError::CaptureFailed(
            "compositor exposes no supported screenshot protocol".into(),
        )),
    }
}
