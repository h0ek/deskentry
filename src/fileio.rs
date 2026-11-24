use std::fs::{self, File, OpenOptions};
use std::io;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

pub fn open_regular(path: &Path, nofollow: bool) -> io::Result<File> {
    let metadata = if nofollow {
        fs::symlink_metadata(path)?
    } else {
        fs::metadata(path)?
    };
    if !metadata.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Not a regular file",
        ));
    }
    let flags = libc::O_NONBLOCK | if nofollow { libc::O_NOFOLLOW } else { 0 };
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(flags)
        .open(path)?;
    if !file.metadata()?.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Not a regular file",
        ));
    }
    Ok(file)
}
