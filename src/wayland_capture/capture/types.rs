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

use wayland_client::protocol::wl_shm::Format as WlShmFormat;

#[derive(Error, Debug)]
pub enum CaptureError {
    #[error("SHM allocation failed: {0}")]
    ShmAllocationFailed(#[from] std::io::Error),
    #[error("Wayland error: {0}")]
    WaylandError(String),
    #[error("Capture failed: {0}")]
    CaptureFailed(String),
    #[error("Unsupported pixel format: {0:?}")]
    UnsupportedFormat(PixelFormat),
    #[error("PNG encoding failed: {0}")]
    EncodingFailed(String),
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

// DRM FourCC pixel format codes, as delivered by weston-output-capture and other
// DRM-based protocols. These differ from the wl_shm format enum, which special-cases
// ARGB8888 (0) and XRGB8888 (1).
const DRM_FORMAT_ARGB8888: u32 = 0x34325241; // 'AR24'
const DRM_FORMAT_XRGB8888: u32 = 0x34325258; // 'XR24'
const DRM_FORMAT_ABGR8888: u32 = 0x34324241; // 'AB24'
const DRM_FORMAT_XBGR8888: u32 = 0x34324258; // 'XB24'
const DRM_FORMAT_RGB565: u32 = 0x36314752; // 'RG16'

impl PixelFormat {
    // Convert a raw wl_shm format code to a PixelFormat enum variant. wl_shm uses 0 and 1
    // for ARGB8888 and XRGB8888 respectively, and the DRM FourCC value for everything else.
    pub fn from_raw(raw: u32) -> Self {
        match raw {
            0x00000000 => Self::Argb8888,
            0x00000001 => Self::Xrgb8888,
            DRM_FORMAT_ABGR8888 => Self::Abgr8888,
            DRM_FORMAT_XBGR8888 => Self::Xbgr8888,
            DRM_FORMAT_RGB565 => Self::Rgb565,
            other => Self::Unknown(other),
        }
    }

    // Convert a raw DRM FourCC format code to a PixelFormat enum variant. Used by protocols
    // (e.g. weston-output-capture) that report the DRM code rather than the wl_shm enum.
    pub fn from_drm_fourcc(raw: u32) -> Self {
        match raw {
            DRM_FORMAT_ARGB8888 => Self::Argb8888,
            DRM_FORMAT_XRGB8888 => Self::Xrgb8888,
            DRM_FORMAT_ABGR8888 => Self::Abgr8888,
            DRM_FORMAT_XBGR8888 => Self::Xbgr8888,
            DRM_FORMAT_RGB565 => Self::Rgb565,
            other => Self::Unknown(other),
        }
    }

    // The wl_shm format enum value used to create a wl_buffer of this format, if supported.
    pub fn to_wl_shm_format(self) -> Option<WlShmFormat> {
        match self {
            Self::Argb8888 => Some(WlShmFormat::Argb8888),
            Self::Xrgb8888 => Some(WlShmFormat::Xrgb8888),
            Self::Abgr8888 => Some(WlShmFormat::Abgr8888),
            Self::Xbgr8888 => Some(WlShmFormat::Xbgr8888),
            Self::Rgb565 => Some(WlShmFormat::Rgb565),
            Self::Invalid | Self::Unknown(_) => None,
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
        const MAX_BUFFER_SIZE: u64 = 4 * 1024 * 1024 * 1024; // 4 GiB
        let size = (stride as u64).checked_mul(height as u64)?;
        if size > MAX_BUFFER_SIZE {
            None
        } else {
            usize::try_from(size).ok()
        }
    }

    /// Slice the pixel buffer data into rows based on the stride.
    pub fn row(&self, y: u32) -> Option<&[u8]> {
        let start = y.checked_mul(self.stride)? as usize;
        let end = start.checked_add(self.stride as usize)?;
        self.data.get(start..end)
    }

    /// Decode a single pixel at (x, y) into RGBA8 without materializing the whole frame.
    pub fn pixel_rgba8(&self, x: u32, y: u32) -> Result<[u8; 4], CaptureError> {
        let bpp = self
            .format
            .bytes_per_pixel()
            .ok_or(CaptureError::UnsupportedFormat(self.format))?;
        let row = self
            .row(y)
            .ok_or_else(|| CaptureError::CaptureFailed("buffer shorter than height".into()))?;
        let start = x as usize * bpp;
        let px = row
            .get(start..start + bpp)
            .ok_or_else(|| CaptureError::CaptureFailed("row shorter than width".into()))?;
        decode_pixel(px, self.format)
    }

    /// Convert the captured buffer into a tightly-packed RGBA8 image (width * height * 4 bytes),
    /// performing any channel swizzle required by the source format. Fully-opaque alpha (0xFF) is
    /// substituted for the padding byte of the X-formats.
    pub fn to_rgba8(&self) -> Result<Vec<u8>, CaptureError> {
        let bpp = self
            .format
            .bytes_per_pixel()
            .ok_or(CaptureError::UnsupportedFormat(self.format))?;

        let width = self.width as usize;
        let height = self.height as usize;
        let mut out = Vec::with_capacity(width * height * 4);

        for y in 0..self.height {
            let row = self
                .row(y)
                .ok_or_else(|| CaptureError::CaptureFailed("buffer shorter than height".into()))?;
            for x in 0..width {
                let px = row
                    .get(x * bpp..x * bpp + bpp)
                    .ok_or_else(|| CaptureError::CaptureFailed("row shorter than width".into()))?;
                let [r, g, b, a] = decode_pixel(px, self.format)?;
                out.extend_from_slice(&[r, g, b, a]);
            }
        }

        Ok(out)
    }

    /// Encode the captured buffer as a PNG image, returning the encoded bytes.
    pub fn encode_png(&self) -> Result<Vec<u8>, CaptureError> {
        let rgba = self.to_rgba8()?;

        let mut buf = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut buf, self.width, self.height);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder
                .write_header()
                .map_err(|e| CaptureError::EncodingFailed(e.to_string()))?;
            writer
                .write_image_data(&rgba)
                .map_err(|e| CaptureError::EncodingFailed(e.to_string()))?;
        }

        Ok(buf)
    }
}

// Decode a single pixel of the given format into RGBA channel order. wl_shm/DRM format names
// describe the 32-bit little-endian word, so the in-memory byte order is reversed:
// ARGB8888 -> [B, G, R, A], ABGR8888 -> [R, G, B, A].
fn decode_pixel(px: &[u8], format: PixelFormat) -> Result<[u8; 4], CaptureError> {
    let rgba = match format {
        PixelFormat::Argb8888 => [px[2], px[1], px[0], px[3]],
        PixelFormat::Xrgb8888 => [px[2], px[1], px[0], 0xFF],
        PixelFormat::Abgr8888 => [px[0], px[1], px[2], px[3]],
        PixelFormat::Xbgr8888 => [px[0], px[1], px[2], 0xFF],
        PixelFormat::Rgb565 => {
            let v = u16::from_le_bytes([px[0], px[1]]);
            let r = ((v >> 11) & 0x1F) as u8;
            let g = ((v >> 5) & 0x3F) as u8;
            let b = (v & 0x1F) as u8;
            // Scale 5/6-bit channels up to 8-bit.
            [
                (r << 3) | (r >> 2),
                (g << 2) | (g >> 4),
                (b << 3) | (b >> 2),
                0xFF,
            ]
        }
        PixelFormat::Invalid | PixelFormat::Unknown(_) => {
            return Err(CaptureError::UnsupportedFormat(format));
        }
    };
    Ok(rgba)
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
