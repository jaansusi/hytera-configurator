use std::time::Duration;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[cfg(feature = "serial")]
    #[error("serial port: {0}")]
    Serial(#[from] serialport::Error),

    #[error("I/O: {0}")]
    Io(#[from] std::io::Error),

    #[error("no answer from the radio within {timeout:?} while waiting for {waiting_for}")]
    Timeout { waiting_for: &'static str, timeout: Duration },

    #[error("malformed packet: {0}")]
    Malformed(String),

    #[error("unexpected response: {0}")]
    Unexpected(String),

    #[error("radio refused {what}: status 0x{status:04x}")]
    Status { what: String, status: u16 },

    #[error("replay mismatch: {0}")]
    Replay(String),
}

pub type Result<T> = std::result::Result<T, Error>;
