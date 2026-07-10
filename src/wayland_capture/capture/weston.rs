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

use std::os::unix::io::{AsRawFd, BorrowedFd};

use wayland_client::{
    Connection, QueueHandle,
    protocol::{wl_buffer::WlBuffer, wl_output::WlOutput, wl_shm::WlShm, wl_shm_pool::WlShmPool},
};

use crate::capture::allocate_shm;
use crate::capture::types::{CaptureError, PixelBuffer, PixelFormat};
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
}

impl WestonState {
    fn new() -> Self {
        Self {
            format: None,
            width: 0,
            height: 0,
            status: Status::Pending,
        }
    }
}

impl WestonCapture {
    pub fn new(factory: WestonCaptureV1, shm: WlShm) -> Self {
        Self { factory, shm }
    }

    /// Capture the given output into a [`PixelBuffer`]. Uses the `framebuffer` pixel source,
    /// which libweston guarantees is always available.
    pub fn capture(
        &self,
        conn: &Connection,
        output: &WlOutput,
    ) -> Result<PixelBuffer, CaptureError> {
        let mut event_queue = conn.new_event_queue::<WestonState>();
        let qh = event_queue.handle();
        let mut state = WestonState::new();

        let source = self.factory.create(output, Source::Framebuffer, &qh, ());

        // Receive the initial `format` and `size` events describing the required buffer.
        event_queue
            .roundtrip(&mut state)
            .map_err(|e| CaptureError::WaylandError(format!("weston roundtrip: {e}")))?;

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
            // weston-output-capture requires 4-byte row alignment and no extra padding.
            let stride = width * bpp as u32;
            let size = PixelBuffer::expected_size(stride, height)
                .ok_or_else(|| CaptureError::CaptureFailed("buffer size overflow".into()))?;

            let (fd, ptr) = allocate_shm(size)?;
            let borrowed = unsafe { BorrowedFd::borrow_raw(fd.as_raw_fd()) };
            let pool = self.shm.create_pool(borrowed, size as i32, &qh, ());
            let buffer = pool.create_buffer(
                0,
                width as i32,
                height as i32,
                stride as i32,
                shm_format,
                &qh,
                (),
            );

            state.status = Status::Pending;
            source.capture(&buffer);

            while matches!(state.status, Status::Pending) {
                event_queue
                    .blocking_dispatch(&mut state)
                    .map_err(|e| CaptureError::WaylandError(format!("weston dispatch: {e}")))?;
            }

            let outcome = state.status.clone();
            match outcome {
                Status::Complete => {
                    let data =
                        unsafe { std::slice::from_raw_parts(ptr as *const u8, size) }.to_vec();
                    unsafe { libc::munmap(ptr, size) };
                    buffer.destroy();
                    pool.destroy();
                    source.destroy();
                    return Ok(PixelBuffer {
                        data,
                        width,
                        height,
                        stride,
                        format,
                    });
                }
                Status::Retry => {
                    // New format/size events already updated `state`; reallocate and retry.
                    unsafe { libc::munmap(ptr, size) };
                    buffer.destroy();
                    pool.destroy();
                    continue;
                }
                Status::Failed(msg) => {
                    unsafe { libc::munmap(ptr, size) };
                    buffer.destroy();
                    pool.destroy();
                    source.destroy();
                    return Err(CaptureError::CaptureFailed(format!(
                        "weston capture failed: {msg}"
                    )));
                }
                Status::Pending => unreachable!("loop exits only on a terminal status"),
            }
        }

        source.destroy();
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

impl wayland_client::Dispatch<WlShmPool, ()> for WestonState {
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

impl wayland_client::Dispatch<WlBuffer, ()> for WestonState {
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
