use criterion::{black_box, criterion_group, criterion_main, Criterion};
use light_speed_io::object_store_adapter::ObjectStoreAdapter;
use object_store::path::Path;

async fn load_files_with_io_uring_local(n: usize) {
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
        assert!(r.is_ok());
        let b = r.unwrap();
        assert!(b.len() == 262144);
        results.push(b);
    }
}

fn criterion_benchmark(c: &mut Criterion) {
    c.bench_function("load 1000 files using io_uring_local", |b| {
        b.iter(|| load_files_with_io_uring_local(black_box(1000)))
    });
}

criterion_group!(benches, criterion_benchmark);
criterion_main!(benches);
