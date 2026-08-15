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

//! Capture backend built on the `weston-output-capture` protocol (`weston_capture_v1`). This is
//! the primary backend for the AGL compositor, which exposes this global via libweston and uses
//! it for its own reference screenshot client.
//!
//! The pixel source is negotiated rather than hardcoded. `framebuffer` is always available but
//! temporarily disables hardware planes, so on an IVI system where video is offloaded to a KMS
//! overlay plane it captures black where the video is. `writeback` keeps those planes in use, so
//! it is tried first and we fall back when the compositor does not offer it. Set
//! `XDP_AGL_CAPTURE_SOURCE` to `writeback`, `framebuffer`, `full-framebuffer` or `blending` to
//! bypass negotiation.

use std::time::Instant;

trait Destroy {
    fn destroy(&self);
}

impl Destroy for WestonCaptureSourceV1 {
    fn destroy(&self) {
        WestonCaptureSourceV1::destroy(self);
    }
}

impl Destroy for WlShmPool {
    fn destroy(&self) {
        WlShmPool::destroy(self);
    }
}

impl Destroy for WlBuffer {
    fn destroy(&self) {
        WlBuffer::destroy(self);
    }
}

struct Guard<T: Destroy>(Option<T>);

impl<T: Destroy> Guard<T> {
    fn new(v: T) -> Self {
        Self(Some(v))
    }

    fn get(&self) -> &T {
        self.0.as_ref().expect("guard used after being disarmed")
    }

    fn disarm(mut self) -> T {
        self.0.take().unwrap()
    }
}

impl<T: Destroy> Drop for Guard<T> {
    fn drop(&mut self) {
        if let Some(v) = &self.0 {
            v.destroy();
        }
    }
}

use wayland_client::{
    Connection, QueueHandle,
    protocol::{
        wl_buffer::WlBuffer, wl_callback::WlCallback, wl_output::WlOutput, wl_shm::WlShm,
        wl_shm_pool::WlShmPool,
    },
};

use crate::capture::types::{CaptureError, PixelBuffer, PixelData, PixelFormat};
use crate::capture::{allocate_shm, capture_timeout, dispatch_until};
use crate::protocols::weston_output_capture::client::weston_capture_source_v1::{
    self, WestonCaptureSourceV1,
};
use crate::protocols::weston_output_capture::client::weston_capture_v1::{Source, WestonCaptureV1};

/// Maximum number of `retry` cycles honoured before giving up. A retry happens when the buffer
/// parameters change between the initial events and the capture; a couple should always suffice.
const MAX_ATTEMPTS: usize = 4;

/// Capture backend using `weston_capture_v1`.
pub struct WestonCapture {
    factory: WestonCaptureV1,
    shm: WlShm,
}

// Terminal outcome of a single `capture` request, driven by the source object's events.
#[derive(Debug, Clone)]
enum Status {
    Pending,
    Complete,
    Retry,
    Failed(String),
}

// Dispatch state for one capture. `format`/`width`/`height` are (re)delivered as initial events
// before every capture attempt; `status` records the outcome of the current attempt.
struct WestonState {
    format: Option<PixelFormat>,
    width: i32,
    height: i32,
    status: Status,
    sync_done: bool, // Set when our wl_display.sync callback fires, bounding the source-availability probe.
}

impl WestonState {
    fn new() -> Self {
        Self {
            format: None,
            width: 0,
            height: 0,
            status: Status::Pending,
            sync_done: false,
        }
    }
}

/// Pixel sources to try, best first. `framebuffer` is the floor: libweston always offers it.
const SOURCE_PREFERENCE: [Source; 2] = [Source::Writeback, Source::Framebuffer];

fn source_from_env() -> Option<Source> {
    match std::env::var("XDP_AGL_CAPTURE_SOURCE").ok()?.as_str() {
        "writeback" => Some(Source::Writeback),
        "framebuffer" => Some(Source::Framebuffer),
        "full-framebuffer" => Some(Source::FullFramebuffer),
        "blending" => Some(Source::Blending),
        _ => None,
    }
}

impl WestonCapture {
    pub fn new(factory: WestonCaptureV1, shm: WlShm) -> Self {
        Self { factory, shm }
    }

