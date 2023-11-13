use io_uring::{opcode, types, IoUring};
use io_uring::squeue::PushError;
use std::os::unix::io::AsRawFd;
use std::{fs, io};

fn io_uring() -> io::Result<()> {
    let mut ring = IoUring::builder()
        .build(8)?;

    let mut buf = vec![0; 1024];

    let fd = fs::File::open("README.md")?;

    submit_read(
        &mut ring,
        &fd,
        buf.as_mut_ptr(),
        buf.len() as _
    ).expect("submission queue full");

    ring.submit_and_wait(1)?;

    let cqe = ring.completion().next().expect("completion queue is empty");

    assert_eq!(cqe.user_data(), 0x42);
    assert!(cqe.result() >= 0, "read error: {}", cqe.result());

    Ok(())
}

/// Note that the developer needs to ensure
/// that the entry pushed into submission queue is valid (e.g. fd, buffer).
fn submit_read(ring: &mut IoUring, fd: &fs::File, buf: *mut u8, len: u32) -> Result<(), PushError>
{
    let read_e = opcode::Read::new(types::Fd(fd.as_raw_fd()), buf, len)
        .build()
        .user_data(0x42);

    unsafe {
        ring.submission()
            .push(&read_e)
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_works() {
        let result = io_uring();
        assert!(result.is_ok(), "Error: {}", result.unwrap_err());
    }
}