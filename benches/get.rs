use criterion::{criterion_group, criterion_main, Criterion};
use light_speed_io::object_store_adapter::ObjectStoreAdapter;
use object_store::{path::Path as ObjectStorePath, ObjectStore};
use std::{
    ops::Range,
    process::Command,
    time::{Duration, Instant},
};
use tokio::runtime::Runtime;

const FILE_SIZE_BYTES: usize = 262_144;
const DATA_PATH: &str = "/tmp/fio/";
const RANGE: Range<isize> = 0..(1024 * 16);

async fn uring_get(filenames: &Vec<ObjectStorePath>, n_iterations: u64) -> Duration {
    let mut total_time = Duration::ZERO;
    for _ in 0..n_iterations {
        // Setup (not timed):
        let store = ObjectStoreAdapter::default();
        clear_page_cache();
        let mut futures = Vec::with_capacity(filenames.len());

        // Timed code:
        let start_of_iter = Instant::now();
        for filename in filenames {
            futures.push(store.get(filename));
        }
        for f in futures {
            let b = f.await.expect("At least one Result was an Error");
            assert_eq!(b.len(), FILE_SIZE_BYTES);
        }
        total_time += start_of_iter.elapsed();
    }
    total_time
}

async fn uring_get_range(filenames: &Vec<ObjectStorePath>, n_iterations: u64) -> Duration {
    let mut total_time = Duration::ZERO;
    for _ in 0..n_iterations {
        // Setup (not timed):
        let store = ObjectStoreAdapter::default();
        clear_page_cache();
        let mut futures = Vec::with_capacity(filenames.len());

        // Timed code:
        let start_of_iter = Instant::now();
        for filename in filenames {
            futures.push(store.get_range(filename, RANGE));
        }
        for f in futures {
            let b = f.await.expect("At least one Result was an Error");
            assert_eq!(b.len(), RANGE.len());
        }
        total_time += start_of_iter.elapsed();
    }
    total_time
}

async fn local_file_system_get(filenames: &Vec<ObjectStorePath>, n_iterations: u64) -> Duration {
    // Unfortunately, I can't find a better way to share code between `load_files_with_io_uring_local`
    // and `load_files_with_local_file_system` because `ObjectStoreAdapter` doesn't yet `impl ObjectStore`.
    // And `ObjectStoreAdapter::get` and `LocalFileSystem::get` return slightly different types.
    // TODO: Reduce duplication if/when `ObjectStoreAdapter` implements `ObjectStore`.

    let mut total_time = Duration::ZERO;
    for _ in 0..n_iterations {
        // Setup (not timed):
        clear_page_cache();
        let mut handles = Vec::with_capacity(filenames.len());

        // Timed code:
        let start_of_iter = Instant::now();
        for filename in filenames {
            let filename = filename.clone();
            handles.push(tokio::spawn(async move {
                // We can't create the `store` outside of `spawn` and move it into `spawn`.
                // So we have to create the `store` _inside_ this `async` block.
                let store = object_store::local::LocalFileSystem::default();
                let result = store.get(&filename).await.unwrap();
                result.bytes().await
            }));
        }

        for h in handles {
            let bytes = h.await.unwrap().unwrap();
            assert_eq!(bytes.len(), FILE_SIZE_BYTES);
        }

        total_time += start_of_iter.elapsed();
    }
    total_time
}

