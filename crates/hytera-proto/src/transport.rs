//! Byte transports: the real serial port and a replay of a recorded CPS session.

use std::time::Duration;

use crate::Result;

/// Something that moves bytes to and from the radio.
pub trait Transport {
    /// Send all of `data`.
    fn write_all(&mut self, data: &[u8]) -> Result<()>;

    /// Read whatever bytes are available, waiting at most `timeout`. Returns `Ok(0)` when nothing
    /// arrived in time.
    fn read(&mut self, buf: &mut [u8], timeout: Duration) -> Result<usize>;
}

impl<T: Transport + ?Sized> Transport for Box<T> {
    fn write_all(&mut self, data: &[u8]) -> Result<()> {
        (**self).write_all(data)
    }
    fn read(&mut self, buf: &mut [u8], timeout: Duration) -> Result<usize> {
        (**self).read(buf, timeout)
    }
}

#[cfg(feature = "serial")]
pub use serial::{SerialTransport, available_ports};

#[cfg(feature = "serial")]
mod serial {
    use std::io::{ErrorKind, Read, Write};
    use std::time::Duration;

    use serialport::{ClearBuffer, DataBits, FlowControl, Parity, SerialPort, StopBits};

    use super::Transport;
    use crate::Result;

    /// Line settings observed from CPS: 115200 baud, 8N1, no flow control.
    pub const BAUD_RATE: u32 = 115_200;

    /// The USB programming cable as a serial port.
    pub struct SerialTransport {
        port: Box<dyn SerialPort>,
        current_timeout: Duration,
    }

    impl SerialTransport {
        /// Open `name` (e.g. `COM3`, `/dev/ttyUSB0`) with CPS's settings, then clear RTS and DTR
        /// as CPS does.
        pub fn open(name: &str) -> Result<Self> {
            let timeout = Duration::from_millis(100);
            let mut port = serialport::new(name, BAUD_RATE)
                .data_bits(DataBits::Eight)
                .parity(Parity::None)
                .stop_bits(StopBits::One)
                .flow_control(FlowControl::None)
                .timeout(timeout)
                .open()?;
            port.write_request_to_send(false)?;
            port.write_data_terminal_ready(false)?;
            port.clear(ClearBuffer::All)?;
            Ok(Self { port, current_timeout: timeout })
        }
    }

    impl Transport for SerialTransport {
        fn write_all(&mut self, data: &[u8]) -> Result<()> {
            self.port.write_all(data)?;
            self.port.flush()?;
            Ok(())
        }

        fn read(&mut self, buf: &mut [u8], timeout: Duration) -> Result<usize> {
            let timeout = timeout.max(Duration::from_millis(1));
            if timeout != self.current_timeout {
                self.port.set_timeout(timeout)?;
                self.current_timeout = timeout;
            }
            match self.port.read(buf) {
                Ok(n) => Ok(n),
                Err(e) if e.kind() == ErrorKind::TimedOut => Ok(0),
                Err(e) => Err(e.into()),
            }
        }
    }

    /// Serial ports present on this machine, with a USB description when available.
    pub fn available_ports() -> Result<Vec<(String, String)>> {
        Ok(serialport::available_ports()?
            .into_iter()
            .map(|p| {
                let desc = match p.port_type {
                    serialport::SerialPortType::UsbPort(u) => format!(
                        "USB {:04x}:{:04x} {} {}",
                        u.vid,
                        u.pid,
                        u.manufacturer.unwrap_or_default(),
                        u.product.unwrap_or_default()
                    ),
                    serialport::SerialPortType::BluetoothPort => "Bluetooth".into(),
                    serialport::SerialPortType::PciPort => "PCI".into(),
                    serialport::SerialPortType::Unknown => String::new(),
                };
                (p.port_name, desc.trim().to_string())
            })
            .collect())
    }
}

#[cfg(feature = "replay")]
pub use replay::ReplayTransport;

