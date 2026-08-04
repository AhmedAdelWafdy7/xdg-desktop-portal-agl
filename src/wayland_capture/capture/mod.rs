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

pub mod agl;
pub mod types;
pub mod weston;

use libc;
use std::os::unix::io::{AsRawFd, BorrowedFd, FromRawFd, OwnedFd};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use wayland_client::{
    Connection, EventQueue, QueueHandle,
    protocol::{
        wl_buffer::WlBuffer,
        wl_output::WlOutput,
        wl_shm::{Format, WlShm},
        wl_shm_pool::WlShmPool,
    },
};

use wayland_protocols_wlr::screencopy::v1::client::{
    zwlr_screencopy_frame_v1::{self, ZwlrScreencopyFrameV1},
    zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1,
};

use types::CaptureError;
use types::{CaptureState, PixelBuffer, PixelData, PixelFormat, ShmMapping};

// Abstract API for Capture backend.
pub trait CaptureBackend {
    fn capture(
        &mut self,
        conn: &Connection,
        output: &WlOutput,
        event_queue: &mut wayland_client::EventQueue<WlrScreencopyState>,
    ) -> Result<PixelBuffer, CaptureError>;
}

macro_rules! impl_noop_dispatch {
    ($state:ty, $($proxy:ty),+ $(,)?) => {
        $(
            impl wayland_client::Dispatch<$proxy, ()> for $state {
                fn event(
                    _state: &mut Self,
                    _proxy: &$proxy,
                    _event: <$proxy as wayland_client::Proxy>::Event,
                    _data: &(),
                    _conn: &wayland_client::Connection,
                    _qh: &QueueHandle<Self>,
                ) {
                }
            }
        )+
    };
}
pub(crate) use impl_noop_dispatch;

/// Bind a `wl_shm_pool`/`wl_buffer` pair over `fd` and hand back the buffer alongside the pool
/// that owns it (callers must `destroy()` both once done). Shared by every wl_shm-based
/// backend, which otherwise each repeat this same `BorrowedFd` + two protocol requests.
#[allow(clippy::too_many_arguments)]
pub(crate) fn create_shm_buffer<S>(
    shm: &WlShm,
    qh: &QueueHandle<S>,
    fd: &OwnedFd,
    width: i32,
    height: i32,
    stride: i32,
    format: Format,
    size: usize,
) -> (WlShmPool, WlBuffer)
where
    S: wayland_client::Dispatch<WlShmPool, ()> + wayland_client::Dispatch<WlBuffer, ()> + 'static,
{
    let borrowed = unsafe { BorrowedFd::borrow_raw(fd.as_raw_fd()) };
    let pool = shm.create_pool(borrowed, size as i32, qh, ());
    let buffer = pool.create_buffer(0, width, height, stride, format, qh, ());
    (pool, buffer)
}

// Dispatch state for the screencopy protocol.
pub struct WlrScreencopyState {
    pub state: Arc<Mutex<CaptureState>>,
}

impl wayland_client::Dispatch<ZwlrScreencopyFrameV1, ()> for WlrScreencopyState {
    fn event(
        state: &mut Self,
        _proxy: &ZwlrScreencopyFrameV1,
        event: zwlr_screencopy_frame_v1::Event,
        _data: &(),
        _conn: &wayland_client::Connection,
        _qhandle: &QueueHandle<Self>,
    ) {
        use zwlr_screencopy_frame_v1::Event;
        let mut state_lock = state.state.lock().unwrap();

        match event {
            Event::Buffer {
                format,
                width,
                height,
                stride,
            } => {
                *state_lock = CaptureState::BufferAllocated {
                    width,
                    height,
                    stride,
                    format: PixelFormat::from_raw(format.into()),
                };
            }

            Event::Ready { .. } => {
                let prev = std::mem::replace(&mut *state_lock, CaptureState::Failed);
                if let CaptureState::BufferAllocated {
                    width,
                    height,
                    stride,
                    format,
                } = prev
                {
                    // The pixels are in the caller's shm mapping; `data` is filled in there.
                    *state_lock = CaptureState::Ready(PixelBuffer {
                        data: PixelData::Owned(Vec::new()),
                        width,
                        height,
                        stride,
                        format,
                    });
                } else {
                    *state_lock = CaptureState::Failed;
                }
            }

            Event::Failed => {
                *state_lock = CaptureState::Failed;
            }

            // `buffer_done` signals no more buffer offers are coming.
            _ => {}
        }
    }
}

