use std::io;
use std::time::Duration;

use crate::protocol::Report;

/// A raw-HID channel carrying fixed 32-byte reports.
/// Only `ReadOnlyGuard` should hold one; see `guard.rs`.
pub trait Transport: Send {
    fn write_report(&mut self, report: &Report) -> io::Result<()>;
    /// Waits up to `timeout` for one input report; `Ok(None)` on timeout.
    fn read_report(&mut self, timeout: Duration) -> io::Result<Option<Report>>;
}

impl<T: Transport + ?Sized> Transport for Box<T> {
    fn write_report(&mut self, report: &Report) -> io::Result<()> {
        (**self).write_report(report)
    }

    fn read_report(&mut self, timeout: Duration) -> io::Result<Option<Report>> {
        (**self).read_report(timeout)
    }
}