    /// Pick the best available pixel source for `output`.
    fn negotiate_source(
        &self,
        conn: &Connection,
        output: &WlOutput,
        qh: &QueueHandle<WestonState>,
        event_queue: &mut wayland_client::EventQueue<WestonState>,
        state: &mut WestonState,
        deadline: Instant,
    ) -> Result<(WestonCaptureSourceV1, Source), CaptureError> {
        let forced = source_from_env();
        let candidates: &[Source] = match &forced {
            Some(source) => std::slice::from_ref(source),
            None => &SOURCE_PREFERENCE,
        };

        for &candidate in candidates {
            state.format = None;
            state.sync_done = false;

            let source = Guard::new(self.factory.create(output, candidate, qh, ()));
            let _sync = conn.display().sync(qh, ());

            dispatch_until(conn, event_queue, state, deadline, |s| {
                (s.format.is_some() && s.width > 0 && s.height > 0) || s.sync_done
            })?;

            if state.format.is_some() {
                tracing::info!("weston capture source: {candidate:?}");
                let source = source.disarm();
                return Ok((source, candidate));
            }
        }

        Err(CaptureError::CaptureFailed(
            "compositor offers no usable weston capture source".into(),
        ))
    }

    /// Capture the given output into a [`PixelBuffer`].
    pub fn capture(
        &self,
        conn: &Connection,
        output: &WlOutput,
    ) -> Result<PixelBuffer, CaptureError> {
        let mut event_queue = conn.new_event_queue::<WestonState>();
        let qh = event_queue.handle();
        let mut state = WestonState::new();
        let deadline = Instant::now() + capture_timeout();

        // Negotiation also delivers the initial `format` and `size` events for the chosen source.
        let (source, _kind) =
            self.negotiate_source(conn, output, &qh, &mut event_queue, &mut state, deadline)?;

        let source = Guard::new(source);
        for _ in 0..MAX_ATTEMPTS {
            let format = state
                .format
                .ok_or_else(|| CaptureError::CaptureFailed("no format from compositor".into()))?;
            if state.width <= 0 || state.height <= 0 {
                return Err(CaptureError::CaptureFailed("invalid capture size".into()));
            }

            let bpp = format
                .bytes_per_pixel()
                .ok_or(CaptureError::UnsupportedFormat(format))?;
            let shm_format = format
                .to_wl_shm_format()
                .ok_or(CaptureError::UnsupportedFormat(format))?;

            let width = state.width as u32;
            let height = state.height as u32;
            let stride = width * bpp as u32;
            if !stride.is_multiple_of(4) {
                return Err(CaptureError::CaptureFailed(format!(
                    "{width}x{height} {format:?} has a {stride}-byte stride, \
                     which weston-output-capture cannot align to 4 bytes without padding"
                )));
            }
            let size = PixelBuffer::expected_size(stride, height)
                .ok_or_else(|| CaptureError::CaptureFailed("buffer size overflow".into()))?;

            let (fd, mapping) = allocate_shm(size)?;
            let (pool, buffer) = crate::capture::create_shm_buffer(
                &self.shm,
                &qh,
                &fd,
                width as i32,
                height as i32,
                stride as i32,
                shm_format,
                size,
            );
            let _pool = Guard::new(pool);
            let buffer = Guard::new(buffer);
            state.status = Status::Pending;
            source.get().capture(buffer.get());

            dispatch_until(conn, &mut event_queue, &mut state, deadline, |s| {
                !matches!(s.status, Status::Pending)
            })?;

            let outcome = state.status.clone();
            match outcome {
                Status::Complete => {
                    // `mapping` moves into the buffer here; reading through it later avoids a
                    // full-frame copy. Unmapped automatically once the caller drops the buffer.
                    return Ok(PixelBuffer {
                        data: PixelData::Mapped(mapping),
                        width,
                        height,
                        stride,
                        format,
                    });
                }
                Status::Retry => {
                    continue;
                }
                Status::Failed(msg) => {
                    return Err(CaptureError::CaptureFailed(format!(
                        "weston capture failed: {msg}"
                    )));
                }
                Status::Pending => unreachable!("loop exits only on a terminal status"),
            }
        }

        Err(CaptureError::CaptureFailed(
            "weston capture exceeded retry limit".into(),
        ))
    }
}

impl wayland_client::Dispatch<WestonCaptureSourceV1, ()> for WestonState {
    fn event(
        state: &mut Self,
        _proxy: &WestonCaptureSourceV1,
        event: weston_capture_source_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        use weston_capture_source_v1::Event;
        match event {
            Event::Format { drm_format } => {
                state.format = Some(PixelFormat::from_drm_fourcc(drm_format));
            }
            Event::Size { width, height } => {
                state.width = width;
                state.height = height;
            }
            Event::Complete => state.status = Status::Complete,
            Event::Retry => state.status = Status::Retry,
            Event::Failed { msg } => {
                state.status = Status::Failed(msg.unwrap_or_else(|| "unspecified".into()));
            }
        }
    }
}

impl wayland_client::Dispatch<WlCallback, ()> for WestonState {
    fn event(
        state: &mut Self,
        _proxy: &WlCallback,
        _event: wayland_client::protocol::wl_callback::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        state.sync_done = true;
    }
}

crate::capture::impl_noop_dispatch!(WestonState, WlShmPool, WlBuffer);