impl_noop_dispatch!(WlrScreencopyState, WlShmPool, WlBuffer, WlShm);

/// How long a single capture may wait on the compositor before giving up.
pub(crate) fn capture_timeout() -> Duration {
    std::env::var("XDP_AGL_CAPTURE_TIMEOUT_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .map(Duration::from_millis)
        .unwrap_or(Duration::from_secs(5))
}

/// Dispatch events on `queue` until `done(state)` is true or `deadline` passes.
pub(crate) fn dispatch_until<S, F>(
    conn: &Connection,
    queue: &mut EventQueue<S>,
    state: &mut S,
    deadline: Instant,
    done: F,
) -> Result<(), CaptureError>
where
    F: Fn(&S) -> bool,
{
    let budget = capture_timeout();
    loop {
        queue
            .dispatch_pending(state)
            .map_err(|e| CaptureError::WaylandError(format!("dispatch: {e}")))?;
        if done(state) {
            return Ok(());
        }

        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(CaptureError::Timeout(budget));
        }

        queue
            .flush()
            .map_err(|e| CaptureError::WaylandError(format!("flush: {e}")))?;

        // `None` means events arrived between the dispatch above and here; go round again.
        let Some(guard) = queue.prepare_read() else {
            continue;
        };

        let mut pfd = libc::pollfd {
            fd: conn.backend().poll_fd().as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };

        let timeout_ms = remaining
            .as_nanos()
            .div_ceil(1_000_000)
            .min(i32::MAX as u128) as i32;
        let rc = unsafe { libc::poll(&mut pfd, 1, timeout_ms) };

        match rc {
            // Re-check the deadline at the top of the loop rather than assuming it has passed.
            0 => continue,
            n if n < 0 => {
                let err = std::io::Error::last_os_error();
                // Drop the read guard before retrying, or the next prepare_read deadlocks.
                drop(guard);
                if err.kind() == std::io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(CaptureError::WaylandError(format!("poll: {err}")));
            }
            _ => {
                guard
                    .read()
                    .map_err(|e| CaptureError::WaylandError(format!("read events: {e}")))?;
            }
        }
    }
}

/// Allocate an anonymous shared-memory file of `size` bytes via `memfd_create`, map it, and
/// return the owning fd plus the mapping. Shared by all wl_shm-based capture backends.
pub(crate) fn allocate_shm(size: usize) -> Result<(OwnedFd, ShmMapping), CaptureError> {
    let fd = unsafe { libc::memfd_create(c"wl_shm".as_ptr(), libc::MFD_CLOEXEC) };
    if fd < 0 {
        return Err(CaptureError::ShmAllocationFailed(
            std::io::Error::last_os_error(),
        ));
    }
    unsafe {
        if libc::ftruncate(fd, size as libc::off_t) < 0 {
            libc::close(fd);
            return Err(CaptureError::ShmAllocationFailed(
                std::io::Error::last_os_error(),
            ));
        }
        let ptr = libc::mmap(
            std::ptr::null_mut(),
            size,
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_SHARED,
            fd,
            0,
        );
        if ptr == libc::MAP_FAILED {
            libc::close(fd);
            return Err(CaptureError::ShmAllocationFailed(
                std::io::Error::last_os_error(),
            ));
        }
        Ok((OwnedFd::from_raw_fd(fd), ShmMapping::new(ptr, size)))
    }
}

