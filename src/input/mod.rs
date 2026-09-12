//! Key events as the operating system reports them (no layer keys; the position is inferred).

pub mod evdev;
pub mod focused;

use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OsSource {
    Focused,
    Evdev,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OsKey {
    pub source: OsSource,
    /// HID usages that could have produced this key (several when ambiguous).
    pub usages: Vec<u8>,
    pub pressed: bool,
    /// Identifies the physical OS key, e.g. "evdev 30" or "A".
    pub name: String,
}

pub enum InputMsg {
    Key(OsKey),
    /// An evdev reader stopped (device unplugged).
    EvdevGone(PathBuf),
}
