//! Stream copy from a [`Read`]er into a [`Write`]r.

use crate::io::{Read, Result, Write};

const STACK_BUF_SIZE: usize = 8192;

/// Copies the entire contents of a reader into a writer.
///
/// Counterpart to [`std::io::copy`]. This function continuously reads data
/// from `reader` and streams it into `writer` until EOF. All instances of
/// [`ErrorKind::Interrupted`](crate::io::ErrorKind::Interrupted) are
/// automatically retried.
///
/// On success, returns the total number of bytes copied.
///
/// If you want to copy the contents of one file to another and you are
/// working with filesystem paths, see [`crate::fs::copy`].
pub fn copy<R: Read + ?Sized, W: Write + ?Sized>(reader: &mut R, writer: &mut W) -> Result<u64> {
    let mut buf = [0u8; STACK_BUF_SIZE];
    let mut total: u64 = 0;

    loop {
        let n = match reader.read(&mut buf) {
            Ok(0) => return Ok(total),
            Ok(n) => n,
            Err(ref e) if e.kind() == crate::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };
        total += n as u64;
        writer.write_all(&buf[..n])?;
    }
}
