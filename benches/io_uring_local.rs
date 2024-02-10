use criterion::{criterion_group, criterion_main, Criterion};
use light_speed_io::object_store_adapter::ObjectStoreAdapter;
use object_store::path::Path as ObjectStorePath;
use std::process::Command;
use tokio::runtime::Runtime;

const FILE_SIZE_BYTES: usize = 262_144;
const DATA_PATH: &str = "/home/jack/temp/fio/";

async fn load_files_with_io_uring_local(filenames: &Vec<ObjectStorePath>) {
    let n = filenames.len();

    // Start reading async:
    let store = ObjectStoreAdapter::default();
    let mut futures = Vec::with_capacity(n);
    for filename in filenames {
        futures.push(store.get(filename));
    }

    // Wait for everything to complete:
    let mut results = Vec::with_capacity(n);
    for f in futures {
        let b = f.await.expect("At least one Result was an Error");
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

    let filenames = get_filenames(N_FILES);

    // Run function:
    group.bench_function("io_uring_local", |b| {
        // Insert a call to `to_async` to convert the bencher to async mode.
        // The timing loops are the same as with the normal bencher.
        b.to_async(Runtime::new().unwrap()).iter_batched(
            || {
                clear_page_cache();
                &filenames
            },
            |filenames| async { load_files_with_io_uring_local(filenames).await },
            criterion::BatchSize::SmallInput,
        );
    });
    group.finish();
}

criterion_group!(benches, bench);
criterion_main!(benches);

fn clear_page_cache() {
    let _ = Command::new("vmtouch")
        .arg("-e")
        .arg(DATA_PATH)
        .output()
        .expect("vmtouch failed to start");
}

fn get_filenames(n: usize) -> Vec<ObjectStorePath> {
    // Create a vector of filenames (files created by `fio`)
    (0..n)
        .map(|i| ObjectStorePath::from(format!("//{DATA_PATH}reader1.0.{i}")))
        .collect()
}
