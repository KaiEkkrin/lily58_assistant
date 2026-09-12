//! Raw-HID access. Everything that reaches the keyboard goes through `guard::ReadOnlyGuard`.

pub mod discover;
#[cfg(test)]
pub mod fake;
pub mod guard;
pub mod hidraw;
pub mod transport;
