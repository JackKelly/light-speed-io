use std::{
    env::temp_dir,
    fs::File,
    io::Write,
    path::{Path, PathBuf},
};

use clap::{error::ErrorKind, CommandFactory, Parser};

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
}

fn main() -> std::io::Result<()> {
    let mut args = Args::parse();

    check_directory_or_use_temp_dir(&mut args.directory);

    create_files(
        args.directory.as_ref().unwrap(),
        args.nrfiles,
        args.filesize,
    )?;

    Ok(())
}

fn check_directory_or_use_temp_dir(directory: &mut Option<PathBuf>) {
    // Check directory exists. Or use temp_dir.
    if let Some(directory) = directory.as_deref() {
        if !directory.is_dir() {
            let mut cmd = Args::command();
            cmd.error(
                ErrorKind::ValueValidation,
                format!("Directory {directory:?} does not exist, or is not a directory"),
            )
            .exit();
        }
    } else {
        *directory = Some(temp_dir());
    }
}

fn create_files(directory: &Path, nrfiles: u32, filesize: u64) -> std::io::Result<()> {
    let mut file_contents: Option<Vec<u8>> = None;
    for file_i in 0..nrfiles {
        let filename = directory.join(format!("lsio_bench_{file_i}"));
        if !(filename.exists() && File::open(&filename)?.metadata()?.len() == filesize) {
            if file_contents.is_none() {
                file_contents = Some((0..filesize).map(|i| i as u8).collect());
            }
            let mut file = File::create(&filename)?;
            file.write_all(file_contents.as_ref().unwrap())?;
            file.flush()?;
        }
    }
    Ok(())
}
