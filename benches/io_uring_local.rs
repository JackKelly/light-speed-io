use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use light_speed_io::object_store_adapter::ObjectStoreAdapter;
use object_store::path::Path;
use std::process::Command;
use tokio::runtime::Runtime;

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
        assert!(b.len() == 262144);
        results.push(b);
    }
}

fn criterion_benchmark(c: &mut Criterion) {
    let size: usize = 1;
    c.bench_with_input(
        BenchmarkId::new("load 1000 files using io_uring_local", size),
        &size,
        |b, &s| {
            // Insert a call to `to_async` to convert the bencher to async mode.
            // The timing loops are the same as with the normal bencher.
            b.to_async(Runtime::new().unwrap())
                .iter(|| load_files_with_io_uring_local(s));
        },
    );
}

criterion_group!(benches, criterion_benchmark);
criterion_main!(benches);
