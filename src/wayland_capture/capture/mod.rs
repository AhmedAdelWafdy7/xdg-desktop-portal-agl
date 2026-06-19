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

pub mod types;

use libc;
use std::os::unix::io::{AsRawFd, BorrowedFd, FromRawFd, OwnedFd};
use std::sync::{Arc, Mutex};

use wayland_client::{
    QueueHandle,
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
use types::{CaptureState, PixelBuffer, PixelFormat};

// Abstract API for Capture backend.
pub trait CaptureBackend {
    fn capture(
        &mut self,
        output: &WlOutput,
        event_queue: &mut wayland_client::EventQueue<WlrScreencopyState>,
    ) -> Result<PixelBuffer, CaptureError>;
}

// Dispatch state for the screencopy protocol.
pub struct WlrScreencopyState {
    pub state: Arc<Mutex<CaptureState>>,
    pub shm: WlShm,
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
                let pixel_format = PixelFormat::from_raw(format.into());
                let size = PixelBuffer::expected_size(stride, height).unwrap_or(0);
                *state_lock = CaptureState::BufferAllocated {
                    width,
                    height,
                    stride,
                    format: pixel_format,
                    data: vec![0u8; size],
                };
            }

            Event::Ready { .. } => {
                let prev = std::mem::replace(&mut *state_lock, CaptureState::Failed);
                if let CaptureState::BufferAllocated {
                    width,
                    height,
                    stride,
                    format,
                    data,
                } = prev
                {
                    let pixel_buffer = PixelBuffer {
                        data,
                        width,
                        height,
                        stride,
                        format,
                    };
                    *state_lock = CaptureState::Ready(pixel_buffer);
                } else {
                    *state_lock = CaptureState::Failed;
                }
            }

            Event::Failed => {
                *state_lock = CaptureState::Failed;
            }

            Event::BufferDone => {}

            _ => {}
        }
    }
}

impl wayland_client::Dispatch<WlShmPool, ()> for WlrScreencopyState {
    fn event(
        _state: &mut Self,
        _proxy: &WlShmPool,
        _event: wayland_client::protocol::wl_shm_pool::Event,
        _data: &(),
        _conn: &wayland_client::Connection,
        _qhandle: &QueueHandle<Self>,
    ) {
        // No events to handle for WlShmPool in this context.
    }
}

impl wayland_client::Dispatch<WlBuffer, ()> for WlrScreencopyState {
    fn event(
        _state: &mut Self,
        _proxy: &WlBuffer,
        _event: wayland_client::protocol::wl_buffer::Event,
        _data: &(),
        _conn: &wayland_client::Connection,
        _qhandle: &QueueHandle<Self>,
    ) {
        // No events to handle for WlBuffer in this context.
    }
}

impl wayland_client::Dispatch<WlShm, ()> for WlrScreencopyState {
    fn event(
        _state: &mut Self,
        _proxy: &WlShm,
        _event: wayland_client::protocol::wl_shm::Event,
        _data: &(),
        _conn: &wayland_client::Connection,
        _qhandle: &QueueHandle<Self>,
    ) {
        // No events to handle for WlShm in this context.
    }
}

/// Concrete implementation of the CaptureBackend trait for Wayland using the wlr-screencopy protocol.
/// wlr-screencopy is a protocol extension for Wayland compositors that allows clients to capture the contents of outputs (screens) in a secure and efficient manner. This struct encapsulates the necessary Wayland objects and state to perform screen capture operations.
/// It works for both wlroots-based compositors and agl-compositors.
pub struct WlrScreencopy {
    manager: ZwlrScreencopyManagerV1,
    shm: WlShm,
}

impl WlrScreencopy {
    pub fn new(manager: ZwlrScreencopyManagerV1, shm: WlShm) -> Self {
        Self { manager, shm }
    }

    /// Allocate an anonymous shared memory file descriptor of the specified size using memfd_create, and return it as an OwnedFd. Returns an error if the allocation fails.
    pub fn allocate_shm(size: usize) -> Result<(OwnedFd, *mut libc::c_void), CaptureError> {
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
            Ok((OwnedFd::from_raw_fd(fd), ptr))
        }
    }
}

impl CaptureBackend for WlrScreencopy {
    fn capture(
        &mut self,
        output: &WlOutput,
        event_queue: &mut wayland_client::EventQueue<WlrScreencopyState>,
    ) -> Result<PixelBuffer, CaptureError> {
        let shared_state = Arc::new(Mutex::new(CaptureState::Pending));
        let mut dispatch_state = WlrScreencopyState {
            state: shared_state.clone(),
            shm: self.shm.clone(),
        };

        let qh = event_queue.handle();
        // request a frame, compositor will send buffer + buffer_done events, then ready or failed.
        let frame = self.manager.capture_output(0, output, &qh, ());

        event_queue
            .roundtrip(&mut dispatch_state)
            .map_err(|e| CaptureError::WaylandError(e.to_string()))?;

        // inspect to know dimensions
        let (width, height, stride, format) = {
            let state_lock = shared_state.lock().unwrap();
            match &*state_lock {
                CaptureState::BufferAllocated {
                    width,
                    height,
                    stride,
                    format,
                    ..
                } => (*width, *height, *stride, *format),
                _ => {
                    return Err(CaptureError::CaptureFailed(
                        "Failed to allocate buffer".to_string(),
                    ));
                }
            }
        };

        let size = PixelBuffer::expected_size(stride, height).ok_or(
            CaptureError::CaptureFailed("Buffer size overflow".to_string()),
        )?;

        let (fd, ptr) = Self::allocate_shm(size)?;

        let borrowed_fd = unsafe { BorrowedFd::borrow_raw(fd.as_raw_fd()) };

        let pool = dispatch_state
            .shm
            .create_pool(borrowed_fd, size as i32, &qh, ());
        let buffer = pool.create_buffer(
            0,
            width as i32,
            height as i32,
            stride as i32,
            match format {
                PixelFormat::Argb8888 => Format::Argb8888,
                PixelFormat::Xrgb8888 => Format::Xrgb8888,
                PixelFormat::Xbgr8888 => Format::Xbgr8888,
                PixelFormat::Abgr8888 => Format::Abgr8888,
                PixelFormat::Rgb565 => Format::Rgb565,
                PixelFormat::Invalid | PixelFormat::Unknown(_) => {
                    return Err(CaptureError::CaptureFailed(
                        "Invalid pixel format".to_string(),
                    ));
                }
            },
            &qh,
            (),
        );

        // copy frame to buffer
        frame.copy(&buffer);

        // wait for ready or failed
        event_queue
            .roundtrip(&mut dispatch_state)
            .map_err(|e| CaptureError::WaylandError(e.to_string()))?;

        let pixel_data = unsafe { std::slice::from_raw_parts(ptr as *const u8, size) }.to_vec();

        unsafe {
            libc::munmap(ptr, size);
        }

        {
            let mut state_lock = shared_state.lock().unwrap();
            if let CaptureState::Ready(ref mut pixel_buffer) = *state_lock {
                pixel_buffer.data = pixel_data;
            }
        }

        let final_state = Arc::try_unwrap(shared_state)
            .map_err(|_| CaptureError::CaptureFailed("Failed to unwrap shared state".to_string()))?
            .into_inner()
            .unwrap();
        match final_state {
            CaptureState::Ready(pixel_buffer) => Ok(pixel_buffer),
            CaptureState::Failed => Err(CaptureError::CaptureFailed("Capture failed".to_string())),
            _ => Err(CaptureError::CaptureFailed(
                "Unexpected capture state".to_string(),
            )),
        }
    }
}
