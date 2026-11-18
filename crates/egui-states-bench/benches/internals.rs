//! Benchmarks of internal hot paths, reached through `egui_states::bench_support`.
//!
//! - `pack_data_set` / `pack_image_set`: building the messages of one transfer.
//! - `server_image_set` / `server_image_update` / `server_image_widen`: what
//!   the server does to its stored image for a set, for an update in a format
//!   the image holds, and for one that widens it first.
//! - `client_color_image`: pixel conversion on the client.
//! - `client_data_assembly`: publishing a chunked Data transfer on the client.

use std::hint::black_box;
use std::time::Duration;

use bytes::Bytes;
use criterion::{BatchSize, BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use egui_states::bench_support::{self, ClientData, PixelFormat, ServerImage};
use egui_states_bench::{MIB, pattern};

const FORMATS: [PixelFormat; 4] = [
    PixelFormat::Gray,
    PixelFormat::GrayAlpha,
    PixelFormat::Color,
    PixelFormat::ColorAlpha,
];

fn packers(c: &mut Criterion) {
    let mut group = c.benchmark_group("pack_data_set");
    group.measurement_time(Duration::from_secs(5));
    for mib in [1, 10, 50] {
        let data = pattern(mib * MIB);
        group.throughput(Throughput::Bytes(data.len() as u64));
        group.bench_function(BenchmarkId::from_parameter(format!("{mib}MiB")), |b| {
            b.iter(|| bench_support::pack_data_set(black_box(&data)))
        });
    }
    group.finish();

    let mut group = c.benchmark_group("pack_image_set");
    group.measurement_time(Duration::from_secs(5));
    for mib in [1, 10] {
        let shape = [mib * 256, 1024];
        let pixels = pattern(shape[0] * shape[1] * 4);
        group.throughput(Throughput::Bytes(pixels.len() as u64));
        group.bench_function(BenchmarkId::from_parameter(format!("{mib}MiB")), |b| {
            b.iter(|| {
                bench_support::pack_image_set(black_box(&pixels), shape, PixelFormat::ColorAlpha)
            })
        });
    }
    group.finish();
}

fn pixels(c: &mut Criterion) {
    // 2 megapixels per conversion.
    let shape = [1024, 2048];
    let count = shape[0] * shape[1];

    // A set stores the pixels in their own format.
    let mut group = c.benchmark_group("server_image_set");
    group.throughput(Throughput::Elements(count as u64));
    for format in FORMATS {
        let source = pattern(count * format.bytes_per_pixel());
        let mut image = ServerImage::new(&source, shape, format);
        group.bench_function(BenchmarkId::from_parameter(format!("{format:?}")), |b| {
            b.iter(|| image.set(black_box(&source), shape, format))
        });
    }
    group.finish();

    // An update of the top half of an RGBA image converts while it writes.
    let half = [shape[0] / 2, shape[1]];
    let half_count = half[0] * half[1];
    let mut group = c.benchmark_group("server_image_update");
    group.throughput(Throughput::Elements(half_count as u64));
    for format in FORMATS {
        let source = pattern(half_count * format.bytes_per_pixel());
        let mut image = ServerImage::new(&pattern(count * 4), shape, PixelFormat::ColorAlpha);
        group.bench_function(BenchmarkId::from_parameter(format!("{format:?}")), |b| {
            b.iter(|| image.update(black_box(&source), half, format))
        });
    }
    group.finish();

    // An RGBA update of one pixel widens the whole image first.
    let mut group = c.benchmark_group("server_image_widen");
    group.throughput(Throughput::Elements(count as u64));
    for format in [
        PixelFormat::Gray,
        PixelFormat::GrayAlpha,
        PixelFormat::Color,
    ] {
        let source = pattern(count * format.bytes_per_pixel());
        group.bench_function(BenchmarkId::from_parameter(format!("{format:?}")), |b| {
            b.iter_batched_ref(
                || ServerImage::new(&source, shape, format),
                |image| image.update(black_box(&[1, 2, 3, 4]), [1, 1], PixelFormat::ColorAlpha),
                BatchSize::LargeInput,
            )
        });
    }
    group.finish();

    let mut group = c.benchmark_group("client_color_image");
    group.throughput(Throughput::Elements(count as u64));
    for format in FORMATS {
        let source = pattern(count * format.bytes_per_pixel());
        group.bench_function(BenchmarkId::from_parameter(format!("{format:?}")), |b| {
            b.iter(|| bench_support::client_color_image(black_box(&source), shape, format))
        });
    }
    group.finish();
}

fn assembly(c: &mut Criterion) {
    let mut group = c.benchmark_group("client_data_assembly");
    group.measurement_time(Duration::from_secs(5));
    let mut client = ClientData::new();
    for (mib, chunk_mib) in [(10, 10), (10, 1), (50, 10), (50, 1)] {
        let data = Bytes::from(pattern(mib * MIB));
        let chunks: Vec<Bytes> = (0..data.len())
            .step_by(chunk_mib * MIB)
            .map(|start| data.slice(start..(start + chunk_mib * MIB).min(data.len())))
            .collect();
        group.throughput(Throughput::Bytes(data.len() as u64));
        group.bench_function(
            BenchmarkId::from_parameter(format!("{mib}MiB_in_{chunk_mib}MiB_chunks")),
            |b| b.iter(|| client.assemble(black_box(&chunks))),
        );
    }
    group.finish();
}

criterion_group!(benches, packers, pixels, assembly);
criterion_main!(benches);
