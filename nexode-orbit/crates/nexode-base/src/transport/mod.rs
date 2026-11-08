//! Local IPC transport.
//!
//! Re-exported from `nexode-transport`, which owns the implementation so the
//! harness API bridge can use it without pulling in this crate. Kept as a
//! re-export because `crate::transport::...` is used throughout the codebase.

pub use nexode_transport::*;
