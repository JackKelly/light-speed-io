use std::{
    env::temp_dir,
    fs::File,
    io::Write,
    ops::Range,
    path::{Path, PathBuf},
};

use clap::{error::ErrorKind, CommandFactory, Parser};
use indicatif::{ProgressBar, ProgressStyle};
use lsio_io::{Completion, Reader};
use lsio_uring::IoUring;

const FILENAME_PREFIX: &str = "lsio_bench_";

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    /// Prefix filenames with this directory. If not set, will default to the system's temporary
    /// directory. This directory must already exist.
    #[arg(short, long)]
    directory: Option<PathBuf>,

    /// The number of files to read from for this benchmark.
    #[arg(short, long, default_value_t = 1, value_parser = clap::value_parser!(u32).range(1..))]
    nrfiles: u32,

    /// The size of each file, in bytes
    #[arg(short, long, default_value_t = 1024 * 1024, value_parser = clap::value_parser!(u64).range(1..))]
    filesize: u64,

    /// The chunk size in bytes. By default, the blocksize will be the same as the filesize.
    #[arg(short, long, value_parser = clap::value_parser!(u64).range(1..))]
    blocksize: Option<u64>,

    /// The number of worker threads that lsio_uring uses:
    #[arg(short, long, default_value_t = 4, value_parser = clap::value_parser!(usize).range(1..1024))]
    nr_worker_threads: usize,
}

fn main() -> std::io::Result<()> {
    let args = Args::parse();

    let directory = check_directory_or_use_temp_dir(&args.directory);

    let filenames: Vec<PathBuf> = (0..args.nrfiles)
        .map(|i| directory.join(format!("{FILENAME_PREFIX}{i}")))
        .collect();

    create_files_if_necessary(&filenames, args.filesize)?;

    read_files(
        &filenames,
        args.filesize,
        args.blocksize,
        args.nr_worker_threads,
    );

    Ok(())
}

fn check_directory_or_use_temp_dir(directory: &Option<PathBuf>) -> PathBuf {
    // Check directory exists. Or use temp_dir.
    if let Some(directory) = directory.as_deref() {
        if directory.is_dir() {
            directory.to_path_buf()
        } else {
            let mut cmd = Args::command();
            cmd.error(
                ErrorKind::ValueValidation,
                format!("Directory {directory:?} does not exist, or is not a directory"),
            )
            .exit();
        }
    } else {
        temp_dir()
    }
}

fn create_files_if_necessary(filenames: &[PathBuf], filesize: u64) -> std::io::Result<()> {
    // Create progress bar:
    println!(
        "Creating {} files (if necessary), each of filesize {filesize} bytes...",
        filenames.len()
    );
    let pb = ProgressBar::new(filenames.len() as _);
    pb.set_style(get_progress_bar_style());

    // Loop through files:
    let mut file_contents: Option<Vec<u8>> = None;
    for filename in filenames {
        if filename.exists() && get_filesize(&filename)? == filesize {
            pb.set_message(format!("exists: {filename:?}"));
        } else {
            pb.set_message(format!("creating: {filename:?}"));
            if file_contents.is_none() {
                file_contents = Some((0..filesize).map(|i| i as u8).collect());
            }
            let mut file = File::create(&filename)?;
            file.write_all(file_contents.as_ref().unwrap())?;
            file.flush()?;
        }
        pb.inc(1);
    }
    pb.finish_with_message("done");
    Ok(())
}

fn get_filesize(filename: &Path) -> std::io::Result<u64> {
    Ok(File::open(&filename)?.metadata()?.len())
}

fn get_progress_bar_style() -> ProgressStyle {
    ProgressStyle::with_template("[{elapsed_precise}] {bar:40.cyan/blue} {pos:>7}/{len:7} {msg}")
        .unwrap()
        .progress_chars("##-")
}

fn read_files(
    filenames: &[PathBuf],
    filesize: u64,
    blocksize: Option<u64>,
    n_worker_threads: usize,
) {
    let blocksize = if let Some(bs) = blocksize {
        bs
    } else {
        filesize
    };

    // Calculate chunks
    let n_chunks = filesize / blocksize;
    let chunks: Vec<Range<isize>> = (0..n_chunks)
        .map(|chunk_i| {
            let chunk_start = (chunk_i * blocksize) as isize;
            let chunk_end = chunk_start + (blocksize as isize);
            chunk_start..chunk_end
        })
        .collect();

    // Define user_data (so we can identify the chunks!)
    let user_data: Vec<u64> = (0..n_chunks as u64).collect();

    let mut uring = IoUring::new(n_worker_threads);

    // Submit all the get_ranges requests:
    for filename in filenames {
        uring.get_ranges(&filename, chunks.clone(), user_data.clone());
    }

    // TODO: Collect results
}
