use criterion::{criterion_group, criterion_main, Criterion};
use light_speed_io::object_store_adapter::ObjectStoreAdapter;
use object_store::{path::Path as ObjectStorePath, ObjectStore};
use std::{
    process::Command,
    time::{Duration, Instant},
};
use tokio::runtime::Runtime;

const FILE_SIZE_BYTES: usize = 262_144;
const DATA_PATH: &str = "/home/jack/temp/fio/";

async fn load_files_with_io_uring_local(
    filenames: &Vec<ObjectStorePath>,
    n_iterations: u64,
) -> Duration {
    let mut total_time = Duration::ZERO;
    for _ in 0..n_iterations {
        // Setup (not timed):
        let store = ObjectStoreAdapter::default();
        clear_page_cache();
        let mut futures = Vec::with_capacity(filenames.len());
        let mut results = Vec::with_capacity(filenames.len());

        // Timed code:
        let start_of_iter = Instant::now();
        for filename in filenames {
            futures.push(store.get(filename));
        }
        for f in futures {
            let b = f.await.expect("At least one Result was an Error");
            assert_eq!(b.len(), FILE_SIZE_BYTES);
            results.push(b);
        }
        total_time += start_of_iter.elapsed();
    }
    total_time
}

async fn load_files_with_local_file_system(
    filenames: &Vec<ObjectStorePath>,
    n_iterations: u64,
) -> Duration {
    // Unfortunately, I can't find a better way to share code between `load_files_with_io_uring_local`
    // and `load_files_with_local_file_system` because `ObjectStoreAdapter` doesn't yet `impl ObjectStore`.
    // And `ObjectStoreAdapter::get` and `LocalFileSystem::get` return slightly different types.
    // TODO: Reduce duplication if/when `ObjectStoreAdapter` implements `ObjectStore`.

    let mut total_time = Duration::ZERO;
    for _ in 0..n_iterations {
        // Setup (not timed):
        let store = object_store::local::LocalFileSystem::default();
        clear_page_cache();
        let mut futures = Vec::with_capacity(filenames.len());
        let mut results = Vec::with_capacity(filenames.len());

        // Timed code:
        let start_of_iter = Instant::now();
        for filename in filenames {
            futures.push(store.get(filename));
        }

        for f in futures {
            let get_result = f.await.expect("At least one GetResult was an Error");
            results.push(get_result);
        }

        for get_result in results {
            let b = get_result.bytes().await.unwrap();
            assert!(b.len() == FILE_SIZE_BYTES);
        }
        total_time += start_of_iter.elapsed();
    }
    total_time
}

fn bench(c: &mut Criterion) {
    const N_FILES: usize = 1000;

    // Configure group:
    let mut group = c.benchmark_group(format!("load_{N_FILES}_files"));
    group.sample_size(10);
    group.warm_up_time(Duration::from_millis(2000));
    group.throughput(criterion::Throughput::Bytes(
        (FILE_SIZE_BYTES * N_FILES) as u64,
    ));

    let filenames = get_filenames(N_FILES);

    // Run function:
    group.bench_function("io_uring_local", |b| {
        // Insert a call to `to_async` to convert the bencher to async mode.
        // The timing loops are the same as with the normal bencher.
        b.to_async(Runtime::new().unwrap())
            .iter_custom(|n_iterations| load_files_with_io_uring_local(&filenames, n_iterations));
    });

    // Run function:
    group.bench_function("local_file_system", |b| {
        // Insert a call to `to_async` to convert the bencher to async mode.
        // The timing loops are the same as with the normal bencher.
        b.to_async(Runtime::new().unwrap())
            .iter_custom(|n_iterations| {
                load_files_with_local_file_system(&filenames, n_iterations)
            });
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
        .map(|i| ObjectStorePath::from(format!("//{DATA_PATH}reader1.0.{i}")))
        .collect()
}