/// Capture backend for `zwlr_screencopy_v1`.
///
/// This is the wlroots-only backend, and the lowest-precedence one: no libweston/AGL compositor
/// advertises `zwlr_screencopy_manager_v1`. It exists so the portal is usable on a wlroots dev
/// desktop (and wlroots-based kiosks such as cage/sway); on an AGL target the weston or agl
/// backend is selected instead.
pub struct WlrScreencopy {
    manager: ZwlrScreencopyManagerV1,
    shm: WlShm,
}

impl WlrScreencopy {
    pub fn new(manager: ZwlrScreencopyManagerV1, shm: WlShm) -> Self {
        Self { manager, shm }
    }
}

impl CaptureBackend for WlrScreencopy {
    fn capture(
        &mut self,
        conn: &Connection,
        output: &WlOutput,
        event_queue: &mut wayland_client::EventQueue<WlrScreencopyState>,
    ) -> Result<PixelBuffer, CaptureError> {
        let shared_state = Arc::new(Mutex::new(CaptureState::Pending));
        let mut dispatch_state = WlrScreencopyState {
            state: shared_state.clone(),
        };

        let qh = event_queue.handle();
        let deadline = Instant::now() + capture_timeout();
        // request a frame, compositor will send buffer (+ buffer_done on protocol v3+), then ready or failed.
        let frame = self.manager.capture_output(0, output, &qh, ());

        // Wait for the first `Buffer` event, which is present on every protocol version
        let geometry = dispatch_until(conn, event_queue, &mut dispatch_state, deadline, |s| {
            !matches!(&*s.state.lock().unwrap(), CaptureState::Pending)
        })
        .and_then(|()| match &*shared_state.lock().unwrap() {
            CaptureState::BufferAllocated {
                width,
                height,
                stride,
                format,
            } => Ok((*width, *height, *stride, *format)),
            _ => Err(CaptureError::CaptureFailed(
                "Failed to allocate buffer".to_string(),
            )),
        });

        let (width, height, stride, format) = match geometry {
            Ok(g) => g,
            Err(e) => {
                frame.destroy();
                return Err(e);
            }
        };

        let shm_format = match format.to_wl_shm_format() {
            Some(f) => f,
            None => {
                frame.destroy();
                return Err(CaptureError::UnsupportedFormat(format));
            }
        };

        let size = match PixelBuffer::expected_size(stride, height) {
            Some(size) => size,
            None => {
                frame.destroy();
                return Err(CaptureError::CaptureFailed(
                    "Buffer size overflow".to_string(),
                ));
            }
        };

        let (fd, mapping) = match allocate_shm(size) {
            Ok(v) => v,
            Err(e) => {
                frame.destroy();
                return Err(e);
            }
        };

        let (pool, buffer) = create_shm_buffer(
            &self.shm,
            &qh,
            &fd,
            width as i32,
            height as i32,
            stride as i32,
            shm_format,
            size,
        );

        // copy frame to buffer
        frame.copy(&buffer);

        // The compositor performs the copy at the next output repaint, which a wl_display.sync
        // callback can outrun. Wait on the terminal state, not on a roundtrip.
        let outcome = dispatch_until(conn, event_queue, &mut dispatch_state, deadline, |s| {
            s.state.lock().unwrap().is_terminal()
        });

        let result = outcome.and_then(|()| {
            let final_state =
                std::mem::replace(&mut *shared_state.lock().unwrap(), CaptureState::Pending);
            match final_state.take_buffer() {
                // Move the mapping straight into the buffer: reading through it later avoids
                // the full-frame copy a `.to_vec()` here would cost.
                Some(mut pixel_buffer) => {
                    pixel_buffer.data = PixelData::Mapped(mapping);
                    Ok(pixel_buffer)
                }
                None => Err(CaptureError::CaptureFailed("Capture failed".to_string())),
            }
        });

        buffer.destroy();
        pool.destroy();
        frame.destroy();

        result
    }
}
