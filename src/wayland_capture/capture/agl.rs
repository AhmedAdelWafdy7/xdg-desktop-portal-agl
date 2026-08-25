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
//!
//! Rotated outputs are the awkward case. The protocol says only that clients "can derive the
//! stride and size from the 'wl_output' object", and `wl_output.mode` reports the *physical*
//! mode size — pre-transform. Whether the compositor sizes the shot against that or against the
//! post-transform logical size is an implementation detail of the compositor, not something the
//! protocol pins down. Rather than guess and hand back a wrongly-shaped image, an output whose
//! transform swaps the axes is captured at the mode size first and, if the compositor answers
//! `bad_buffer`, retried once with the axes swapped. Whichever convention the compositor holds
//! to, one of the two is accepted, and a size it rejects costs a rejected request rather than a
//! garbled screenshot.

use std::time::Instant;

use wayland_client::{
    Connection, QueueHandle, WEnum,
    protocol::{
        wl_buffer::WlBuffer, wl_output::WlOutput, wl_registry::WlRegistry, wl_shm::WlShm,
        wl_shm_pool::WlShmPool,
    },
};

use crate::capture::types::{CaptureError, PixelBuffer, PixelData, PixelFormat};
use crate::capture::{Destroy, Guard, allocate_shm, capture_timeout, dispatch_until};
use crate::protocols::agl_screenshooter::client::agl_screenshooter::{
    self, AglScreenshooter, DoneStatus,
};

impl Destroy for AglScreenshooter {
    fn destroy(&self) {
        AglScreenshooter::destroy(self);
    }
}

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

// Result of one take_shot at one candidate geometry. `bad_buffer` is separated out because it is
// the compositor telling us the geometry was wrong, which is recoverable by trying the other one.
enum Shot {
    Captured(PixelBuffer),
    BadBuffer,
}

impl AglScreenshot {
    pub fn new(screenshooter_name: u32, screenshooter_version: u32, shm: WlShm) -> Self {
        Self {
            screenshooter_name,
            screenshooter_version,
            shm,
        }
    }

    /// Capture `output` into a [`PixelBuffer`] using XRGB8888.
    ///
    /// `width`/`height` are the output's current mode size in physical pixels; `swaps_axes` says
    /// whether the output's transform rotates by 90 or 270 degrees, in which case the logical
    /// size is the transpose and both are tried.
    pub fn capture(
        &self,
        conn: &Connection,
        registry: &WlRegistry,
        output: &WlOutput,
        width: i32,
        height: i32,
        swaps_axes: bool,
    ) -> Result<PixelBuffer, CaptureError> {
        if width <= 0 || height <= 0 {
            return Err(CaptureError::CaptureFailed(
                "output has unknown dimensions".into(),
            ));
        }

        let mut event_queue = conn.new_event_queue::<AglState>();
        let qh = event_queue.handle();
        let mut state = AglState {
            status: Status::Pending,
        };
        let deadline = Instant::now() + capture_timeout();

        let mut screenshooter = Guard::new(registry.bind::<AglScreenshooter, _, _>(
            self.screenshooter_name,
            self.screenshooter_version,
            &qh,
            (),
        ));

        let mode = (width as u32, height as u32);
        let candidates: &[(u32, u32)] = if swaps_axes {
            tracing::warn!(
                "output transform swaps axes; agl_screenshooter does not report a capture size, \
                 so {}x{} is tried first and {}x{} on rejection",
                mode.0,
                mode.1,
                mode.1,
                mode.0
            );
            &[mode, (mode.1, mode.0)]
        } else {
            std::slice::from_ref(&mode)
        };

        for (i, &(w, h)) in candidates.iter().enumerate() {
            match self.take_shot(
                conn,
                &mut screenshooter,
                output,
                &qh,
                &mut event_queue,
                &mut state,
                deadline,
                w,
                h,
            )? {
                Shot::Captured(buffer) => return Ok(buffer),
                Shot::BadBuffer if i + 1 < candidates.len() => {
                    tracing::warn!("agl_screenshooter rejected a {w}x{h} buffer; retrying swapped");
                }
                Shot::BadBuffer => {
                    return Err(CaptureError::CaptureFailed(format!(
                        "agl_screenshooter rejected a {w}x{h} buffer for this output"
                    )));
                }
            }
        }

        unreachable!("the candidate list is never empty")
    }

    /// One `take_shot` at one geometry.
    #[allow(clippy::too_many_arguments)]
    fn take_shot(
        &self,
        conn: &Connection,
        screenshooter: &mut Guard<AglScreenshooter>,
        output: &WlOutput,
        qh: &QueueHandle<AglState>,
        event_queue: &mut wayland_client::EventQueue<AglState>,
        state: &mut AglState,
        deadline: Instant,
        width: u32,
        height: u32,
    ) -> Result<Shot, CaptureError> {
        let format = PixelFormat::Xrgb8888;
        let bpp = format.bytes_per_pixel().expect("xrgb8888 has known bpp");
        let shm_format = format
            .to_wl_shm_format()
            .expect("xrgb8888 has a wl_shm format");

        let stride = width * bpp as u32;
        let size = PixelBuffer::expected_size(stride, height)
            .ok_or_else(|| CaptureError::CaptureFailed("buffer size overflow".into()))?;

        // Allocate before binding any protocol objects, so a failure here has nothing to clean up.
        let (fd, mapping) = allocate_shm(size)?;

        let (pool, buffer) = crate::capture::create_shm_buffer(
            &self.shm,
            qh,
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
        screenshooter.get().take_shot(output, buffer.get());

        if let Err(e) = dispatch_until(conn, event_queue, state, deadline, |s| {
            !matches!(s.status, Status::Pending)
        }) {
            // After a Timeout the compositor may still be writing into `buffer`. Destroying the
            // screenshooter is what abandons the shot, so it has to go before the buffer and pool
            // guards below release the memory it would be writing into.
            screenshooter.destroy_now();
            return Err(e);
        }

        match &state.status {
            // `mapping` moves into the buffer here, so reading it later costs no extra copy;
            // it unmaps automatically once the caller drops the buffer.
            Status::Done(WEnum::Value(DoneStatus::Success)) => Ok(Shot::Captured(PixelBuffer {
                data: PixelData::Mapped(mapping),
                width,
                height,
                stride,
                format,
            })),
            Status::Done(WEnum::Value(DoneStatus::BadBuffer)) => Ok(Shot::BadBuffer),
            Status::Done(other) => Err(CaptureError::CaptureFailed(format!(
                "agl_screenshooter reported {other:?}"
            ))),
            Status::Pending => unreachable!("dispatch_until only returns Ok when done"),
        }
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
