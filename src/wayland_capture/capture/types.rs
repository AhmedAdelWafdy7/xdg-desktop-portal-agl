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

use thiserror::Error;

#[derive(Error, Debug)]
pub enum CaptureError {
    #[error("SHM allocation failed: {0}")]
    ShmAllocationFailed(#[from] std::io::Error),
    #[error("Wayland error: {0}")]
    WaylandError(String),
    #[error("Capture failed: {0}")]
    CaptureFailed(String),
}

// Pixel format of the captured frame. Mirrioring the wl_shm_format on embedded targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelFormat {
    Xrgb8888,
    Argb8888,
    Xbgr8888,
    Abgr8888,
    Rgb565,
    Invalid,
    /// Any format the compositor reported that this crate does not yet name.
    Unknown(u32),
}

impl PixelFormat {
    // Convert a raw u32 format code to a PixelFormat enum variant.
    pub fn from_raw(raw: u32) -> Self {
        match raw {
            0x00000000 => Self::Argb8888,
            0x00000001 => Self::Xrgb8888,
            0x34324241 => Self::Abgr8888, // 'AB24'
            0x34324258 => Self::Xbgr8888, // 'XB24'
            other => Self::Unknown(other),
        }
    }

    // Returns the number of bytes per pixel for this format, if known.
    pub fn bytes_per_pixel(self) -> Option<usize> {
        match self {
            Self::Argb8888 | Self::Xrgb8888 | Self::Abgr8888 | Self::Xbgr8888 => Some(4),
            Self::Rgb565 => Some(2),
            Self::Invalid | Self::Unknown(_) => None,
        }
    }
}

#[derive(Debug)]
pub struct PixelBuffer {
    pub data: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub format: PixelFormat,
}

impl PixelBuffer {
    // Total Expected size of the pixel buffer in bytes, calculated from width, height, stride, and format. Returns None in overflow.
    pub fn expected_size(stride: u32, height: u32) -> Option<usize> {
        let size = (stride as u64).checked_mul(height as u64)? as usize;
        const MAX_BUFFER_SIZE: usize = 4 * 1024 * 1024 * 1024; // 4 GiB
        if size > MAX_BUFFER_SIZE {
            None
        } else {
            Some(size)
        }
    }

    /// Slice the pixel buffer data into rows based on the stride.
    pub fn row(&self, y: u32) -> Option<&[u8]> {
        let start = y.checked_mul(self.stride)? as usize;
        let end = start.checked_add(self.stride as usize)?;
        self.data.get(start..end)
    }
}

// State machine for a single screencopy frame capture operation.
// Driven forward by wayland dispatch callbacks.
#[derive(Debug)]
pub enum CaptureState {
    Pending, // Frame Requested, waiting for compositor buffer event.
    BufferAllocated {
        // Compositor told us the buffer size and format, we allocated a local buffer to receive the data.
        width: u32,
        height: u32,
        stride: u32,
        format: PixelFormat,
        data: Vec<u8>,
    },

    Ready(PixelBuffer), // Buffer is ready to be read by the caller.
    Failed,
}

impl CaptureState {
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Ready(_) | Self::Failed)
    }

    pub fn take_buffer(self) -> Option<PixelBuffer> {
        match self {
            Self::Ready(buf) => Some(buf),
            _ => None,
        }
    }
}
