//! `/dev/hidrawN` transport.

use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::fd::AsRawFd;
use std::path::Path;
use std::time::Duration;

use crate::hid::guard::ReadOnlyGuard;
use crate::hid::transport::Transport;
use crate::protocol::{REPORT_LEN, Report};

/// Deliberately has no public constructor: `open_guarded` is the only way to get one,
/// and it hands the transport straight to the read-only guard.
pub struct Hidraw {
    file: File,
}

pub fn open_guarded(path: &Path) -> io::Result<ReadOnlyGuard<Hidraw>> {
    let file = OpenOptions::new().read(true).write(true).open(path)?;
    Ok(ReadOnlyGuard::new(Hidraw { file }))
}

impl Transport for Hidraw {
    fn write_report(&mut self, report: &Report) -> io::Result<()> {
        // hidraw wants the report number first; QMK's raw-HID interface uses unnumbered reports (0).
        let mut buf = [0u8; REPORT_LEN + 1];
        buf[1..].copy_from_slice(report);
        let n = self.file.write(&buf)?;
        if n != buf.len() {
            return Err(io::Error::new(io::ErrorKind::WriteZero, format!("short hidraw write ({n} bytes)")));
        }
        Ok(())
    }

    fn read_report(&mut self, timeout: Duration) -> io::Result<Option<Report>> {
        let mut pfd = libc::pollfd { fd: self.file.as_raw_fd(), events: libc::POLLIN, revents: 0 };
        let ms = timeout.as_millis().min(i32::MAX as u128) as i32;
        // SAFETY: `pfd` is a valid pollfd for the duration of the call, and nfds is 1.
        let rc = unsafe { libc::poll(&mut pfd, 1, ms) };
        if rc < 0 {
            let err = io::Error::last_os_error();
            return if err.kind() == io::ErrorKind::Interrupted { Ok(None) } else { Err(err) };
        }
        if rc == 0 {
            return Ok(None);
        }
        if pfd.revents & libc::POLLIN == 0 {
            // POLLHUP / POLLERR / POLLNVAL without data: the device went away.
            return Err(io::Error::new(io::ErrorKind::BrokenPipe, "hidraw device gone"));
        }
        let mut report = [0u8; REPORT_LEN];
        let n = self.file.read(&mut report)?;
        if n == 0 {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "hidraw device gone"));
        }
        Ok(Some(report))
    }
}
