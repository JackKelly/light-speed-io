use crossbeam_channel::RecvTimeoutError;
use lsio_aligned_bytes::AlignedBytes;
use lsio_io::{Completion, Reader};
use lsio_uring::IoUring;
use rand::RngCore;
use std::fs::File;
use std::io::Read;
use std::{io::Write, time::Duration};

const KIBIBYTE: usize = 1024;
const MEBIBYTE: usize = KIBIBYTE * 1024;

#[test]
fn test_get_ranges() -> anyhow::Result<()> {
    const N_WORKER_THREADS: usize = 1;
    const FILE_SIZE: usize = MEBIBYTE;
    const CHUNK_SIZE: usize = KIBIBYTE * 4;
    const N_CHUNKS: usize = FILE_SIZE / CHUNK_SIZE;

    let mut uring = IoUring::new(N_WORKER_THREADS);

    // Automatically create file
    // TODO: Persist this file?
    let filename =
        std::env::temp_dir().join(format!("lsio_uring_tempfile_{}", rand::random::<u32>()));
    let mut file_contents: Vec<u8> = Vec::with_capacity(FILE_SIZE);
    rand::thread_rng().fill_bytes(&mut file_contents);
    unsafe {
        file_contents.set_len(FILE_SIZE);
    }
    // Prevent mutation of file_contents:
    let file_contents = file_contents;

    // Write file:
    {
        let mut file = File::create(&filename)?;
        file.write_all(&file_contents)?;
    }

    assert_eq!(file_contents.len(), FILE_SIZE);

    // Wait for data to be flushed to disk:
    std::thread::sleep(Duration::from_millis(500));

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

    let completion = uring.completion().clone();
    drop(uring);
    assert!(completion.recv().is_err());

    let mut assembled_buf = Vec::with_capacity(FILE_SIZE);
    for aligned_bytes in vec_of_aligned_bytes {
        assembled_buf.extend_from_slice(aligned_bytes.unwrap().as_slice());
    }

    println!("Read from disk: {:?}", &assembled_buf[0..10]);
    println!("Ground truth  : {:?}", &file_contents[0..10]);

    assert!(assembled_buf.eq(&file_contents));

    // Clean up:
    std::fs::remove_file(&filename)?;

    Ok(())
}
