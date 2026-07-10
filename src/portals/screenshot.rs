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

//! Backend implementation of `org.freedesktop.impl.portal.Screenshot`. xdg-desktop-portal
//! routes Screenshot / PickColor requests here; we capture the output via the wayland_capture
//! library (weston-output-capture on AGL), write a PNG, and return its URI.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use zbus::{
    interface,
    zvariant::{OwnedObjectPath, OwnedValue, Value},
};

use wayland_capture::{OutputSelector, PixelBuffer, capture_output};

// impl.portal response codes: 0 = success, 1 = user cancelled, 2 = ended with error.
const RESPONSE_SUCCESS: u32 = 0;
const RESPONSE_ERROR: u32 = 2;

pub struct ScreenshotPortal;

impl ScreenshotPortal {
    pub fn new() -> Self {
        Self
    }
}

impl Default for ScreenshotPortal {
    fn default() -> Self {
        Self::new()
    }
}

#[interface(name = "org.freedesktop.impl.portal.Screenshot")]
impl ScreenshotPortal {
    #[zbus(property, name = "version")]
    async fn version(&self) -> u32 {
        1
    }

    /// Capture the primary output and return a `file://` URI to a PNG. The `interactive` option
    /// (region/window selection UI) is not offered on AGL; the whole screen is always captured.
    async fn screenshot(
        &self,
        _handle: OwnedObjectPath,
        app_id: String,
        _parent_window: String,
        _options: HashMap<String, OwnedValue>,
    ) -> zbus::fdo::Result<(u32, HashMap<String, OwnedValue>)> {
        tracing::info!("Screenshot request from app_id={:?}", app_id);

        let capture = tokio::task::spawn_blocking(|| {
            let buffer = capture_output(&OutputSelector::First)?;
            buffer.encode_png()
        })
        .await;

        let png = match capture {
            Ok(Ok(png)) => png,
            Ok(Err(e)) => {
                tracing::error!("capture failed: {e}");
                return Ok((RESPONSE_ERROR, HashMap::new()));
            }
            Err(e) => {
                tracing::error!("capture task panicked: {e}");
                return Ok((RESPONSE_ERROR, HashMap::new()));
            }
        };

        match save_png(&png) {
            Ok(uri) => {
                tracing::info!("screenshot saved: {uri}");
                let mut results = HashMap::new();
                results.insert(
                    "uri".to_string(),
                    Value::from(uri)
                        .try_into()
                        .expect("string is a valid value"),
                );
                Ok((RESPONSE_SUCCESS, results))
            }
            Err(e) => {
                tracing::error!("failed to write screenshot: {e}");
                Ok((RESPONSE_ERROR, HashMap::new()))
            }
        }
    }

    /// Return the colour of a pixel. AGL has no interactive colour picker, so the centre pixel
    /// of the primary output is sampled and returned as sRGB components in the range [0, 1].
    async fn pick_color(
        &self,
        _handle: OwnedObjectPath,
        app_id: String,
        _parent_window: String,
        _options: HashMap<String, OwnedValue>,
    ) -> zbus::fdo::Result<(u32, HashMap<String, OwnedValue>)> {
        tracing::info!("PickColor request from app_id={:?}", app_id);

        let sampled = tokio::task::spawn_blocking(|| {
            let buffer = capture_output(&OutputSelector::First)?;
            center_color(&buffer)
        })
        .await;

        match sampled {
            Ok(Ok((r, g, b))) => {
                let mut results = HashMap::new();
                results.insert(
                    "color".to_string(),
                    Value::from((r, g, b))
                        .try_into()
                        .expect("(ddd) is a valid value"),
                );
                Ok((RESPONSE_SUCCESS, results))
            }
            Ok(Err(e)) => {
                tracing::error!("pick color capture failed: {e}");
                Ok((RESPONSE_ERROR, HashMap::new()))
            }
            Err(e) => {
                tracing::error!("pick color task panicked: {e}");
                Ok((RESPONSE_ERROR, HashMap::new()))
            }
        }
    }
}

/// Sample the centre pixel of the buffer as sRGB components in [0, 1].
fn center_color(buffer: &PixelBuffer) -> Result<(f64, f64, f64), wayland_capture::CaptureError> {
    let rgba = buffer.to_rgba8()?;
    let x = (buffer.width / 2) as usize;
    let y = (buffer.height / 2) as usize;
    let i = (y * buffer.width as usize + x) * 4;
    let r = rgba[i] as f64 / 255.0;
    let g = rgba[i + 1] as f64 / 255.0;
    let b = rgba[i + 2] as f64 / 255.0;
    Ok((r, g, b))
}

/// Write the PNG to the user's pictures directory (or a temp dir) and return a `file://` URI.
fn save_png(png: &[u8]) -> std::io::Result<String> {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let dir = screenshot_dir();
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("Screenshot-{ts}.png"));
    std::fs::write(&path, png)?;

    Ok(format!("file://{}", path.display()))
}

/// Pick a destination directory: $XDG_PICTURES_DIR, else ~/Pictures, else the system temp dir.
fn screenshot_dir() -> PathBuf {
    if let Some(pics) = std::env::var_os("XDG_PICTURES_DIR") {
        let p = PathBuf::from(pics);
        if p.is_dir() {
            return p;
        }
    }
    if let Some(home) = std::env::var_os("HOME") {
        let pics = Path::new(&home).join("Pictures");
        if pics.is_dir() {
            return pics;
        }
    }
    std::env::temp_dir()
}
