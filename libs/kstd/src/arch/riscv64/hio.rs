use core::fmt::{Error, Write};
use core::{fmt, slice};
use core::ops::DerefMut;
use crate::sync::Mutex;
use super::semihosting::syscall;

const OPEN: usize = 0x01;
const WRITE: usize = 0x05;
const OPEN_W_TRUNC: usize = 4;
const OPEN_W_APPEND: usize = 8;

pub struct HostStream(usize);

impl HostStream {
    pub fn new_stdout() -> Self {
        Self::open(":tt\0", OPEN_W_TRUNC).unwrap()
    }

    pub fn new_stderr() -> Self {
        Self::open(":tt\0", OPEN_W_APPEND).unwrap()
    }

    #[allow(clippy::result_unit_err)]
    pub fn write_all(&mut self, mut buf: &[u8]) -> Result<(), ()> {
        while !buf.is_empty() {
            match unsafe { syscall!(WRITE, self.0, buf.as_ptr(), buf.len()) } {
                // Done
                0 => return Ok(()),
                // `n` bytes were not written
                n if n <= buf.len() => {
                    let offset = (buf.len() - n) as isize;
                    buf = unsafe { slice::from_raw_parts(buf.as_ptr().offset(offset), n) }
                }
                // #[cfg(feature = "jlink-quirks")]
                // // Error (-1) - should be an error but JLink can return -1, -2, -3,...
                // // For good measure, we allow up to negative 15.
                // n if n > 0xfffffff0 => return Ok(()),
                // Error
                _ => return Err(()),
            }
        }

        Ok(())
    }

    #[allow(clippy::result_unit_err)]
    fn open(name: &str, mode: usize) -> Result<Self, ()> {
        let name = name.as_bytes();
        match unsafe { syscall!(OPEN, name.as_ptr() as usize, mode, name.len() - 1) } as isize {
            -1 => Err(()),
            fd => Ok(Self(fd as usize)),
        }
    }
}

impl Write for HostStream {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        #[allow(clippy::default_constructed_unit_structs)]
        self.write_all(s.as_bytes()).map_err(|_| Error::default())?;

        Ok(())
    }
}


static STDOUT: Mutex<Option<HostStream>> = Mutex::new(None);
static STDERR: Mutex<Option<HostStream>> = Mutex::new(None);

pub fn with_hstdout(f: impl FnOnce(&mut HostStream)) {
    let mut stream = STDOUT.lock();

    if stream.is_none() {
        stream.replace(HostStream::new_stdout());
    }

    match stream.deref_mut() {
        Some(stream) => f(stream),
        None => unreachable!(),
    }
}

pub fn with_hstderr(f: impl FnOnce(&mut HostStream)) {
    let mut stream = STDERR.lock();

    if stream.is_none() {
        stream.replace(HostStream::new_stderr());
    }

    match stream.deref_mut() {
        Some(stream) => f(stream),
        None => unreachable!(),
    }
}

pub fn _print(args: fmt::Arguments) {
    with_hstdout(|stdout| {
        stdout
            .write_fmt(args)
            .expect("failed to write to semihosting stdout")
    })
}

pub fn _eprint(args: fmt::Arguments) {
    with_hstderr(|stderr| {
        stderr
            .write_fmt(args)
            .expect("failed to write to semihosting stderr")
    })
}