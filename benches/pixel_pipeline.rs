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

//! Benchmarks for the in-process pixel pipeline: format conversion, PNG encoding, and the
//! full-frame copy that capture backends used to pay on every screenshot before `PixelData`
//! let them read straight out of the compositor's shm mapping instead.
//!
//! These are deliberately compositor-free - no Wayland connection, no live protocol state —
//! so they run the same way in CI as on a dev machine and stay reproducible as the code
//! changes. End-to-end (D-Bus call to file on disk) and cross-portal numbers need a live
//! compositor and are measured separately.

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use wayland_capture::{PixelBuffer, PixelData, PixelFormat};

/// Common capture resolutions: a small embedded panel and a desktop-class 1080p output.
const SIZES: &[(u32, u32)] = &[(800, 480), (1920, 1080)];

fn synthetic_frame(width: u32, height: u32, format: PixelFormat) -> PixelBuffer {
    let bpp = format
        .bytes_per_pixel()
        .expect("benchmarked formats have known bpp");
    let stride = width * bpp as u32;
    let size = PixelBuffer::expected_size(stride, height).expect("size fits usize");
    // Content doesn't matter for timing; a repeating pattern avoids degenerate all-zero pages.
    let data: Vec<u8> = (0..size).map(|i| (i % 251) as u8).collect();
    PixelBuffer {
        data: PixelData::Owned(data),
        width,
        height,
        stride,
        format,
    }
}

fn bench_eliminated_copy(c: &mut Criterion) {
    let mut group = c.benchmark_group("eliminated_mmap_copy");
    for &(w, h) in SIZES {
        let size = (w * h * 4) as usize; // worst case: 4-byte formats
        let src = vec![0xABu8; size];
        group.throughput(Throughput::Bytes(size as u64));
        group.bench_with_input(
            BenchmarkId::new("to_vec", format!("{w}x{h}")),
            &src,
            |b, src| {
                b.iter(|| std::hint::black_box(src.as_slice()).to_vec());
            },
        );
    }
    group.finish();
}

fn bench_to_rgba8(c: &mut Criterion) {
    let mut group = c.benchmark_group("to_rgba8");
    for &(w, h) in SIZES {
        for format in [
            PixelFormat::Argb8888,
            PixelFormat::Xrgb8888,
            PixelFormat::Rgb565,
        ] {
            let frame = synthetic_frame(w, h, format);
            let bytes = frame.data.len() as u64;
            group.throughput(Throughput::Bytes(bytes));
            group.bench_with_input(
                BenchmarkId::new(format!("{format:?}"), format!("{w}x{h}")),
                &frame,
                |b, frame| b.iter(|| std::hint::black_box(frame).to_rgba8().unwrap()),
            );
        }
    }
    group.finish();
}

fn bench_encode_png(c: &mut Criterion) {
    let mut group = c.benchmark_group("encode_png");
    for &(w, h) in SIZES {
        let frame = synthetic_frame(w, h, PixelFormat::Argb8888);
        group.throughput(Throughput::Bytes(frame.data.len() as u64));
        group.bench_with_input(
            BenchmarkId::from_parameter(format!("{w}x{h}")),
            &frame,
            |b, frame| b.iter(|| std::hint::black_box(frame).encode_png().unwrap()),
        );
    }
    group.finish();
}

criterion_group!(
    benches,
    bench_eliminated_copy,
    bench_to_rgba8,
    bench_encode_png
);
criterion_main!(benches);