#[cfg(feature = "replay")]
mod replay {
    use std::collections::VecDeque;
    use std::path::Path;
    use std::time::Duration;

    use super::Transport;
    use crate::{Error, Result};

    struct Step {
        tx: Vec<u8>,
        rx: Vec<u8>,
    }

    /// Plays back a recording made with `tools/serial_tap.js` (one JSON object per line).
    ///
    /// Every write must be byte-identical to the next byte block CPS wrote. After each matching
    /// write, the bytes the radio sent in reply become readable. A mismatch is an error, so a test
    /// that replays a full recording proves that we send exactly what CPS sent.
    pub struct ReplayTransport {
        steps: VecDeque<Step>,
        pending_tx: Vec<u8>,
        readable: VecDeque<u8>,
        writes_matched: usize,
    }

    impl ReplayTransport {
        pub fn from_jsonl_file(path: &Path) -> Result<Self> {
            Self::from_jsonl(&std::fs::read_to_string(path)?)
        }

        pub fn from_jsonl(text: &str) -> Result<Self> {
            let mut steps: VecDeque<Step> = VecDeque::new();
            let mut readable = VecDeque::new();
            for (line_no, line) in text.lines().enumerate() {
                if line.trim().is_empty() {
                    continue;
                }
                let v: serde_json::Value = serde_json::from_str(line)
                    .map_err(|e| Error::Replay(format!("line {}: {e}", line_no + 1)))?;
                let ev = v["ev"].as_str().unwrap_or_default();
                if ev != "tx" && ev != "rx" {
                    continue;
                }
                let bytes = decode_hex(v["hex"].as_str().unwrap_or_default())
                    .ok_or_else(|| Error::Replay(format!("line {}: bad hex", line_no + 1)))?;
                match (ev, steps.back_mut()) {
                    ("tx", _) => steps.push_back(Step { tx: bytes, rx: Vec::new() }),
                    (_, Some(step)) => step.rx.extend(bytes),
                    (_, None) => readable.extend(bytes),
                }
            }
            Ok(Self { steps, pending_tx: Vec::new(), readable, writes_matched: 0 })
        }

        /// True when every recorded write has been matched.
        pub fn finished(&self) -> bool {
            self.steps.is_empty() && self.pending_tx.is_empty()
        }

        pub fn writes_matched(&self) -> usize {
            self.writes_matched
        }
    }

    impl Transport for ReplayTransport {
        fn write_all(&mut self, data: &[u8]) -> Result<()> {
            self.pending_tx.extend_from_slice(data);
            while let Some(step) = self.steps.front() {
                let n = step.tx.len().min(self.pending_tx.len());
                if step.tx[..n] != self.pending_tx[..n] {
                    return Err(Error::Replay(format!(
                        "write #{} differs from the recording\n  expected {}\n  got      {}",
                        self.writes_matched + 1,
                        to_hex(&step.tx),
                        to_hex(&self.pending_tx)
                    )));
                }
                if self.pending_tx.len() < step.tx.len() {
                    return Ok(());
                }
                let step = self.steps.pop_front().expect("front exists");
                self.pending_tx.drain(..step.tx.len());
                self.readable.extend(step.rx);
                self.writes_matched += 1;
            }
            if !self.pending_tx.is_empty() {
                return Err(Error::Replay(format!(
                    "write past the end of the recording: {}",
                    to_hex(&self.pending_tx)
                )));
            }
            Ok(())
        }

        fn read(&mut self, buf: &mut [u8], _timeout: Duration) -> Result<usize> {
            let n = buf.len().min(self.readable.len());
            for (dst, src) in buf.iter_mut().zip(self.readable.drain(..n)) {
                *dst = src;
            }
            Ok(n)
        }
    }

    fn decode_hex(s: &str) -> Option<Vec<u8>> {
        if !s.len().is_multiple_of(2) {
            return None;
        }
        (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok()).collect()
    }

    fn to_hex(b: &[u8]) -> String {
        b.iter().map(|x| format!("{x:02x}")).collect()
    }
}
