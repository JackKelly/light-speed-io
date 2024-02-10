use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use light_speed_io::object_store_adapter::ObjectStoreAdapter;
use object_store::path::Path;
use std::process::Command;
use tokio::runtime::Runtime;

const FILE_SIZE_BYTES: usize = 262_144;

async fn load_files_with_io_uring_local(n: usize) {
    // Clear page cache
    let _ = Command::new("vmtouch")
        .arg("-e")
        .arg("/home/jack/temp/fio/")
        .output()
        .expect("vmtouch failed");

    // Create a vector of filenames (files created by `fio`)
    let filenames: Vec<Path> = (0..n)
        .map(|i| Path::from(format!("///home/jack/temp/fio/reader1.0.{i}")))
        .collect();

    // Start reading async:
    let store = ObjectStoreAdapter::default();
    let mut futures = Vec::with_capacity(n);
    for filename in &filenames {
        futures.push(store.get(filename));
    }

    // Wait for everything to complete:
    let mut results = Vec::with_capacity(n);
    for f in futures {
        let r = f.await;
        let b = r.expect("At least one Result was an Error");
        assert!(b.len() == FILE_SIZE_BYTES);
        results.push(b);
    }
}

fn bench(c: &mut Criterion) {
    const N_FILES: usize = 1000;

    // Configure group:
    let mut group = c.benchmark_group(format!("Load {N_FILES} files"));
    group.sample_size(10);
    group.throughput(criterion::Throughput::Bytes(
        (FILE_SIZE_BYTES * N_FILES) as u64,
    ));

    // Run function:
    group.bench_function("io_uring_local", |b| {
        // Insert a call to `to_async` to convert the bencher to async mode.
        // The timing loops are the same as with the normal bencher.
        b.to_async(Runtime::new().unwrap())
            .iter(|| async { load_files_with_io_uring_local(N_FILES).await });
    });
    group.finish();
}

criterion_group!(benches, bench);
criterion_main!(benches);
