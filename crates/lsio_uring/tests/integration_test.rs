use crossbeam_channel::RecvTimeoutError;
use lsio_aligned_bytes::AlignedBytes;
use lsio_io::{Completion, Reader};
use lsio_uring::IoUring;
use rand::Rng;
use std::fs::File;
use std::io::Read;
use std::{io::Write, time::Duration};

const KIBIBYTE: usize = 1024;
const MEBIBYTE: usize = KIBIBYTE * 1024;

#[test]
fn test_get_ranges() -> anyhow::Result<()> {
    const N_WORKER_THREADS: usize = 4;
    const FILE_SIZE: usize = MEBIBYTE;
    const CHUNK_SIZE: usize = KIBIBYTE * 4;
    const N_CHUNKS: usize = FILE_SIZE / CHUNK_SIZE;

    // Create random ASCII text (that we will write to disk later):
    println!("Creating random data...");
    let distr = rand::distributions::Uniform::new_inclusive(32, 126);
    let file_contents: Vec<u8> = rand::thread_rng()
        .sample_iter(distr)
        .take(((CHUNK_SIZE as f32) * 1.5) as _)
        .collect::<Vec<u8>>()
        .into_iter()
        .cycle()
        .take(FILE_SIZE)
        .collect();
    assert_eq!(file_contents.len(), FILE_SIZE);

    // Create filename in temporary directory:
    let filename =
        std::env::temp_dir().join(format!("lsio_uring_tempfile_{}", rand::random::<u32>()));

    // Write file:
    println!("Writing random data to disk...");
    {
        let mut file = File::create(&filename)?;
        file.write_all(&file_contents)?;
        file.flush()?;
        file.sync_all()?;
    }

    // Check file is correctly written to disk:
    {
        let mut file = File::open(&filename)?;
        let mut temp_buffer = Vec::with_capacity(FILE_SIZE);
        file.read_to_end(&mut temp_buffer)?;
        assert!(temp_buffer.eq(&file_contents));
        assert_eq!(temp_buffer.len(), FILE_SIZE);
    }

    // Define byte ranges to load:
    let ranges = (0..N_CHUNKS)
        .map(|chunk_i| {
            let chunk_start = (chunk_i * CHUNK_SIZE) as isize;
            let chunk_end = chunk_start + (CHUNK_SIZE as isize);
            chunk_start..chunk_end
        })
        .collect();

    // Define user_data (so we can identify the chunks!)
    let user_data = (0..N_CHUNKS as u64).collect();

    // Submit get_ranges operation:
    println!("Reading data using io_uring!!!");
    let mut uring = IoUring::new(N_WORKER_THREADS);
    uring.get_ranges(&filename, ranges, user_data)?;

    // Re-assemble byte ranges:
    let mut vec_of_aligned_bytes: Vec<Option<AlignedBytes>> = (0..N_CHUNKS).map(|_| None).collect();

    for i in 0..N_CHUNKS {
        match uring.completion().recv_timeout(Duration::from_millis(500)) {
            Ok(output) => match output {
                Ok(c) => {
                    let lsio_io::Output::Chunk(c) = c;
                    vec_of_aligned_bytes[c.user_data as usize] = Some(c.buffer);
                }
                Err(e) => panic!("Error reading chunk {i}! {e:?}"),
            },
            Err(RecvTimeoutError::Timeout) => panic!("Timed out waiting for chunk {i}!"),
            Err(RecvTimeoutError::Disconnected) => {
                panic!("Disconnected whilst waiting for chunk {i}!")
            }
        };
    }
    println!("Finished reading using io_uring!");

    // Check that the completion queue does the right thing when IoUring is dropped:
    let completion = uring.completion().clone();
    drop(uring);
    assert!(completion.recv().is_err());
    drop(completion);

    // Re-assemble the chunks into the complete file:
    println!("Assembling buffer:");
    let mut assembled_buf = Vec::with_capacity(FILE_SIZE);
    for aligned_bytes in vec_of_aligned_bytes {
        assembled_buf.extend_from_slice(aligned_bytes.unwrap().as_slice());
    }

    println!(
        "Read from disk: {:?}",
        core::str::from_utf8(&assembled_buf[0..100]).unwrap()
    );
    println!(
        "Ground truth  : {:?}",
        core::str::from_utf8(&file_contents[0..100]).unwrap()
    );

    assert!(assembled_buf.eq(&file_contents));

    // Clean up:
    std::fs::remove_file(&filename)?;

    Ok(())
}
