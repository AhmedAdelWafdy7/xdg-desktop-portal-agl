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
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use zbus::{
    interface,
    zvariant::{OwnedObjectPath, OwnedValue, Value},
};

use wayland_capture::{CaptureError, OutputSelector, PixelBuffer, WaylandConnection, capture_on};

// impl.portal response codes: 0 = success, 1 = user cancelled, 2 = ended with error.
const RESPONSE_SUCCESS: u32 = 0;
const RESPONSE_ERROR: u32 = 2;

const MAX_NAME_ATTEMPTS: usize = 64;

/// Holds the Wayland connection open across requests instead of reconnecting (fresh socket +
/// registry roundtrip) on every capture. A cold connect costs tens of milliseconds — measured
/// against xdg-desktop-portal-wlr, which stays connected, that gap alone roughly doubled our
/// per-request latency.
pub struct ScreenshotPortal {
    connection: Arc<Mutex<Option<WaylandConnection>>>,
}

impl ScreenshotPortal {
    pub fn new() -> Self {
        Self {
            connection: Arc::new(Mutex::new(None)),
        }
    }
}

/// Capture through the cached connection, connecting lazily on first use. On a Wayland-level
/// error the cached connection is dropped so the *next* request reconnects — covers the
/// compositor restarting or the socket otherwise dying — rather than every request repeating
/// the same failure forever.
///
/// The lock is held only long enough to connect-if-needed and clone the handle out — every
/// field of `WaylandConnection` is `Arc`-backed, so cloning is cheap — not across the capture
/// itself, which can take seconds waiting on the compositor. `wayland-client`'s
/// `prepare_read`/`ReadEventsGuard` mechanism is built for exactly this: independent
/// `EventQueue`s on separate threads coordinating fair access to one shared `Connection`, which
/// is what `dispatch_until` already does. Without this, two concurrent Screenshot/PickColor
/// requests would fully serialize on this mutex for the slower request's entire timeout instead
/// of running — and potentially failing — independently.
fn capture_buffer(
    cache: &Mutex<Option<WaylandConnection>>,
    selector: &OutputSelector,
) -> Result<PixelBuffer, CaptureError> {
    let connection = {
        let mut guard = cache.lock().unwrap();
        if guard.is_none() {
            *guard = Some(wayland_capture::probe::connect()?);
        }
        guard.as_ref().unwrap().clone()
    };

    match capture_on(&connection, selector) {
        Ok(buffer) => Ok(buffer),
        Err(e @ CaptureError::WaylandError(_)) => {
            *cache.lock().unwrap() = None;
            Err(e)
        }
        Err(e) => Err(e),
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

        let connection = self.connection.clone();
        let capture = tokio::task::spawn_blocking(move || {
            let buffer = capture_buffer(&connection, &output_selector())?;
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
        // TODO: export an org.freedesktop.impl.portal.Request at this path so callers can
        // Close() an in-flight capture. Accepted at interface version 1, where the spec does not
        // require it; a Close() today gets an unknown-object error.
        _handle: OwnedObjectPath,
        app_id: String,
        _parent_window: String,
        _options: HashMap<String, OwnedValue>,
    ) -> zbus::fdo::Result<(u32, HashMap<String, OwnedValue>)> {
        tracing::info!("PickColor request from app_id={:?}", app_id);

        let connection = self.connection.clone();
        let sampled = tokio::task::spawn_blocking(move || {
            let buffer = capture_buffer(&connection, &output_selector())?;
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

/// Which output to capture.
///
/// The Screenshot interface has no output option, but a multi-display AGL system (instrument
/// cluster plus centre stack) still needs a way to say which screen it means, so
/// `XDP_AGL_OUTPUT` takes an index or an output name. Default is the first output.
fn output_selector() -> OutputSelector {
    match std::env::var("XDP_AGL_OUTPUT") {
        Ok(value) if !value.is_empty() => match value.parse::<usize>() {
            Ok(index) => OutputSelector::Index(index),
            Err(_) => OutputSelector::Name(value),
        },
        _ => OutputSelector::First,
    }
}

/// Sample the centre pixel of the buffer as sRGB components in [0, 1].
fn center_color(buffer: &PixelBuffer) -> Result<(f64, f64, f64), wayland_capture::CaptureError> {
    let [r, g, b, _a] = buffer.pixel_rgba8(buffer.width / 2, buffer.height / 2)?;
    Ok((r as f64 / 255.0, g as f64 / 255.0, b as f64 / 255.0))
}

/// Write the PNG somewhere durable and return a `file://` URI.
fn save_png(png: &[u8]) -> std::io::Result<String> {
    save_png_in(&screenshot_dir(), png)
}

/// Write `png` into `dir` under a name no concurrent request can steal.
fn save_png_in(dir: &Path, png: &[u8]) -> std::io::Result<String> {
    std::fs::create_dir_all(dir)?;

    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);

    // Millisecond stamps narrow the collision window but don't close it: two requests can still
    // land in the same millisecond. O_EXCL is what actually makes the name ours.
    for attempt in 0..MAX_NAME_ATTEMPTS {
        let name = match attempt {
            0 => format!("Screenshot-{ts}.png"),
            n => format!("Screenshot-{ts}-{n}.png"),
        };
        let path = dir.join(name);

        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(mut file) => {
                use std::io::Write;
                file.write_all(png)?;
                return Ok(format!("file://{}", percent_encode_path(&path)));
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }

    Err(std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        "could not find a free screenshot filename",
    ))
}

fn percent_encode_path(path: &Path) -> String {
    use std::os::unix::ffi::OsStrExt;

    let mut out = String::new();
    for &b in path.as_os_str().as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' => {
                out.push(b as char);
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Pick a destination directory for screenshots.
fn screenshot_dir() -> PathBuf {
    if let Some(pics) = std::env::var_os("XDG_PICTURES_DIR") {
        let p = PathBuf::from(pics);
        if p.is_dir() {
            return p;
        }
    }

    if let Some(pics) = pictures_dir_from_user_dirs().filter(|p| p.is_dir()) {
        return pics;
    }

    if let Some(home) = std::env::var_os("HOME") {
        let pics = Path::new(&home).join("Pictures");
        if pics.is_dir() {
            return pics;
        }
    }

    state_dir()
}

/// Directory used when the user has no pictures directory at all.
fn state_dir() -> PathBuf {
    if let Some(state) = std::env::var_os("XDG_STATE_HOME") {
        return PathBuf::from(state).join("xdg-desktop-portal-agl");
    }
    if let Some(home) = std::env::var_os("HOME") {
        return Path::new(&home)
            .join(".local/state")
            .join("xdg-desktop-portal-agl");
    }
    std::env::temp_dir().join("xdg-desktop-portal-agl")
}

/// Read `XDG_PICTURES_DIR` out of `$XDG_CONFIG_HOME/user-dirs.dirs`.
fn pictures_dir_from_user_dirs() -> Option<PathBuf> {
    let config_home = match std::env::var_os("XDG_CONFIG_HOME") {
        Some(dir) => PathBuf::from(dir),
        None => Path::new(&std::env::var_os("HOME")?).join(".config"),
    };
    let contents = std::fs::read_to_string(config_home.join("user-dirs.dirs")).ok()?;
    parse_pictures_dir(&contents, &std::env::var("HOME").ok()?)
}

/// Parse the `XDG_PICTURES_DIR="$HOME/Pictures"` line out of a user-dirs.dirs body.
fn parse_pictures_dir(contents: &str, home: &str) -> Option<PathBuf> {
    for line in contents.lines() {
        let line = line.trim();
        if line.starts_with('#') {
            continue;
        }
        let Some(value) = line.strip_prefix("XDG_PICTURES_DIR=") else {
            continue;
        };
        // Values are shell-quoted, and relative ones are written as "$HOME/...".
        let value = value.trim().trim_matches('"');
        let expanded = match value.strip_prefix("$HOME") {
            Some(rest) => format!("{home}{rest}"),
            None => value.to_string(),
        };
        if expanded.is_empty() {
            return None;
        }
        return Some(PathBuf::from(expanded));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_dirs_home_relative_value_is_expanded() {
        let body = "XDG_DESKTOP_DIR=\"$HOME/Desktop\"\nXDG_PICTURES_DIR=\"$HOME/Pictures\"\n";
        assert_eq!(
            parse_pictures_dir(body, "/home/agl"),
            Some(PathBuf::from("/home/agl/Pictures"))
        );
    }

    #[test]
    fn user_dirs_absolute_value_is_kept() {
        let body = "XDG_PICTURES_DIR=\"/mnt/media/shots\"\n";
        assert_eq!(
            parse_pictures_dir(body, "/home/agl"),
            Some(PathBuf::from("/mnt/media/shots"))
        );
    }

    #[test]
    fn user_dirs_comments_are_ignored() {
        let body = "# XDG_PICTURES_DIR=\"$HOME/Wrong\"\nXDG_PICTURES_DIR=\"$HOME/Right\"\n";
        assert_eq!(
            parse_pictures_dir(body, "/home/agl"),
            Some(PathBuf::from("/home/agl/Right"))
        );
    }

    #[test]
    fn user_dirs_without_pictures_key_yields_nothing() {
        let body = "XDG_DESKTOP_DIR=\"$HOME/Desktop\"\nXDG_MUSIC_DIR=\"$HOME/Music\"\n";
        assert_eq!(parse_pictures_dir(body, "/home/agl"), None);
    }

    #[test]
    fn user_dirs_empty_value_yields_nothing() {
        assert_eq!(
            parse_pictures_dir("XDG_PICTURES_DIR=\"\"\n", "/home/agl"),
            None
        );
    }

    /// The regression this guards: with a second-granularity name and a plain write, the second
    /// save silently overwrote the first.
    #[test]
    fn concurrent_saves_do_not_overwrite_each_other() {
        let dir = std::env::temp_dir().join(format!("xdpa-save-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);

        let mut uris = Vec::new();
        for i in 0..8u8 {
            uris.push(save_png_in(&dir, &[i]).expect("save"));
        }

        let unique: std::collections::HashSet<_> = uris.iter().collect();
        assert_eq!(unique.len(), uris.len(), "filenames collided: {uris:?}");

        let written = std::fs::read_dir(&dir).expect("read dir").count();
        assert_eq!(written, uris.len(), "a save overwrote an earlier one");

        std::fs::remove_dir_all(&dir).expect("cleanup");
    }
}