async fn local_file_system_get_range(
    filenames: &Vec<ObjectStorePath>,
    n_iterations: u64,
) -> Duration {
    // Unfortunately, I can't find a better way to share code between `load_files_with_io_uring_local`
    // and `load_files_with_local_file_system` because `ObjectStoreAdapter` doesn't yet `impl ObjectStore`.
    // And `ObjectStoreAdapter::get` and `LocalFileSystem::get` return slightly different types.
    // TODO: Reduce duplication if/when `ObjectStoreAdapter` implements `ObjectStore`.

    const RANGE_USIZE: Range<usize> = Range {
        start: RANGE.start as usize,
        end: RANGE.end as usize,
    };

    let mut total_time = Duration::ZERO;
    for _ in 0..n_iterations {
        // Setup (not timed):
        clear_page_cache();
        let mut handles = Vec::with_capacity(filenames.len());

        // Timed code:
        let start_of_iter = Instant::now();
        for filename in filenames {
            let filename = filename.clone();
            handles.push(tokio::spawn(async move {
                // We can't create the `store` outside of `spawn` and move it into `spawn`.
                // So we have to create the `store` _inside_ this `async` block.
                let store = object_store::local::LocalFileSystem::default();
                store.get_range(&filename, RANGE_USIZE).await.unwrap()
            }));
        }

        for h in handles {
            let bytes = h.await.unwrap();
            assert_eq!(bytes.len(), RANGE.len());
        }

        total_time += start_of_iter.elapsed();
    }
    total_time
}

fn bench_get(c: &mut Criterion) {
    const N_FILES: usize = 1000;

    // Configure group:
    let mut group = c.benchmark_group(format!("get_{N_FILES}_whole_files"));
    group.sample_size(10);
    group.warm_up_time(Duration::from_millis(2000));
    group.throughput(criterion::Throughput::Bytes(
        (FILE_SIZE_BYTES * N_FILES) as u64,
    ));

    let filenames = get_filenames(N_FILES);

    // Run function:
    group.bench_function("uring_get", |b| {
        // Insert a call to `to_async` to convert the bencher to async mode.
        // The timing loops are the same as with the normal bencher.
        b.to_async(Runtime::new().unwrap())
            .iter_custom(|n_iterations| uring_get(&filenames, n_iterations));
    });

    // Run function:
    group.bench_function("local_file_system_get", |b| {
        // Insert a call to `to_async` to convert the bencher to async mode.
        // The timing loops are the same as with the normal bencher.
        b.to_async(Runtime::new().unwrap())
            .iter_custom(|n_iterations| local_file_system_get(&filenames, n_iterations));
    });

    group.finish();
}

fn bench_get_range(c: &mut Criterion) {
    const N_FILES: usize = 1000;

    // Configure group:
    let mut group = c.benchmark_group(format!("get_{}_bytes_from_{N_FILES}_files", RANGE.len()));
    group.sample_size(10);
    group.warm_up_time(Duration::from_millis(2000));
    group.throughput(criterion::Throughput::Bytes((RANGE.len() * N_FILES) as u64));

    let filenames = get_filenames(N_FILES);

    // Run function:
    group.bench_function("uring_get_range", |b| {
        // Insert a call to `to_async` to convert the bencher to async mode.
        // The timing loops are the same as with the normal bencher.
        b.to_async(Runtime::new().unwrap())
            .iter_custom(|n_iterations| uring_get_range(&filenames, n_iterations));
    });

    // Run function:
    group.bench_function("local_file_system_get_range", |b| {
        // Insert a call to `to_async` to convert the bencher to async mode.
        // The timing loops are the same as with the normal bencher.
        b.to_async(Runtime::new().unwrap())
            .iter_custom(|n_iterations| local_file_system_get_range(&filenames, n_iterations));
    });

    group.finish();
}

criterion_group!(benches, bench_get, bench_get_range);
criterion_main!(benches);

fn clear_page_cache() {
    let _ = Command::new("vmtouch")
        .arg("-e")
        .arg(DATA_PATH)
        .output()
        .expect("vmtouch failed to start");

    // let _ = Command::new("sudo")
    //     .arg("sysctl")
    //     .arg("-w")
    //     .arg("vm.drop_caches=3")
    //     .output()
    //     .expect("sudo sysctl failed to start");
}

fn get_filenames(n: usize) -> Vec<ObjectStorePath> {
    // Create a vector of filenames (files created by `fio`)
    (0..n)
        .map(|i| {
            ObjectStorePath::from(format!(
                "//{DATA_PATH}sequential_read_1000_files_each_256KiB.0.{i}"
            ))
        })
        .collect()
}
