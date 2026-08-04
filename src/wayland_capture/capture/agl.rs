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

//! Fallback capture backend built on the legacy `agl_screenshooter` protocol. Unlike
//! weston-output-capture, this protocol delivers no format/size events: the client derives the
//! buffer geometry from the `wl_output` and picks the format. The AGL reference client used
//! XRGB8888, which we mirror here.
//!
//! The `done` event is emitted on the screenshooter global itself, so the backend rebinds the
//! global onto its own event queue to receive completion on the same queue it dispatches.

use std::time::Instant;

use wayland_client::{
    Connection, QueueHandle, WEnum,
    protocol::{
        wl_buffer::WlBuffer, wl_output::WlOutput, wl_registry::WlRegistry, wl_shm::WlShm,
        wl_shm_pool::WlShmPool,
    },
};

use crate::capture::types::{CaptureError, PixelBuffer, PixelData, PixelFormat};
use crate::capture::{allocate_shm, capture_timeout, dispatch_until};
use crate::protocols::agl_screenshooter::client::agl_screenshooter::{
    self, AglScreenshooter, DoneStatus,
};

/// Fallback capture backend using `agl_screenshooter`.
pub struct AglScreenshot {
    /// Registry global name of the `agl_screenshooter`, so it can be rebound per capture.
    screenshooter_name: u32,
    screenshooter_version: u32,
    shm: WlShm,
}

// Outcome of a single take_shot request.
#[derive(Debug, Clone)]
enum Status {
    Pending,
    Done(WEnum<DoneStatus>),
}

struct AglState {
    status: Status,
}

impl AglScreenshot {
    pub fn new(screenshooter_name: u32, screenshooter_version: u32, shm: WlShm) -> Self {
        Self {
            screenshooter_name,
            screenshooter_version,
            shm,
        }
    }

    /// Capture `output` (of size `width` x `height`) into a [`PixelBuffer`] using XRGB8888.
    pub fn capture(
        &self,
        conn: &Connection,
        registry: &WlRegistry,
        output: &WlOutput,
        width: i32,
        height: i32,
    ) -> Result<PixelBuffer, CaptureError> {
        if width <= 0 || height <= 0 {
            return Err(CaptureError::CaptureFailed(
                "output has unknown dimensions".into(),
            ));
        }

        let format = PixelFormat::Xrgb8888;
        let bpp = format.bytes_per_pixel().expect("xrgb8888 has known bpp");
        let shm_format = format
            .to_wl_shm_format()
            .expect("xrgb8888 has a wl_shm format");

        let width_u = width as u32;
        let height_u = height as u32;
        let stride = width_u * bpp as u32;
        let size = PixelBuffer::expected_size(stride, height_u)
            .ok_or_else(|| CaptureError::CaptureFailed("buffer size overflow".into()))?;

        // Allocate before binding any protocol objects, so a failure here has nothing to clean up.
        let (fd, mapping) = allocate_shm(size)?;

        let mut event_queue = conn.new_event_queue::<AglState>();
        let qh = event_queue.handle();
        let mut state = AglState {
            status: Status::Pending,
        };
        let deadline = Instant::now() + capture_timeout();

        let screenshooter: AglScreenshooter =
            registry.bind(self.screenshooter_name, self.screenshooter_version, &qh, ());

        let (pool, buffer) = crate::capture::create_shm_buffer(
            &self.shm,
            &qh,
            &fd,
            width,
            height,
            stride as i32,
            shm_format,
            size,
        );

        screenshooter.take_shot(output, &buffer);

        let result = dispatch_until(conn, &mut event_queue, &mut state, deadline, |s| {
            !matches!(s.status, Status::Pending)
        })
        .and_then(|()| match &state.status {
            // `mapping` moves into the buffer here, so reading it later costs no extra copy;
            // it unmaps automatically once the caller drops the buffer.
            Status::Done(WEnum::Value(DoneStatus::Success)) => Ok(PixelBuffer {
                data: PixelData::Mapped(mapping),
                width: width_u,
                height: height_u,
                stride,
                format,
            }),
            Status::Done(other) => Err(CaptureError::CaptureFailed(format!(
                "agl_screenshooter reported {other:?}"
            ))),
            Status::Pending => unreachable!("dispatch_until only returns Ok when done"),
        });

        buffer.destroy();
        pool.destroy();
        screenshooter.destroy();

        result
    }
}

impl wayland_client::Dispatch<AglScreenshooter, ()> for AglState {
    fn event(
        state: &mut Self,
        _proxy: &AglScreenshooter,
        event: agl_screenshooter::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        let agl_screenshooter::Event::Done { status } = event;
        state.status = Status::Done(status);
    }
}

crate::capture::impl_noop_dispatch!(AglState, WlShmPool, WlBuffer);
