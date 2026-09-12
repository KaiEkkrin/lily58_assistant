//! `/dev/hidrawN` transport.

use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::fd::AsRawFd;
use std::path::Path;
use std::time::{Duration, Instant};

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
        // hidraw takes a whole report per write(2) or none of it, so a short count can't be made
        // up with a second write: it is an error. An interrupted write sent nothing; retry it.
        let n = retry_interrupted(|| self.file.write(&buf))?;
        if n != buf.len() {
            return Err(io::Error::new(io::ErrorKind::WriteZero, format!("short hidraw write ({n} bytes)")));
        }
        Ok(())
    }

    fn read_report(&mut self, timeout: Duration) -> io::Result<Option<Report>> {
        let fd = self.file.as_raw_fd();
        let deadline = Instant::now() + timeout;
        let (rc, revents) = retry_interrupted(|| {
            let mut pfd = libc::pollfd { fd, events: libc::POLLIN, revents: 0 };
            // Round up, so a wait never ends before the deadline.
            let left = deadline.saturating_duration_since(Instant::now());
            let ms = left.as_micros().div_ceil(1000).min(i32::MAX as u128) as i32;
            // SAFETY: `pfd` is a valid pollfd for the duration of the call, and nfds is 1.
            let rc = unsafe { libc::poll(&mut pfd, 1, ms) };
            if rc < 0 { Err(io::Error::last_os_error()) } else { Ok((rc, pfd.revents)) }
        })?;
        if rc == 0 {
            return Ok(None);
        }
        if revents & libc::POLLIN == 0 {
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

/// Runs `op` until it finishes with something other than `EINTR`: a signal arriving mid-call
/// is not a failure.
fn retry_interrupted<T>(mut op: impl FnMut() -> io::Result<T>) -> io::Result<T> {
    loop {
        match op() {
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            result => return result,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::fd::OwnedFd;

    #[test]
    fn interrupted_calls_are_retried() {
        let mut calls = 0;
        let result = retry_interrupted(|| {
            calls += 1;
            if calls < 3 { Err(io::ErrorKind::Interrupted.into()) } else { Ok(calls) }
        });
        assert_eq!(result.unwrap(), 3);
    }

    #[test]
    fn other_errors_are_returned_at_once() {
        let mut calls = 0;
        let result: io::Result<()> = retry_interrupted(|| {
            calls += 1;
            Err(io::ErrorKind::BrokenPipe.into())
        });
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::BrokenPipe);
        assert_eq!(calls, 1);
    }

    #[test]
    fn read_report_waits_out_the_timeout_then_reads_a_report() {
        let (reader, mut writer) = io::pipe().unwrap();
        let mut hidraw = Hidraw { file: File::from(OwnedFd::from(reader)) };
        let start = Instant::now();
        assert_eq!(hidraw.read_report(Duration::from_millis(50)).unwrap(), None);
        assert!(start.elapsed() >= Duration::from_millis(45), "waited about the whole timeout");
        writer.write_all(&[7; REPORT_LEN]).unwrap();
        assert_eq!(hidraw.read_report(Duration::from_millis(50)).unwrap(), Some([7; REPORT_LEN]));
    }
}
