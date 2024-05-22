use std::path::PathBuf;

use clap::Parser;

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    /// Prefix filenames with this directory. If not set, will default to the system's temporary
    /// directory.
    #[arg(short, long)]
    directory: Option<PathBuf>,

    /// The number of files to read from for this benchmark.
    #[arg(short, long, default_value_t = 1)]
    nrfiles: u32,

    /// The size of each file, in bytes
    #[arg(short, long, default_value_t = 1024 * 1024)]
    filesize: u64,

    /// The chunk size in bytes. By default, the blocksize will be the same as the filesize.
    #[arg(short, long)]
    blocksize: Option<u64>,
}

fn main() {
    let args = Args::parse();
}
