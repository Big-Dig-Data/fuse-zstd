use sled;
use std::io;

pub fn convert_io_error<E>(err: E) -> libc::c_int
where
    E: Into<io::Error>,
{
    let err: io::Error = err.into();
    err.raw_os_error().unwrap_or(libc::EIO)
}

pub fn convert_sled_error(err: sled::Error) -> libc::c_int {
    match err {
        sled::Error::Io(ioerror) => convert_io_error(ioerror),
        _ => libc::EIO,
    }
}
