//! Serial programming protocol of Hytera G2 (Auctus platform) radios, as used by Hytera CPS.
//!
//! Layers, bottom to top (see `docs/protocol.md`):
//! - [`transport`]: moves bytes (serial port, or a replayed recording).
//! - [`link`]: 20-byte framed packets with an Internet checksum, sequence numbers and acknowledgements.
//! - [`app`]: `0x0A0B` application packets (opcode, sub-command, transaction ID, data).
//! - [`session`]: radio commands (info, programming mode, file reads) and CPS-compatible flows.

pub mod app;
pub mod checksum;
pub mod error;
pub mod link;
pub mod session;
pub mod transport;

pub use error::{Error, Result};
pub use session::{InfoItem, RadioInfo, Session};
